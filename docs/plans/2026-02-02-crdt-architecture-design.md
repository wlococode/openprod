# CRDT Architecture Design

**Date:** 2026-02-02
**Status:** Approved
**Author:** Claude (with Will)

---

## Overview

This document specifies CRDT (Conflict-free Replicated Data Type) support for Openprod, enabling collaborative editing of text fields and ordered collections without explicit conflict resolution.

### Goals

1. **Text CRDTs** for long-form fields (notes, descriptions) — concurrent edits auto-merge
2. **List CRDTs** for ordered primitive values (tags, options)
3. **Ordered edges** for ordered entity references (cue lists, playlists)
4. **Library-agnostic** — specify semantics, not implementation details

### Non-Goals

- Real-time cursor presence (future work)
- Rich text formatting (future work, requires Peritext or similar)
- Map CRDTs, counter CRDTs (future work)

---

## Architecture Decisions

### Decision 1: CRDTs are a field merge strategy

CRDT fields are regular fields with different merge semantics. The oplog doesn't need to understand CRDTs—it stores operations, and state derivation interprets them based on schema declarations.

### Decision 2: Ordered entity lists use edges, not CRDT fields

Lists of entity references (cue lists, playlists) use **ordered edges** rather than list CRDT fields because:

- Reverse lookups are indexed ("which lists contain cue X?")
- Per-item properties are supported (edge properties)
- Deletion cascades automatically (existing edge behavior)
- Fits the entity-field-edge model

List CRDT fields are reserved for primitive values (strings, numbers).

### Decision 3: Abstract semantics, not concrete library

The spec defines operation semantics and merge behavior without mandating a specific CRDT library. Recommended implementations (Yrs, Diamond-types) are noted but not required.

---

## Data Model

### CRDT Field Declaration

Plugin manifests declare CRDT-enabled fields:

```yaml
# In plugin manifest
fields:
  description:
    type: string
    shared_key: description
    crdt: text              # Text CRDT enabled

  notes:
    type: string
    crdt:
      type: text
      granularity: character  # Optional: character or paragraph

  tags:
    type: list
    crdt: list              # List CRDT for primitive arrays
    item_type: string
```

### Ordered Edge Declaration

Edge types declare ordering support:

```yaml
# In plugin manifest
edge_types:
  in_cue_list:
    source_tables: [cues]
    target_tables: [cue_lists]
    ordered: true           # Enables CRDT-based ordering
    properties:
      call_text: string
      timing_override: number
```

---

## Operations

### ApplyCRDT

Delta update to a CRDT field.

```yaml
ApplyCRDT:
  op_id: <uuid>
  entity_id: <uuid>
  field: "description"
  crdt_type: "text"           # Matches schema declaration
  delta: <opaque_bytes>       # CRDT-specific delta format
  actor_id: <actor>
  hlc: <timestamp>
  signature: <signature>
```

**Behavior:**
- If field has no CRDT state, initialize empty state first
- Apply delta using CRDT merge algorithm
- Update `fields.value` with new state and rendered value

**Validation:**
- Field must be declared as CRDT in schema
- `crdt_type` must match schema declaration
- Delta must be valid for the CRDT type

### SetField (unchanged, extended behavior)

Full replacement still works for CRDT fields:

```yaml
SetField:
  entity_id: <uuid>
  field: "description"
  value: <crdt_state_or_plain_value>
```

For CRDT fields, `SetField` replaces the entire CRDT state. Used for:
- Initialization (first write)
- Snapshots (full state sync)
- Resets (clear and start over)

### CreateOrderedEdge

Insert an edge with position in an ordered edge type.

```yaml
CreateOrderedEdge:
  op_id: <uuid>
  edge_id: <uuid>
  edge_type: "in_cue_list"
  source: <entity_id>         # The item (e.g., cue)
  target: <entity_id>         # The container (e.g., cue list)
  after: <edge_id | null>     # Insert after this edge (null = start)
  before: <edge_id | null>    # Insert before this edge (null = end)
  properties:
    call_text: "GO"
  actor_id: <actor>
  hlc: <timestamp>
  signature: <signature>
```

**Behavior:**
- Generate `_position` from `after`/`before` references
- Create edge with generated position
- Position enables deterministic ordering

**Position Generation:**
- Positions are variable-length byte strings, lexicographically sortable
- Insert between A and B generates position that sorts between them
- Concurrent inserts at same position use `(position, actor_id, hlc)` tiebreaker

### MoveOrderedEdge

Reorder an existing edge.

```yaml
MoveOrderedEdge:
  op_id: <uuid>
  edge_id: <uuid>
  after: <edge_id | null>
  before: <edge_id | null>
  actor_id: <actor>
  hlc: <timestamp>
  signature: <signature>
```

**Behavior:**
- Generate new `_position` from `after`/`before`
- Update edge's position property
- Concurrent moves: LWW by HLC

---

## State Derivation

### CRDT Field Derivation

```
for each op affecting (entity_id, field) in reception order:
    if op is SetField:
        state = op.value  # Full replacement
    elif op is ApplyCRDT:
        if state is None:
            state = crdt_init(field.crdt_type)
        state = crdt_merge(state, op.delta)

fields[entity_id][field] = {
    _crdt: true,
    type: field.crdt_type,
    state: state,
    rendered: crdt_render(state)
}
```

**Key property:** CRDT operations can be applied in any order and converge. Reception order is used for simplicity.

### Ordered Edge Derivation

```
for each op affecting edges of (target, edge_type) in canonical order:
    if op is CreateOrderedEdge:
        position = generate_position(op.after, op.before, existing_edges)
        edges[op.edge_id] = { ...op.properties, _position: position }
    elif op is MoveOrderedEdge:
        position = generate_position(op.after, op.before, existing_edges)
        edges[op.edge_id]._position = position
    elif op is DeleteEdge:
        delete edges[op.edge_id]

return sorted(edges, by: _position)
```

---

## Conflict Model

### Text CRDT Conflicts

| Scenario | Behavior |
|----------|----------|
| Concurrent insertions | Auto-merged, deterministic order |
| Concurrent deletions | Auto-merged |
| Insert + delete same range | Insert preserved |

Text CRDTs do not produce traditional field-level conflicts. All concurrent edits merge automatically.

### Ordered Edge Conflicts

| Scenario | Behavior |
|----------|----------|
| Concurrent inserts at same position | Both appear, tiebreak by (actor_id, hlc) |
| Concurrent moves of same edge | LWW by HLC |
| Move + delete | Delete wins |

### What Doesn't Conflict

CRDT fields bypass the normal conflict detection system for content changes. The existing conflict system remains for:

- Plain fields (LWW with explicit conflict resolution)
- Non-position edge properties (LWW)
- Structural conflicts (rare, implementation-dependent)

---

## Storage

### CRDT Field Storage

No schema changes. The `fields.value` column stores:

```yaml
# MessagePack encoded
{
  "_crdt": true,
  "type": "text",
  "state": <crdt_state_bytes>,
  "rendered": "The human-readable text content"
}
```

The `rendered` field caches the human-readable value for queries.

### Ordered Edge Storage

The `edges.properties` column includes `_position`:

```yaml
# MessagePack encoded
{
  "call_text": "GO",
  "_position": "Pm3xK"  # Lexicographically sortable
}
```

### New Index

```sql
CREATE INDEX idx_edges_ordered ON edges (
    target_id,
    edge_type
) WHERE deleted_at IS NULL;
```

Queries sort by `_position` extracted from properties.

---

## Script API

Scripts edit CRDT fields through a high-level API:

### Text CRDT API

```javascript
// Insert text at position
entity.description.insertAt(position, "text");

// Delete range
entity.description.deleteRange(start, end);

// Replace range
entity.description.replaceRange(start, end, "replacement");

// Get current text
const text = entity.description.toString();
```

### List CRDT API (for primitive lists)

```javascript
// Insert at position
entity.tags.insert(2, "urgent");

// Append
entity.tags.append("review");

// Remove by value
entity.tags.remove("done");

// Remove by index
entity.tags.removeAt(0);
```

### Ordered Edge API

```javascript
// Get ordered edges
const cues = entity.edges("in_cue_list", { ordered: true });

// Insert edge
entity.createEdge("in_cue_list", cue, {
    after: prevCue,
    properties: { call_text: "GO" }
});

// Move edge
entity.moveEdge(edgeId, { after: newPrevCue });
```

The script runtime translates these to `ApplyCRDT`, `CreateOrderedEdge`, and `MoveOrderedEdge` operations.

---

## Query Integration

### CRDT Fields in Queries

Queries see the rendered value:

```
entity.description  →  "The text content"  (string)
entity.tags         →  ["urgent", "review"]  (array)
```

The CRDT state is an implementation detail hidden from queries.

### Ordered Edges in Queries

```
// Get ordered edges to a target
edges_to(cue_list, "in_cue_list", ordered: true)
  → [{ source: cue_42, call_text: "GO" }, { source: cue_43, ... }]

// Expand to full entities
edges_to(cue_list, "in_cue_list", ordered: true).sources.expand()
  → [{ id: cue_42, cue_number: 42, ... }, ...]
```

Order is preserved unless explicitly re-sorted.

---

## Implementation Phases

### Phase 1: Foundation
- Add `ApplyCRDT` operation type to `operations.md`
- Add CRDT field declarations to `data-model.md`
- Create `crdt.md` specification
- Implement text CRDT state derivation

### Phase 2: Ordered Edges
- Add `CreateOrderedEdge`, `MoveOrderedEdge` operations
- Add ordered edge declarations to `data-model.md`
- Create `ordered-edges.md` specification
- Add position index to `sqlite-schema.md`

### Phase 3: Script Integration
- Add CRDT editing API to `scripts.md`
- Implement script runtime translation

### Phase 4: Refinements
- Conflict model updates in `conflicts.md`
- Delta sync optimization (future)
- Tombstone compaction (future)

---

## Recommended CRDT Libraries

| Library | Language | Notes |
|---------|----------|-------|
| **Yrs** | Rust | Port of Yjs, battle-tested, good ecosystem |
| **Diamond-types** | Rust | Optimized for offline-first |
| **Automerge** | Rust/JS | Full CRDT toolkit |

Start with Yrs for text. Ordered edge positions can use a simple fractional indexing scheme without external dependencies.

---

## Open Questions (Deferred)

- Rich text formatting (Peritext integration)
- Real-time cursor/selection presence
- CRDT garbage collection / tombstone compaction
- Delta sync optimization for large documents
- Map and counter CRDT types
