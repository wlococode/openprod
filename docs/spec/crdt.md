# CRDT Specification

This document defines CRDT (Conflict-free Replicated Data Type) fields, which enable automatic merging of concurrent edits without explicit conflict resolution.

---

## Overview

CRDT fields are regular fields with special merge semantics. Instead of Last-Writer-Wins, concurrent edits are merged automatically by a CRDT algorithm.

**Anchor invariant:** CRDT fields converge to identical state on all peers given the same set of operations, regardless of operation order.

### Supported CRDT Types

| Type | Use Case | Value |
|------|----------|-------|
| `text` | Long-form text (notes, descriptions) | String |
| `list` | Ordered primitives (tags, options) | Array |

Future types (map, counter) may be added.

---

## Field Declaration

Modules declare CRDT-enabled fields in their manifest:

```yaml
fields:
  # Simple declaration
  description:
    type: string
    shared_key: description
    crdt: text

  # With options
  notes:
    type: string
    crdt:
      type: text
      granularity: character    # character (default) or paragraph

  # List of primitives
  tags:
    type: list
    crdt: list
    item_type: string           # string, number, boolean
```

### Declaration Rules

- `crdt` attribute enables CRDT semantics for the field
- `type` must be compatible (`string` for text, `list` for list)
- `item_type` required for list CRDTs (primitives only, not entity refs)
- CRDT declaration is immutable after module adoption (changing requires migration)

### Standard CRDT Fields

Recommended shared keys for common CRDT fields:

| Shared Key | CRDT Type | Description |
|------------|-----------|-------------|
| `description` | text | Long-form description |
| `notes` | text | General notes |
| `body` | text | Main content body |
| `tags` | list | Tag labels |

---

## Text CRDT

Text CRDTs enable collaborative editing of string fields. Concurrent insertions and deletions merge automatically.

### Semantics

**Insertion:** Characters inserted at a position appear at that position. Concurrent insertions at the same position appear in deterministic order (by actor ID).

**Deletion:** Deleted characters are removed. Concurrent deletions of the same character are idempotent.

**Merge behavior:**

| Scenario | Result |
|----------|--------|
| User A inserts "X" at position 5, User B inserts "Y" at position 5 | Both appear: "XY" or "YX" (deterministic by actor) |
| User A inserts "X" at position 5, User B deletes position 5-10 | "X" appears before the remaining text |
| User A deletes position 5-10, User B deletes position 8-15 | Union of deletions (positions 5-15 deleted) |

### Granularity Options

| Granularity | Behavior | Use Case |
|-------------|----------|----------|
| `character` (default) | Character-by-character merge | Real-time collaborative editing |
| `paragraph` | Block-level merge | Structured documents |

Character granularity provides finest-grained merging but larger CRDT state. Paragraph granularity reduces state size but may produce conflicts within paragraphs.

### Operations

Text CRDT fields are updated via:

1. **`ApplyCRDT`** — Delta update (character insert, delete)

`SetField` is **rejected** on CRDT fields (type error at validation time). See [operations.md](operations.md) for operation formats.

---

## List CRDT

List CRDTs enable collaborative editing of ordered arrays of primitive values.

### Semantics

**Insertion:** Items inserted at a position appear at that position. Concurrent insertions at the same position appear in deterministic order.

**Deletion:** Deleted items are removed. Concurrent deletions are merged.

**Reordering:** Not directly supported. To reorder, delete and re-insert.

### Supported Item Types

List CRDTs store primitive values only:

| Item Type | Example |
|-----------|---------|
| `string` | `["urgent", "review", "approved"]` |
| `number` | `[1, 2, 3, 5, 8, 13]` |
| `boolean` | `[true, false, true]` |

For ordered lists of entity references, use [ordered edges](ordered-edges.md) instead.

### Operations

List CRDT fields are updated via:

1. **`ApplyCRDT`** — Delta update (insert, delete)

`SetField` is **rejected** on CRDT fields (type error at validation time).

---

## State Format

### Storage

CRDT field values are stored in the `fields.value` column as MessagePack:

```yaml
{
  "_crdt": true,
  "type": "text",               # or "list"
  "state": <crdt_state_bytes>,  # Opaque CRDT state
  "rendered": "..."             # Human-readable value (cached)
}
```

- `_crdt: true` — Marker identifying CRDT storage
- `type` — CRDT type for interpretation
- `state` — Opaque binary state (implementation-dependent)
- `rendered` — Cached rendered value for queries

### Rendered Value

The `rendered` field caches the human-readable form:

| CRDT Type | Rendered Value |
|-----------|----------------|
| `text` | The text string |
| `list` | JSON array of items |

Queries access the rendered value, not the raw CRDT state.

### State Size

CRDT state includes tombstones (markers for deleted content). State grows with edit history, not just current content.

**Compaction:** Implementations may compact tombstones after all peers have observed deletions. Compaction is an optimization, not required for correctness.

---

## State Derivation

### Algorithm

```
function derive_crdt_field(entity_id, field_key):
    state = null

    for each op affecting (entity_id, field_key) in reception order:
        if op.type == "ApplyCRDT":
            if state == null:
                state = crdt_init(schema.crdt_type)
            state = crdt_merge(state, op.delta)
        # SetField is rejected at validation time for CRDT fields

    return {
        _crdt: true,
        type: schema.crdt_type,
        state: state,
        rendered: crdt_render(state)
    }
```

### Order Independence

**Anchor invariant:** CRDT merge is commutative and associative. Operations can be applied in any order and produce identical final state.

Reception order is used for simplicity—no canonical sorting required for CRDT state computation.

### SetField Behavior

`SetField` on a CRDT field is a **type error** and is **rejected at validation time**. CRDT fields must be modified exclusively through `ApplyCRDT` operations. This prevents silent loss of concurrent CRDT edits that would occur if a full replacement were allowed.

For CRDT set fields, use `ClearAndAdd` to reset to specific values while preserving merge semantics. See [operations.md](operations.md) for details.

---

## Conflict Behavior

### No Content Conflicts

CRDT fields do not produce field-level conflicts for content changes. All concurrent edits merge automatically.

**Anchor invariant:** CRDT fields bypass the conflict detection system. There is no "conflict" state for CRDT content—only merged state.

### What Can Still Conflict

| Scenario | Behavior |
|----------|----------|
| Non-CRDT metadata on same entity | Normal conflict rules apply |
| Permissions/access changes | Normal conflict rules apply |

### Structural Conflicts (Rare)

Some CRDT implementations may detect structural conflicts (e.g., concurrent block-type changes in rich text). If detected:

1. CRDT algorithm chooses a deterministic resolution
2. Conflict may be logged for awareness
3. No user intervention required

Structural conflicts are implementation-dependent and rare in practice.

---

## Sync Behavior

### Full State Sync

During catch-up sync, CRDT fields sync their full state:

```yaml
FieldSync:
  entity_id: <uuid>
  field: "description"
  value:
    _crdt: true
    type: "text"
    state: <full_crdt_state>
    rendered: "..."
```

The receiving peer merges the incoming state with local state.

### Delta Sync

For incremental sync, peers exchange Yjs state vectors to compute minimal deltas. See [sync.md](sync.md#crdt-field-sync) for the full protocol.

```yaml
CRDTSyncRequest:
  entity_id: <uuid>
  field: "description"
  state_vector: <yjs_sv_bytes>    # Peer's current state vector

CRDTSyncResponse:
  entity_id: <uuid>
  field: "description"
  delta: <yjs_update_bytes>       # Only changes peer is missing
  state_vector: <yjs_sv_bytes>    # For reverse sync
```

Delta size is proportional to changes since last sync, not document size.

---

## Implementation Requirements

### Required Library: Yrs (Yjs)

**Anchor invariant:** All CRDT fields use Yrs (the Rust port of Yjs) for state encoding and merge operations. This ensures all peers can merge state regardless of platform.

| CRDT Type | Yjs Type | Notes |
|-----------|----------|-------|
| `text` | `Y.Text` | Character-level collaborative text |
| `list` | `Y.Array` | Ordered array of primitives |

**Why Yjs:**
- Battle-tested in production (Notion, Figma, etc.)
- Excellent Rust port (Yrs) with Swift and other bindings
- Handles text and arrays in a unified model
- Well-documented binary format enables cross-platform interop

### State Encoding

**Anchor invariant:** CRDT state is encoded using the Yjs binary format (Yrs `encode_state_as_update`). All peers must use this format.

```yaml
# ApplyCRDT delta format
delta: <yjs_update_bytes>    # Output of encode_state_as_update_v2()

# Stored state format
state: <yjs_state_bytes>     # Output of encode_state_as_update_v2()
```

**Encoding requirements:**
- Use Yjs v2 encoding (`encode_state_as_update_v2`) for efficiency
- State vectors use Yjs `StateVector` format
- Deltas are Yjs updates that can be applied via `apply_update_v2`

### Version Tracking

Yjs maintains internal state vectors for:

- **Client ID:** Unique identifier per editing session
- **Clock:** Monotonic counter per client
- **State vector:** Map of `{client_id: clock}` for delta sync

These are internal to Yjs, separate from HLC. The HLC provides cross-field causal ordering; Yjs clocks provide within-field operation ordering.

### Compatibility Guarantees

- Peers using different Yrs versions must produce compatible state
- Minimum supported Yrs version: 0.17.0 (or current stable at implementation time)
- State produced by any supported version can be merged by any other supported version
- Breaking Yrs version upgrades require workspace migration

---

## Queries

### Accessing CRDT Fields

Queries see the rendered value:

```
entity.description    →  "The text content"
entity.tags           →  ["urgent", "review"]
```

### Full-Text Search

Text CRDT fields support full-text search on the rendered value. The CRDT state is not searchable.

### Filtering Lists

List CRDT fields support array operations:

```
entity.tags.contains("urgent")    →  true
entity.tags.length                →  3
entity.tags[0]                    →  "urgent"
```

---

## Open Questions

- Rich text formatting (Yjs supports it; need to define attribute schema)
- Tombstone compaction policy (Yjs `gc` option; when to trigger?)
- Maximum CRDT state size advisory (warn at 1MB? 10MB?)
- Yrs version upgrade policy

### Future: Suggestions Mode

A Google Docs-style "Suggestions" mode for CRDT fields:

- Edits by certain users marked as suggestions (not immediately canonical)
- Suggestions visible inline with distinct styling
- Accept/reject suggestions individually or in batch
- Uses Yjs formatting marks to track suggestion metadata
- Preserves real-time collaboration while enabling review workflow

This would enable collaborative editing with review for sensitive content without losing CRDT benefits. See [approval-workflows.md](approval-workflows.md) for approval policy interaction *(archived / deferred to post-v1)*.

---

## Related Documents

- [operations.md](operations.md) — ApplyCRDT operation format
- [ordered-edges.md](ordered-edges.md) — Ordered entity lists (use edges, not list CRDT)
- [conflicts.md](conflicts.md) — Conflict model (CRDT fields exempt)
- [scripts.md](scripts.md) — Script API for CRDT editing
- [sync.md](sync.md) — Sync protocol
