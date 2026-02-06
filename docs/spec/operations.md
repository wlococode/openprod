# Operations Specification

This document defines the operation model, oplog structure, bundles, and state derivation.

---

## Operations: Mutation Model

- Operations are immutable once committed
- Operations must be schema-versioned
- Replay of operation sequence is deterministic
- Idempotency: Duplicate operations must always converge to identical state
- Operations attribute an **actor ID** and **timestamp** (HLC)
- State is never mutated without an explicit operation

### Granularity & Bundles

- Operations are stored at field granularity
- Operations are grouped into **bundles** for atomicity
- A bundle either fully commits or fully fails
- Undo/redo operates on bundles, not individual ops
- Scripts produce exactly one bundle per execution

### Bundle Atomicity

**Anchor invariant:** Bundles are atomic. All operations in a bundle either commit together or fail together. No partial bundles ever persist.

**Crash recovery (WAL):**
- Write-ahead log tracks bundle boundaries
- On crash recovery, scan WAL for incomplete bundles
- Incomplete bundles are rolled back (never partially committed)

**Network sync:**
- Bundles are transmitted with framing (bundle_id, op_count, checksum)
- Receiver buffers ops until complete bundle received
- Incomplete bundles discarded, sender retransmits

**Permission check:**
- Permission state is computed at bundle's HLC (deterministic)
- All peers derive same permission state at that HLC
- Bundle either succeeds everywhere or fails everywhere

```yaml
BundleFrame:
  bundle_id: <uuid>
  op_count: 5
  ops: [op1, op2, op3, op4, op5]
  checksum: blake3(ops)
```

### Bulk Operations

Bulk operations are collections of changes applied atomically as a single bundle.

- Bulk operations are represented as bundles
- Bulk operations must be previewable before commit
- Bulk operations may be staged in overlays for review
- Bulk operations must be atomic when committed
- Bulk operation preview shows all affected entities and fields

### Undo/Redo

**Anchor invariant:** Undo/redo is per-user, operates on bundles, and gracefully handles conflicts.

- Each user has their own undo stack tracking only their operations
- Undo stack does not persist across app restarts
- Undo stack depth limited to 50-100 operations
- Undo creates inverse operations for the last bundle in the user's stack
- **Conflict detection:** If another user edited the same data after the operation being undone, undo is skipped
  - User sees notification: "Cannot undo: [entity/field] was modified by [other user]"
  - Undo stack advances to next operation (skip + advance)
- Redo re-applies operations that were undone
- Undo operates on bundles, not individual operations
- UI context determines bundle granularity (immediate edits = fine-grained, form submission = coarse-grained)

---

## Operation Types

**Anchor invariant:** Entity creation is an explicit operation, not implicit. All mutations are symmetric operation types.

### Core Operation Types

| Operation | Description |
|-----------|-------------|
| `CreateEntity` | Create a new entity |
| `DeleteEntity` | Delete an entity (cascades to edges) |
| `SetField` | Set a field value on an entity (non-CRDT fields only) |
| `ClearField` | Clear a field value on an entity |
| `ApplyCRDT` | Apply a delta to a CRDT field |
| `CreateEdge` | Create a relationship between entities |
| `DeleteEdge` | Remove an edge |
| `CreateOrderedEdge` | Create an edge with position in an ordered list |
| `MoveOrderedEdge` | Reorder an existing ordered edge |
| `RebalanceOrderedEdges` | Recompute positions for an ordered edge set |

### CreateEntity Operation

```yaml
CreateEntity:
  entity_id: <uuidv7>
  initial_table: "contacts"    # Optional: table to add the entity to
  actor: <actor_id>
  hlc: <timestamp>
```

- Entity creation is explicit (not implicit from first field write)
- Writing to a non-existent entity ID is an error
- Provides clear audit trail for entity lifecycle
- If `initial_table` is provided, the corresponding facet is attached atomically

### DeleteEntity Operation

```yaml
DeleteEntity:
  entity_id: <uuid>
  cascade_edges: [<edge_id>, ...]  # Edges cascade-deleted atomically
  actor: <actor_id>
  hlc: <timestamp>
```

- `cascade_edges` is **computed by the system** at deletion time (not provided by the user)
- The system finds all edges where the entity is source or target
- Storing computed edges in the operation provides full audit trail of what was deleted
- This hybrid approach ensures no manual edge enumeration while maintaining auditability

### LinkTables Operation

```yaml
LinkTables:
  source_table: "contacts"
  target_table: "attendees"
  field_mappings:
    - source_field: "contacts.name"
      target_field: "attendees.name"
      confirmed: true
    - source_field: "contacts.email"
      target_field: "attendees.email"
      confirmed: true
  actor: <actor_id>
  hlc: <timestamp>
```

- Establishes a link between two tables with confirmed field mappings
- Existing entities in the source table may be added to the target table (subject to rules or user action)
- Table-level linking is a convenience shortcut; per-entity membership is the fundamental mechanism
- System warns on unlikely table combinations; user decides whether to proceed

### UnlinkTables Operation

```yaml
UnlinkTables:
  source_table: "contacts"
  target_table: "attendees"
  data_handling: "copy"    # "copy" | "discard"
  actor: <actor_id>
  hlc: <timestamp>
```

- Removes a table-level link
- `copy`: entities in both tables have their shared data copied to standalone records before unlinking
- `discard`: shared data is removed from the target table; entities retain only their source table data

### AddToTable Operation

```yaml
AddToTable:
  entity_id: <uuid>
  table: "attendees"
  defaults:
    role: "guest"
  actor: <actor_id>
  hlc: <timestamp>
```

- Adds an existing entity to a table by attaching the table's facet
- Optional `defaults` set initial values on the newly attached facet's fields
- Used for per-entity table membership (as opposed to table-level linking)

### RemoveFromTable Operation

```yaml
RemoveFromTable:
  entity_id: <uuid>
  table: "attendees"
  data_handling: "preserve"   # "preserve" | "discard"
  actor: <actor_id>
  hlc: <timestamp>
```

- Removes an entity from a table by detaching the table's facet
- `preserve`: facet data is soft-deleted (recoverable)
- `discard`: facet data is permanently removed

### ApplyCRDT Operation

Apply a delta update to a CRDT field. See [crdt.md](crdt.md) for CRDT field semantics.

```yaml
ApplyCRDT:
  op_id: <uuid>
  entity_id: <uuid>
  field: "description"
  crdt_type: "text"           # Matches schema declaration
  delta: <opaque_bytes>       # CRDT-specific delta format
  actor_id: <actor>
  hlc: <timestamp>
```

- Field must be declared as CRDT in module schema
- `crdt_type` must match the schema declaration
- `delta` is opaque to the system; interpretation depends on CRDT implementation
- If field has no state, an empty CRDT state is initialized before applying delta
- CRDT operations can be applied in any order and converge to identical state

**Validation:**
- Entity must exist
- Field must be declared with `crdt` attribute in schema
- `crdt_type` must match schema declaration

**Note:** `SetField` on a CRDT-typed field is a validation/type error and is **rejected** at validation time. CRDT fields must be modified via `ApplyCRDT`. Use `ClearAndAdd` for reset operations on CRDT set fields.

### CreateOrderedEdge Operation

Create an edge with position in an ordered edge type. See [ordered-edges.md](ordered-edges.md) for ordering semantics.

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

- Edge type must be declared with `ordered: true` in module schema
- `after` and `before` specify insertion position relative to existing edges
- System generates `_position` property from `after`/`before` references
- Concurrent insertions at the same position use deterministic tiebreaking

**Position resolution:**
- `after: null` -> insert at start
- `before: null` -> insert at end
- Both null -> insert as only/first item
- Both specified -> insert between them

### MoveOrderedEdge Operation

Reorder an existing edge within an ordered edge type.

```yaml
MoveOrderedEdge:
  op_id: <uuid>
  edge_id: <uuid>
  after: <edge_id | null>
  before: <edge_id | null>
  actor_id: <actor>
  hlc: <timestamp>
```

- Edge must exist and be of an ordered type
- Generates new `_position` from `after`/`before` references
- Concurrent moves of the same edge: LWW by HLC (later move wins)

### ClearAndAdd Operation

Reset a CRDT set field to specific values while preserving CRDT semantics.

```yaml
ClearAndAdd:
  op_id: <uuid>
  entity_id: <uuid>
  field: "tags"
  values: ["low", "priority"]
  actor_id: <actor>
  hlc: <timestamp>
```

**Semantics:**
- Removes all elements added before this op's HLC
- Adds the specified values
- Concurrent adds AFTER this HLC still apply (not cleared)

**Use case:** "Reset this set to exactly these values" while allowing concurrent edits to merge properly.

**Why not SetField:** SetField on CRDT fields is rejected because it would silently discard concurrent CRDT edits from other users. ClearAndAdd preserves CRDT merge semantics.

### CreateRule Operation

```yaml
CreateRule:
  rule_id: <uuid>
  name: "Called cues appear in SM cues table"
  when: <query>
  action: <action_spec>
  auto_accept: false
  actor: <actor_id>
  hlc: <timestamp>
```

- Rules are created and modified through explicit operations
- Rule changes are part of the oplog and sync to all peers
- See [rules.md](rules.md) for rule semantics

---

## Oplog & History

- Append-only source of canonical truth
- Full deterministic state reconstructable from oplog
- History is never deleted, only superseded
- Every op has a checksum (hash of content)
- Corrupt ops are detected and quarantined, not applied

### Canonical History Ordering

- Every valid operation has a deterministic position in canonical history
- Canonical history ordering is identical across all peers that have integrated the same operations
- New peers reconstruct history deterministically and observe the same ordering as existing peers

### Timestamps & Causality

- Wall-clock timestamps are untrusted metadata and must not determine history order
- Clock skew is treated as untrusted metadata
- Causal metadata (HLC, wall-clock time, author) may be displayed as annotations, not as ordering authority

### Concurrency Presentation

- Concurrent operations may be labeled or grouped as concurrent, even when a deterministic internal order exists
- History presentation must not imply false causality between concurrent operations

---

## State Model

- State is always derived from oplog
- No state is authoritative
- Rebuilding state is always legal
- Restarting the app produces identical state

### CRDT State Derivation

**Anchor invariant:** CRDT fields are derived using reception order (not canonical order) because CRDT merge is commutative--order doesn't affect the result.

For CRDT fields, `ApplyCRDT` deltas are merged into the field state. For non-CRDT fields, `SetField` operations use LWW with canonical ordering. See [crdt.md](crdt.md) for details.

---

## Deterministic Total Ordering

Operations are ordered deterministically across all peers:

1. Sort by HLC (ascending)
2. Tiebreak by `op_id` when HLCs equal

This produces identical ordering on all peers given the same operations. No leader required for correctness.

### Oplog vs Canonical Order

**Anchor invariant:** The oplog is append-only (reception order). State is derived from canonical order (deterministic sort).

```
+-------------------------------------------------------------+
|  OPLOG (append-only, reception order)                       |
|                                                             |
|  Peer's oplog:  [ B (HLC 100), A (HLC 100) ]               |
|                   ^ created     ^ received                  |
|                                                             |
|  Never reordered. Permanent record of what was received.    |
+-------------------------------------------------------------+
                            |
                            v
+-------------------------------------------------------------+
|  CANONICAL ORDER (derived, deterministic)                   |
|                                                             |
|  Sort by (HLC, op_id):  [ A, B ]                            |
|                                                             |
|  Identical on every peer. Used for state derivation.        |
+-------------------------------------------------------------+
                            |
                            v
+-------------------------------------------------------------+
|  STATE (derived by replaying canonical order)               |
|                                                             |
|  Apply A, then B. Deterministic final state.                |
+-------------------------------------------------------------+
```

- Oplog append order may differ between peers
- Canonical order is always identical given same operations
- State derivation uses canonical order, not reception order

---

## Operation Identity

Every operation has a globally unique identifier:

- **op_id** -- UUIDv7 generated locally (16 bytes, see [data-model.md](data-model.md))
- **HLC** -- Hybrid Logical Clock provides causal ordering (12 bytes, see [hlc.md](hlc.md))
- **actor_id** -- Ed25519 public key of the author (32 bytes, see [identity.md](identity.md))

The `op_id` is generated locally using UUIDv7 (time-sortable, no coordination required). It serves as the tiebreaker when HLCs are equal during canonical ordering.

---

## Operation Structure

```yaml
Operation:
  id: unique_op_id
  actor_id: <ed25519_public_key>   # 32 bytes
  hlc: timestamp
  signature: <ed25519_signature>   # 64 bytes, signs (id + actor_id + hlc + payload)
  module_versions:
    contacts: "1.1.0"
    scheduler: "2.0.0"
  payload: { ... }
```

Operations include the module versions they were created with. This allows replay to interpret operations against the correct schema. Each operation is signed with the actor's Ed25519 private key, enabling peers to verify authorship without trusting the transport layer.

---

## State Hash Verification

After sync quiesces, all connected peers must have identical state:

```
state_hash = hash(sorted_oplog_ids + derived_state)
```

- Peers exchange state hashes after sync completes
- Mismatch indicates bug, corruption, or missing operations
- Hash mismatch triggers diagnostic mode (compare oplogs)

---

## Persistence and Storage

- Atomic append of operations
- Crash safety: partial writes never corrupt oplog (SQLite WAL guarantees)
- On crash recovery, database is consistent (WAL replay)
- Incomplete bundles are discarded on recovery (never partially committed)
- Snapshot-at-index semantics
- Asset references are content-addressed or immutable

---

## Open Questions

- Compression format
- Stream/wire formats
- Bundle types (user_edit, script_output, import, merge_resolution)?
- Max bundle size advisory?
- Snapshotting for new clients, catch-up
- Dedupe/skip superseded ops when building snapshot?
- Pruning strategies
- Archiving
- Checkpoints, flattening, compression
- Caching strategies
- Partial materialization
- In-memory vs disk
- Sharding
- Cloud storage/sync
- DB abstraction layer (SQLite local, Postgres cloud)
