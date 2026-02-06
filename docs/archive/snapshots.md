# Snapshots and Archiving Specification

This document defines snapshots, oplog segmentation, archiving, and optional garbage collection.

---

## Overview

The oplog grows unbounded over time. To maintain performance while preserving history and auditability:

1. **Snapshots** capture derived state at known positions for fast sync
2. **Segments** partition the oplog into immutable, compressible chunks
3. **Archives** relocate old segments to cold storage
4. **Garbage collection** (optional) removes history beyond a retention period

**Anchor invariant:** Snapshots are optimizations, not authority. The oplog remains the source of truth. Any snapshot can be verified by replaying the oplog from the beginning.

---

## Snapshots

### Purpose

Snapshots enable new and catching-up peers to skip replaying the entire oplog. Instead of replaying millions of operations, a peer can:

1. Load a snapshot (derived state at position N)
2. Replay only operations after position N

### Snapshot Structure

```yaml
Snapshot:
  # Identity
  snapshot_id: <UUIDv7>
  workspace_id: <UUID>

  # Position in oplog
  oplog_position: <integer>           # Operations included (0 to position-1)
  boundary_hlc: <HLC>                 # HLC of last included operation

  # Verification
  state_hash: <BLAKE3>                # Hash of serialized state
  oplog_hash: <BLAKE3>                # Hash of oplog[0:position] for verification

  # Authorship
  created_by: <actor_id>
  created_at: <HLC>

  # Signature (same model as operations)
  signature: <Ed25519 signature>

  # Content
  format_version: 1
  data: <compressed state>
```

### Snapshot Content

The `data` field contains serialized derived state:

```yaml
SnapshotData:
  entities:
    - id: <entity_id>
      kind: <string>
      created_at: <HLC>
      created_by: <actor_id>
      deleted_at: <HLC | null>
      redirect_to: <entity_id | null>

  fields:
    - entity_id: <entity_id>
      field_key: <string>             # "name" or "plugin.field"
      value: <any>
      set_at: <HLC>
      set_by: <actor_id>

  edges:
    - edge_id: <edge_id>
      edge_type: <string>
      source_id: <entity_id>
      target_id: <entity_id>
      properties: <map>
      created_at: <HLC>
      created_by: <actor_id>

  facets:
    - entity_id: <entity_id>
      facet: <string>
      attached_at: <HLC>
      attached_by: <actor_id>

  # Metadata state
  actors: [...]
  roles: [...]
  rules: [...]
  triggers: [...]

  # Conflict state (unresolved conflicts at snapshot time)
  conflicts: [...]

  # Proposal state (pending proposals at snapshot time)
  proposals: [...]
```

### Determinism

**Anchor invariant:** Snapshots are deterministic. Given the same oplog prefix, any peer produces an identical snapshot.

Requirements for determinism:
- Entities serialized in sorted order by `entity_id`
- Fields serialized in sorted order by `(entity_id, field_key)`
- All collections sorted by their natural key
- Serialization uses canonical MessagePack (sorted maps, no floats for integers)

### Snapshot Verification

Peers verify snapshots before trusting them:

```
function verify_snapshot(snapshot, oplog):
    # Option A: Full verification (slow, definitive)
    derived_state = replay(oplog[0:snapshot.oplog_position])
    return hash(derived_state) == snapshot.state_hash

    # Option B: Signature verification (fast, requires trust in signer)
    return verify_signature(snapshot.signature, snapshot.created_by)
```

**Trust model:**
- Snapshots from self: trusted without verification
- Snapshots from peers: verify signature, optionally spot-check state hash
- Snapshots from untrusted sources: full verification required

### Snapshot Creation Triggers

Snapshots are created when either condition is met:

| Trigger | Default | Configurable |
|---------|---------|--------------|
| Operation count | Every 10,000 ops | Yes |
| Time elapsed | Every 24 hours | Yes |
| Manual request | User-triggered | N/A |

```yaml
# Workspace configuration
snapshots:
  op_count_interval: 10000    # Create snapshot every N operations
  time_interval_hours: 24     # Create snapshot every N hours
  enabled: true               # Can be disabled for small workspaces
```

### Snapshot Storage

Snapshots are stored locally, separate from the oplog:

```
workspace/
├── canonical.db              # Oplog + derived state
├── snapshots/
│   ├── snapshot_000001.snap  # Oldest
│   ├── snapshot_000002.snap
│   └── snapshot_000003.snap  # Most recent
└── archive/                  # Archived segments
```

**Retention:** Keep the N most recent snapshots (default: 3). Older snapshots can be deleted since they're derivable from the oplog.

---

## Oplog Segments

### Purpose

Segmentation enables:
- Compression of historical operations
- Efficient archiving (move whole segments)
- Parallel sync (transfer multiple segments concurrently)
- Integrity verification at boundaries

### Segment Structure

```yaml
Segment:
  # Identity
  segment_id: <integer>               # Sequential: 0, 1, 2, ...
  workspace_id: <UUID>

  # Boundaries
  start_position: <integer>           # First operation index (inclusive)
  end_position: <integer>             # Last operation index (exclusive)
  start_hlc: <HLC>                    # HLC of first operation
  end_hlc: <HLC>                      # HLC of last operation

  # Verification
  operations_hash: <BLAKE3>           # Hash of all operations in segment
  state_hash_at_end: <BLAKE3>         # Derived state hash after applying segment
  previous_state_hash: <BLAKE3>       # State hash before segment (for chaining)

  # Metadata
  operation_count: <integer>
  created_at: <timestamp>
  compressed: <boolean>
  compression_algo: "zstd" | null

  # Content
  operations: [<Operation>, ...]
```

### Segment Lifecycle

```
┌─────────────────────────────────────────────────────────────────┐
│  Segment Lifecycle                                              │
│                                                                 │
│  1. ACTIVE: Current segment, append-only                        │
│     └─ Operations appended as they arrive                       │
│                                                                 │
│  2. SEALED: Segment full, no more appends                       │
│     └─ Triggered by: op count OR time elapsed                   │
│     └─ State hash computed at boundary                          │
│                                                                 │
│  3. COMPRESSED: Sealed segment compressed                       │
│     └─ Original operations preserved, just compressed           │
│                                                                 │
│  4. ARCHIVED: Moved to archive storage                          │
│     └─ Triggered by: age OR segment count                       │
│     └─ Still accessible, just not in hot path                   │
│                                                                 │
│  5. DELETED (optional): Removed after retention period          │
│     └─ Only if GC enabled and retention exceeded                │
└─────────────────────────────────────────────────────────────────┘
```

### Segment Sealing Triggers

| Trigger | Default | Configurable |
|---------|---------|--------------|
| Operation count | 10,000 ops | Yes |
| Time elapsed | 1 hour | Yes |
| Manual seal | User-triggered | N/A |

```yaml
# Workspace configuration
segments:
  op_count_limit: 10000       # Seal segment after N operations
  time_limit_minutes: 60      # Seal segment after N minutes
  compression_enabled: true   # Compress sealed segments
  compression_level: 3        # zstd compression level (1-19)
```

### Hash Chaining

Segments form a hash chain for integrity verification:

```
Segment 0:
  previous_state_hash: <initial empty state hash>
  state_hash_at_end: H0

Segment 1:
  previous_state_hash: H0
  state_hash_at_end: H1

Segment 2:
  previous_state_hash: H1
  state_hash_at_end: H2
```

**Verification:** A peer can verify segment chain integrity without replaying operations:
```
for each segment:
    assert segment.previous_state_hash == previous_segment.state_hash_at_end
```

Full verification (replay) is only needed if:
- Segment chain is broken
- State hash mismatch detected
- Explicit audit requested

---

## Archive System

### Purpose

Archives relocate old segments from the hot database to cold storage, reducing active database size while preserving full history.

### Archive Abstraction

**Anchor invariant:** Archive storage is abstracted. The core interacts with archives through a storage interface, enabling local files for v1 and cloud storage later.

```rust
trait ArchiveStorage {
    /// Store a segment in the archive
    fn store_segment(&self, segment: &Segment) -> Result<ArchiveRef>;

    /// Retrieve a segment from the archive
    fn retrieve_segment(&self, ref: ArchiveRef) -> Result<Segment>;

    /// List available segments in the archive
    fn list_segments(&self) -> Result<Vec<SegmentMetadata>>;

    /// Delete a segment from the archive (for GC)
    fn delete_segment(&self, ref: ArchiveRef) -> Result<()>;

    /// Check if a segment exists
    fn exists(&self, ref: ArchiveRef) -> bool;
}

// V1: Local filesystem implementation
struct LocalArchiveStorage {
    archive_path: PathBuf,
}

// Future: Cloud storage implementation
// struct S3ArchiveStorage { bucket: String, prefix: String }
// struct GCSArchiveStorage { bucket: String, prefix: String }
```

### Archive Triggers

Segments are archived when either condition is met:

| Trigger | Default | Configurable |
|---------|---------|--------------|
| Segment age | > 30 days old | Yes |
| Active segment count | Keep 10 most recent | Yes |

```yaml
# Workspace configuration
archive:
  enabled: true
  age_days: 30                # Archive segments older than N days
  keep_recent_count: 10       # Keep N most recent segments in active DB
  storage: "local"            # "local" for v1 (future: "s3", "gcs")
  local_path: "./archive"     # Path for local archive storage
```

### Archive Index

The active database maintains an index of archived segments:

```sql
CREATE TABLE archived_segments (
    segment_id INTEGER PRIMARY KEY,
    start_position INTEGER NOT NULL,
    end_position INTEGER NOT NULL,
    start_hlc BLOB NOT NULL,
    end_hlc BLOB NOT NULL,
    state_hash_at_end BLOB NOT NULL,
    archive_ref TEXT NOT NULL,        -- Storage-specific reference
    archived_at INTEGER NOT NULL,
    operation_count INTEGER NOT NULL
);

CREATE INDEX idx_archived_hlc ON archived_segments (start_hlc, end_hlc);
```

### Archive Retrieval

When operations from an archived segment are needed:

```
function get_operations(start_hlc, end_hlc):
    # Check active segments first
    ops = active_db.query_operations(start_hlc, end_hlc)
    if ops.covers_range(start_hlc, end_hlc):
        return ops

    # Find archived segments that cover the gap
    archived = archive_index.find_segments(start_hlc, end_hlc)
    for segment_meta in archived:
        segment = archive_storage.retrieve_segment(segment_meta.archive_ref)
        ops.extend(segment.operations)

    return ops.sorted_by_hlc()
```

---

## Garbage Collection

### Philosophy

**Anchor invariant:** Garbage collection is optional and off by default. Users who need full audit trails keep everything. Users who don't can reclaim space.

GC deletes operations and archived segments beyond a retention period. This is a **destructive, irreversible** operation.

### GC Configuration

```yaml
# Workspace configuration
garbage_collection:
  enabled: false              # Off by default
  retention_days: 365         # Keep operations for N days
  require_confirmation: true  # Prompt before deleting

  # What to preserve even after retention
  preserve:
    - conflict_resolutions    # Always keep resolution history
    - entity_creates          # Always keep creation records
    - entity_deletes          # Always keep deletion records
    - role_changes            # Always keep permission history
```

### GC Process

```
┌─────────────────────────────────────────────────────────────────┐
│  Garbage Collection Process                                     │
│                                                                 │
│  1. Identify segments older than retention period               │
│  2. Check for preserved operation types                         │
│  3. Create "tombstone" summary for deleted operations:          │
│     - Operation count deleted                                   │
│     - Time range covered                                        │
│     - Actors involved                                           │
│  4. Delete archived segment files                               │
│  5. Update archive index                                        │
│  6. Log GC event (auditable)                                    │
└─────────────────────────────────────────────────────────────────┘
```

### GC Tombstones

When operations are deleted, a tombstone preserves metadata:

```yaml
GCTombstone:
  tombstone_id: <UUID>
  segment_ids: [1, 2, 3]              # Deleted segments
  operation_count: 30000              # Total operations deleted
  start_hlc: <HLC>
  end_hlc: <HLC>
  actors_involved: [<actor_id>, ...]  # Who authored deleted ops
  gc_performed_by: <actor_id>
  gc_performed_at: <HLC>
  reason: "Retention period exceeded"
```

Tombstones are stored in the oplog as special operations, preserving the audit trail of what was deleted and when.

### GC Restrictions

GC cannot delete:
- Operations newer than retention period
- The most recent snapshot's backing operations (need at least one valid snapshot)
- Operations referenced by unresolved conflicts
- Operations referenced by pending proposals

---

## Sync Protocol Changes

### New Peer Sync

```
┌─────────────────────────────────────────────────────────────────┐
│  New Peer Sync Flow                                             │
│                                                                 │
│  1. Request latest snapshot from any peer                       │
│  2. Verify snapshot signature                                   │
│  3. Load snapshot as initial state                              │
│  4. Request segments after snapshot.oplog_position              │
│  5. Apply operations from segments                              │
│  6. Request any operations after last segment                   │
│  7. Verify state hash matches peers                             │
└─────────────────────────────────────────────────────────────────┘
```

### Catching-Up Sync

```
┌─────────────────────────────────────────────────────────────────┐
│  Catching-Up Sync Flow                                          │
│                                                                 │
│  1. Exchange vector clocks to identify gap                      │
│  2. If gap is small (< 1 segment): request operations directly  │
│  3. If gap is large: request segments covering the gap          │
│  4. Apply operations in canonical order                         │
│  5. Verify state hash at segment boundaries                     │
└─────────────────────────────────────────────────────────────────┘
```

### Sync Messages

```yaml
# Request snapshot
SnapshotRequest:
  workspace_id: <UUID>
  max_age_hours: 24           # Accept snapshots up to N hours old

SnapshotResponse:
  snapshot: <Snapshot>
  available_segments: [<SegmentMetadata>, ...]

# Request segments
SegmentRequest:
  workspace_id: <UUID>
  segment_ids: [1, 2, 3]

SegmentResponse:
  segments: [<Segment>, ...]

# Request operations (for small gaps)
OperationsRequest:
  workspace_id: <UUID>
  after_hlc: <HLC>
  limit: 1000

OperationsResponse:
  operations: [<Operation>, ...]
  has_more: <boolean>
```

---

## Storage Layout

### Active Database

```sql
-- Segments table (replaces flat oplog for sealed segments)
CREATE TABLE segments (
    segment_id INTEGER PRIMARY KEY,
    start_position INTEGER NOT NULL,
    end_position INTEGER NOT NULL,
    start_hlc BLOB NOT NULL,
    end_hlc BLOB NOT NULL,
    state_hash_at_end BLOB NOT NULL,
    previous_state_hash BLOB NOT NULL,
    operation_count INTEGER NOT NULL,
    compressed BOOLEAN NOT NULL DEFAULT FALSE,
    data BLOB NOT NULL,               -- MessagePack array of operations
    created_at INTEGER NOT NULL
);

-- Active operations (current unsealed segment)
CREATE TABLE active_oplog (
    -- Same schema as original oplog table
    rowid INTEGER PRIMARY KEY,
    op_id BLOB NOT NULL UNIQUE,
    actor_id BLOB NOT NULL,
    hlc BLOB NOT NULL,
    bundle_id BLOB NOT NULL,
    payload BLOB NOT NULL,
    signature BLOB NOT NULL,
    op_type TEXT NOT NULL,
    entity_id BLOB,
    received_at INTEGER NOT NULL
);

-- Snapshot metadata
CREATE TABLE snapshots (
    snapshot_id BLOB PRIMARY KEY,
    oplog_position INTEGER NOT NULL,
    boundary_hlc BLOB NOT NULL,
    state_hash BLOB NOT NULL,
    oplog_hash BLOB NOT NULL,
    created_by BLOB NOT NULL,
    created_at BLOB NOT NULL,
    signature BLOB NOT NULL,
    file_path TEXT NOT NULL           -- Path to snapshot file
);

-- Archive index
CREATE TABLE archived_segments (
    segment_id INTEGER PRIMARY KEY,
    start_position INTEGER NOT NULL,
    end_position INTEGER NOT NULL,
    start_hlc BLOB NOT NULL,
    end_hlc BLOB NOT NULL,
    state_hash_at_end BLOB NOT NULL,
    archive_ref TEXT NOT NULL,
    archived_at INTEGER NOT NULL,
    operation_count INTEGER NOT NULL
);

-- GC tombstones
CREATE TABLE gc_tombstones (
    tombstone_id BLOB PRIMARY KEY,
    segment_ids BLOB NOT NULL,        -- MessagePack array
    operation_count INTEGER NOT NULL,
    start_hlc BLOB NOT NULL,
    end_hlc BLOB NOT NULL,
    actors_involved BLOB NOT NULL,    -- MessagePack array
    gc_performed_by BLOB NOT NULL,
    gc_performed_at BLOB NOT NULL,
    reason TEXT
);
```

### File Layout

```
workspace/
├── canonical.db                      # SQLite database
├── snapshots/
│   ├── 00000001.snapshot             # Compressed snapshot files
│   ├── 00000002.snapshot
│   └── 00000003.snapshot
├── archive/
│   ├── segment_000000.seg.zst        # Archived, compressed segments
│   ├── segment_000001.seg.zst
│   └── segment_000002.seg.zst
└── blobs/                            # Asset storage (unchanged)
```

---

## Configuration Summary

```yaml
# Full configuration with defaults
snapshots:
  enabled: true
  op_count_interval: 10000
  time_interval_hours: 24
  keep_count: 3

segments:
  op_count_limit: 10000
  time_limit_minutes: 60
  compression_enabled: true
  compression_level: 3

archive:
  enabled: true
  age_days: 30
  keep_recent_count: 10
  storage: "local"
  local_path: "./archive"

garbage_collection:
  enabled: false                      # Off by default
  retention_days: 365
  require_confirmation: true
  preserve:
    - conflict_resolutions
    - entity_creates
    - entity_deletes
    - role_changes
```

---

## Open Questions

- Snapshot format: MessagePack vs SQLite dump vs custom binary?
- Compression algorithm: zstd (default) vs lz4 (faster) vs user choice?
- Cloud archive authentication and encryption model?
- Snapshot sharing between workspaces (for forks)?
- Incremental snapshots (delta from previous snapshot)?

