# Ordered Edges Specification

This document defines ordered edges, which enable ordered lists of entity references with CRDT-based positioning.

---

## Overview

Ordered edges extend the edge model to support deterministic ordering. They are used for ordered collections of entities, such as cue lists, playlists, and task lists.

**Anchor invariant:** Ordered edges converge to identical order on all peers given the same operations, regardless of operation order. Concurrent insertions at the same position appear in deterministic order.

### Why Ordered Edges (Not List CRDT Fields)

For ordered lists of entity references, ordered edges are preferred over list CRDT fields:

| Concern | List CRDT Field | Ordered Edges |
|---------|-----------------|---------------|
| Reverse lookup | Scan all containers | Indexed edge query |
| Per-item properties | Not supported | Edge properties |
| Deletion cascade | Manual cleanup | Automatic |
| Query integration | Requires expansion | Native edge queries |

Use list CRDT fields for ordered primitives (strings, numbers). Use ordered edges for ordered entity references.

---

## Edge Type Declaration

Modules declare ordered edge types in their schema:

```yaml
edge_types:
  in_cue_list:
    source_tables: [lighting_cues]
    target_tables: [cue_lists]
    ordered: true               # Enables CRDT-based ordering
    properties:
      call_text: string
      timing_override: number

  in_playlist:
    source_tables: [media_items]
    target_tables: [playlists]
    ordered: true
    properties:
      play_count: number
      last_played: timestamp
```

### Declaration Rules

- `ordered: true` enables position tracking
- Source and target table constraints apply as normal
- Properties are per-edge (per-item in the list)
- Ordering is within (target, edge_type) scope

---

## Position Identifiers

### Format

Position identifiers are variable-length byte strings that sort lexicographically:

```
Position: <bytes>
Comparison: memcmp (lexicographic byte comparison)
```

### Generation Algorithm

Positions are generated to sort between existing positions:

```
function generate_position(after_edge, before_edge):
    after_pos = after_edge?._position ?? MIN_POSITION
    before_pos = before_edge?._position ?? MAX_POSITION

    return midpoint(after_pos, before_pos)
```

The `midpoint` function generates a position that sorts between `after_pos` and `before_pos`.

### Fractional Indexing

A simple implementation uses base-62 fractional indexing:

```
Initial list:       []
Insert first:       [A]     positions: ["P"]
Insert at end:      [A, B]  positions: ["P", "V"]
Insert between:     [A, C, B]  positions: ["P", "S", "V"]
Insert at start:    [D, A, C, B]  positions: ["H", "P", "S", "V"]
```

More sophisticated schemes (Logoot, LSEQ) provide better distribution for heavy concurrent editing.

### Concurrent Insert Tiebreaking

When two peers insert at the same position concurrently:

1. Both generate the same position value
2. Final order is determined by `(position, actor_id, hlc)`
3. Lower actor_id wins (deterministic)

This ensures all peers converge to identical order.

---

## Operations

### CreateOrderedEdge

Insert an edge with position.

```yaml
CreateOrderedEdge:
  op_id: <uuid>
  edge_id: <uuid>
  edge_type: "in_cue_list"
  source: <entity_id>           # The item (e.g., cue)
  target: <entity_id>           # The container (e.g., cue list)
  after: <edge_id | null>       # Insert after this edge (null = start)
  before: <edge_id | null>      # Insert before this edge (null = end)
  properties:
    call_text: "GO"
  actor_id: <actor>
  hlc: <timestamp>
```

**Behavior:**

1. Validate edge type is declared as `ordered: true`
2. Resolve `after` and `before` to position values
3. Generate new position between them
4. Create edge with `_position` property
5. Validate source/target table constraints

**Position resolution:**

- `after: null` -> use minimum position (insert at start)
- `before: null` -> use maximum position (insert at end)
- Both null -> insert as only item
- Both specified -> insert between them

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
```

**Behavior:**

1. Validate edge exists and is ordered type
2. Generate new position from `after`/`before`
3. Update edge's `_position` property

**Concurrent moves:** If two peers move the same edge concurrently, LWW by HLC applies--the later move wins.

### DeleteEdge (Existing)

Deleting an ordered edge uses the existing `DeleteEdge` operation. The edge is removed from the ordered list.

---

## State Derivation

### Algorithm

```
function derive_ordered_edges(target_id, edge_type):
    edges = {}

    for each op in canonical_order:
        if op.type == "CreateOrderedEdge":
            if op.target == target_id and op.edge_type == edge_type:
                pos = generate_position(op.after, op.before, edges)
                edges[op.edge_id] = {
                    source: op.source,
                    properties: op.properties,
                    _position: pos,
                    _actor: op.actor_id,
                    _hlc: op.hlc
                }

        elif op.type == "MoveOrderedEdge":
            if op.edge_id in edges:
                pos = generate_position(op.after, op.before, edges)
                edges[op.edge_id]._position = pos
                edges[op.edge_id]._hlc = op.hlc

        elif op.type == "DeleteEdge":
            if op.edge_id in edges:
                delete edges[op.edge_id]

    # Sort by (position, actor_id, hlc) for deterministic order
    return sorted(edges.values(), key=lambda e: (e._position, e._actor, e._hlc))
```

### Position Regeneration

Positions are regenerated during derivation because:

- The `after`/`before` references may point to edges that were later deleted
- Concurrent operations may have created edges at the referenced positions
- Regeneration ensures consistency

The same `after`/`before` references always produce the same position given the same edge state, ensuring determinism.

---

## Conflict Behavior

### Concurrent Insertions

Concurrent insertions at the same position both succeed:

```
User A: CreateOrderedEdge { after: cue_1, before: cue_2 }  -> cue_A
User B: CreateOrderedEdge { after: cue_1, before: cue_2 }  -> cue_B

Result: [cue_1, cue_A, cue_B, cue_2]  (or [cue_1, cue_B, cue_A, cue_2])
Order determined by (position, actor_id, hlc) tiebreaker
```

No conflict surfaced to user--both insertions appear.

### Concurrent Moves

Concurrent moves of the same edge use LWW:

```
User A (HLC 100): MoveOrderedEdge { edge: cue_X, after: cue_1 }
User B (HLC 101): MoveOrderedEdge { edge: cue_X, after: cue_5 }

Result: cue_X appears after cue_5 (User B's move wins by HLC)
```

No conflict surfaced--later move wins.

### Move + Delete

If one peer moves an edge while another deletes it:

```
User A: MoveOrderedEdge { edge: cue_X, after: cue_1 }
User B: DeleteEdge { edge: cue_X }

Result: Edge is deleted (delete wins)
```

### Edge Property Conflicts

Non-position edge properties (e.g., `call_text`) follow normal LWW conflict rules. If two peers edit the same property, it may produce a conflict.

---

## Storage

### Edge Table

Ordered edges are stored in the standard `edges` table. The `_position` value is stored as a row in the `edge_properties` table with `property_key = "_position"`, following the same per-property storage pattern as all other edge properties:

```yaml
# edge_properties rows for an ordered edge:
(edge_id, "call_text", "GO")
(edge_id, "timing_override", 2.5)
(edge_id, "_position", "Pm3xK")    # Lexicographically sortable position identifier
```

See [sqlite-schema.md](sqlite-schema.md) for the `edge_properties` table DDL.

### Index

```sql
CREATE INDEX idx_edges_target ON edges (target_id, edge_type) WHERE deleted_at IS NULL;
```

Queries retrieve edges by `(target_id, edge_type)` and sort by `_position` in application code. The `_position` value is read from `edge_properties` and sorted in memory.

---

## Queries

### Basic Query

Get ordered edges to a target:

```
edges_to(cue_list, "in_cue_list")
  -> [
      { edge_id: e1, source: cue_42, call_text: "GO" },
      { edge_id: e2, source: cue_43, call_text: "STANDBY" },
      ...
    ]
```

Results are returned in position order.

### Expand Sources

Get full source entities:

```
edges_to(cue_list, "in_cue_list").expand_sources()
  -> [
      { id: cue_42, cue_number: 42, intensity: 80, call_text: "GO" },
      { id: cue_43, cue_number: 43, intensity: 65, call_text: "STANDBY" },
      ...
    ]
```

Edge properties are merged into the expanded entity.

### Reverse Lookup

Find which containers include an entity:

```
edges_from(cue_42, "in_cue_list")
  -> [
      { edge_id: e1, target: act_1_cues, call_text: "GO" },
      { edge_id: e5, target: act_2_cues, call_text: "WARN" },
    ]
```

This uses the standard edge index--no scanning required.

### Filter and Sort

```
# Filter by edge property
edges_to(cue_list, "in_cue_list")
    .filter(e => e.call_text == "GO")

# Re-sort (overrides position order)
edges_to(cue_list, "in_cue_list")
    .expand_sources()
    .sort_by(cue => cue.cue_number)
```

---

## Use Cases

### Cue List

```yaml
Entity: act_1_cues
  table: cue_lists
  fields:
    name: "Act 1 Lighting Cues"

Edge: cue_42 --[in_cue_list]--> act_1_cues
  properties:
    call_text: "GO"
    _position: "P"

Edge: cue_43 --[in_cue_list]--> act_1_cues
  properties:
    call_text: "STANDBY"
    _position: "V"
```

### Playlist

```yaml
Entity: road_trip_playlist
  table: playlists
  fields:
    name: "Road Trip Mix"

Edge: song_1 --[in_playlist]--> road_trip_playlist
  properties:
    added_by: "alice"
    _position: "M"
```

### Task List

```yaml
Entity: sprint_backlog
  table: task_lists
  fields:
    name: "Sprint 42 Backlog"

Edge: task_101 --[in_task_list]--> sprint_backlog
  properties:
    assignee: "bob"
    _position: "K"
```

---

## Implementation Notes

### Position Alphabet

A simple base-62 alphabet works well:

```
0-9, A-Z, a-z
```

This provides 62 choices per character, allowing fine-grained positioning.

### Position Length

Positions grow longer with repeated insertions at the same spot:

```
Insert at start repeatedly: "P", "H", "D", "B", "A5", "A2", "A1", ...
```

For most use cases, positions stay short (1-4 characters). Heavy editing at one position may grow longer.

### Rebalancing (Optional)

If positions grow too long, a rebalancing operation can reassign positions:

```yaml
RebalanceOrderedEdges:
  target: <entity_id>
  edge_type: "in_cue_list"
  new_positions:
    edge_1: "P"
    edge_2: "V"
    edge_3: "b"
```

Rebalancing is cosmetic--it doesn't change order, only position values.

---

## Open Questions

- Maximum position length before forced rebalance
- Position encoding (base-62 vs binary vs other)
- Batch insert optimization (insert multiple at once)
- Position compression for storage

---

## Related Documents

- [data-model.md](data-model.md) -- Edge model
- [operations.md](operations.md) -- CreateOrderedEdge, MoveOrderedEdge operations
- [crdt.md](crdt.md) -- CRDT fields for primitive lists
- [scripts.md](scripts.md) -- Script API for ordered edges
