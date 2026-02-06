# SQLite Schema Specification

This document defines the exact table definitions, indexes, and configuration for local SQLite storage.

---

## Database Architecture

**Anchor invariant:** The oplog is append-only and is the source of truth. All other tables are materialized views that can be rebuilt from the oplog.

### Database File

The system uses a single SQLite database file: **`oplog.db`**

All tables -- both canonical (synced) and local-only (never synced) -- live in this single file. Local-only tables are clearly marked in their section headers below.

| Scope | Tables | Syncs to Peers |
|-------|--------|----------------|
| Canonical | oplog, bundles, entities, fields, edges, facets, blobs, conflicts, tables, table memberships, rules, scripts, triggers, actors, workspace config, modules | Yes |
| Local-only | overlays, overlay_ops, sync_state, local_vector_clock, peer_info, module_local_data, stale_operation_flags, awareness_events, rule/trigger dependencies, execution state | No |

**Rationale for single database:**
- Simpler implementation: one connection, one WAL, one backup
- Foreign keys can reference across canonical and local tables
- Local-only tables are clearly identified by convention (and by the sync layer, which knows which tables to replicate)
- Local-only data can be rebuilt from canonical state if needed

---

## SQLite Configuration

### PRAGMA Settings

Applied on every database connection:

```sql
-- Write-Ahead Logging for crash safety and concurrent reads
PRAGMA journal_mode = WAL;

-- Synchronous mode: NORMAL provides good durability with better performance
-- FULL would sync after every transaction (slower but safer)
PRAGMA synchronous = NORMAL;

-- Foreign key enforcement
PRAGMA foreign_keys = ON;

-- Increase cache size (negative = KiB, default is ~2MB)
PRAGMA cache_size = -32000;  -- 32MB cache

-- Memory-mapped I/O for read performance (256MB)
PRAGMA mmap_size = 268435456;

-- Busy timeout for concurrent access (5 seconds)
PRAGMA busy_timeout = 5000;

-- Enable strict mode for type checking
PRAGMA strict = ON;
```

### WAL Mode Rationale

WAL (Write-Ahead Logging) is required for:
- **Crash safety:** Partial writes never corrupt the database
- **Concurrent reads:** Readers don't block writers
- **Atomic bundles:** Multi-statement transactions are atomic

**Recovery behavior:**
- On crash, SQLite automatically replays the WAL on next open
- Incomplete transactions are rolled back
- Database is always in a consistent state

---

## Canonical Tables (Synced)

### oplog Table

Append-only log of all operations. This is the source of truth.

```sql
CREATE TABLE oplog (
    -- Reception order (local append sequence)
    rowid INTEGER PRIMARY KEY,

    -- Operation identity
    op_id BLOB NOT NULL UNIQUE CHECK (length(op_id) = 16),  -- UUID (16 bytes)
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 32),   -- Ed25519 public key (32 bytes)
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),             -- HLC timestamp (12 bytes, see hlc.md)

    -- Bundle membership
    bundle_id BLOB NOT NULL,              -- UUID (16 bytes)

    -- Operation content (MessagePack encoded)
    payload BLOB NOT NULL,

    -- Signature for verification
    signature BLOB NOT NULL,              -- Ed25519 signature (64 bytes)

    -- Denormalized for efficient queries
    op_type TEXT NOT NULL,                -- 'set_field', 'attach_facet', etc.
    entity_id BLOB,                       -- Target entity (if applicable)

    -- Timestamps for debugging (not authoritative)
    received_at INTEGER NOT NULL DEFAULT (unixepoch('now', 'subsec') * 1000)
);

-- Canonical ordering index (HLC + hash for deterministic sort)
CREATE INDEX idx_oplog_canonical_order ON oplog (hlc, op_id);

-- Find operations by actor (for vector clock queries)
CREATE INDEX idx_oplog_actor_hlc ON oplog (actor_id, hlc);

-- Find operations affecting an entity
CREATE INDEX idx_oplog_entity ON oplog (entity_id, hlc) WHERE entity_id IS NOT NULL;

-- Find operations in a bundle
CREATE INDEX idx_oplog_bundle ON oplog (bundle_id);
```

**Index justifications:**
- `idx_oplog_canonical_order`: Efficient canonical order iteration for state derivation
- `idx_oplog_actor_hlc`: Vector clock queries ("what ops from actor X after HLC Y?")
- `idx_oplog_entity`: Entity history queries
- `idx_oplog_bundle`: Bundle membership queries, undo operations

### bundles Table

Bundle metadata for atomic operation groups.

```sql
CREATE TABLE bundles (
    bundle_id BLOB PRIMARY KEY CHECK (length(bundle_id) = 16),  -- UUID (16 bytes)
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 32),       -- Bundle author
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),                 -- Bundle timestamp (max HLC of ops)
    bundle_type INTEGER NOT NULL,         -- 1=user_edit, 2=script_output, etc.

    -- Entity lifecycle markers (MessagePack arrays of UUIDs)
    creates BLOB,                         -- Entities created in this bundle
    deletes BLOB,                         -- Entities deleted in this bundle

    -- Metadata (MessagePack map)
    meta BLOB,

    -- Bundle signature
    signature BLOB NOT NULL,

    -- Operation count for integrity
    op_count INTEGER NOT NULL,

    -- Reception timestamp
    received_at INTEGER NOT NULL DEFAULT (unixepoch('now', 'subsec') * 1000)
);

-- Chronological bundle listing
CREATE INDEX idx_bundles_hlc ON bundles (hlc);

-- Find bundles by author
CREATE INDEX idx_bundles_actor ON bundles (actor_id, hlc);

-- Find bundles by type (e.g., all imports)
CREATE INDEX idx_bundles_type ON bundles (bundle_type, hlc);
```

**Index justifications:**
- `idx_bundles_hlc`: History view, chronological iteration
- `idx_bundles_actor`: "Show my recent changes" queries
- `idx_bundles_type`: Filter by bundle type (imports, scripts, etc.)

### entities Table

Entity existence and redirect tracking. Entity "type" is derived from table membership and facet attachment, not stored as a field.

```sql
CREATE TABLE entities (
    entity_id BLOB PRIMARY KEY CHECK (length(entity_id) = 16),  -- UUIDv7 (16 bytes)

    -- Lifecycle state
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),  -- HLC of creation
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),  -- Actor who created
    created_in_bundle BLOB NOT NULL,      -- Bundle that created this entity

    deleted_at BLOB CHECK (deleted_at IS NULL OR length(deleted_at) = 12),  -- HLC of deletion (NULL if active)
    deleted_by BLOB CHECK (deleted_by IS NULL OR length(deleted_by) = 32),  -- Actor who deleted
    deleted_in_bundle BLOB,               -- Bundle that deleted this entity

    -- Redirect for merged entities (NULL if not redirected)
    redirect_to BLOB,                     -- Target entity ID
    redirect_at BLOB CHECK (redirect_at IS NULL OR length(redirect_at) = 12),  -- HLC of redirect

    FOREIGN KEY (redirect_to) REFERENCES entities(entity_id),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (deleted_in_bundle) REFERENCES bundles(bundle_id)
);

-- Find active entities (not deleted, not redirected)
CREATE INDEX idx_entities_active ON entities (created_at) WHERE deleted_at IS NULL AND redirect_to IS NULL;

-- Find deleted entities (for history, undo)
CREATE INDEX idx_entities_deleted ON entities (deleted_at) WHERE deleted_at IS NOT NULL;

-- Find redirects (for transparent resolution)
CREATE INDEX idx_entities_redirects ON entities (redirect_to) WHERE redirect_to IS NOT NULL;
```

**Index justifications:**
- `idx_entities_active`: Find active (non-deleted, non-redirected) entities
- `idx_entities_deleted`: Deleted entity listing, undo operations
- `idx_entities_redirects`: Redirect chain resolution

### fields Table

Materialized view of current field values. Rebuilt from oplog.

```sql
CREATE TABLE fields (
    entity_id BLOB NOT NULL,
    field_key TEXT NOT NULL,              -- Shared key or namespaced (module.field)

    -- Current value (MessagePack encoded)
    value BLOB NOT NULL,

    -- Provenance
    source_op BLOB NOT NULL,              -- Operation that set this value
    source_actor BLOB NOT NULL CHECK (length(source_actor) = 32),  -- Actor who wrote this
    updated_at BLOB NOT NULL CHECK (length(updated_at) = 12),      -- HLC of the write

    PRIMARY KEY (entity_id, field_key),
    FOREIGN KEY (entity_id) REFERENCES entities(entity_id),
    FOREIGN KEY (source_op) REFERENCES oplog(op_id)
);

-- Find all fields for an entity (primary access pattern)
-- Covered by PRIMARY KEY

-- Find entities by field value (for matching rules)
CREATE INDEX idx_fields_key_value ON fields (field_key, value);

-- Find fields by source (for undo, attribution)
CREATE INDEX idx_fields_source_op ON fields (source_op);
```

**Index justifications:**
- Primary key covers entity field lookups
- `idx_fields_key_value`: Matching rules ("find entities where name = X")
- `idx_fields_source_op`: Undo operations, attribution queries

**CRDT field storage:**

CRDT fields store their state in the `value` column as MessagePack with a special structure:

```yaml
# CRDT field value encoding
{
  "_crdt": true,              # Marker for CRDT storage
  "type": "text",             # CRDT type: "text" or "list"
  "state": <bytes>,           # Opaque CRDT state (implementation-dependent)
  "rendered": "..."           # Cached human-readable value
}
```

For CRDT fields, `source_op` is the most recent operation that modified the field (not "the operation that set the current value" since CRDT values are derived from multiple operations).

See [crdt.md](crdt.md) for CRDT field semantics.

### edges Table

Relationships between entities.

```sql
CREATE TABLE edges (
    edge_id BLOB PRIMARY KEY CHECK (length(edge_id) = 16),  -- UUID (16 bytes)
    edge_type TEXT NOT NULL,              -- Namespaced type (module.edge_type)
    source_id BLOB NOT NULL,              -- Source entity
    target_id BLOB NOT NULL,              -- Target entity

    -- Edge properties (MessagePack map)
    properties BLOB,

    -- Provenance
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),  -- HLC of creation
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),  -- Actor who created
    created_in_bundle BLOB NOT NULL,

    deleted_at BLOB CHECK (deleted_at IS NULL OR length(deleted_at) = 12),  -- HLC of deletion (NULL if active)
    deleted_by BLOB CHECK (deleted_by IS NULL OR length(deleted_by) = 32),
    deleted_in_bundle BLOB,

    FOREIGN KEY (source_id) REFERENCES entities(entity_id),
    FOREIGN KEY (target_id) REFERENCES entities(entity_id),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (deleted_in_bundle) REFERENCES bundles(bundle_id)
);

-- Outgoing edges from an entity
CREATE INDEX idx_edges_source ON edges (source_id, edge_type) WHERE deleted_at IS NULL;

-- Incoming edges to an entity
CREATE INDEX idx_edges_target ON edges (target_id, edge_type) WHERE deleted_at IS NULL;

-- Find all edges of a type
CREATE INDEX idx_edges_type ON edges (edge_type) WHERE deleted_at IS NULL;

-- Find deleted edges (for cascade restoration on undo)
CREATE INDEX idx_edges_deleted ON edges (deleted_in_bundle) WHERE deleted_at IS NOT NULL;
```

**Index justifications:**
- `idx_edges_source`: Outgoing edge traversal ("edges from entity X")
- `idx_edges_target`: Incoming edge traversal ("edges to entity X")
- `idx_edges_type`: Global edge type queries
- `idx_edges_deleted`: Undo cascade restoration

**Ordered edges:**

Edge types with `ordered: true` store a `_position` property for deterministic ordering:

```yaml
# properties column for ordered edges (MessagePack)
{
  "call_text": "GO",
  "_position": "Pm3xK"    # Lexicographically sortable position identifier
}
```

Ordered edges are queried by `(target_id, edge_type)` and sorted by `_position`. The existing `idx_edges_target` index supports this query pattern; application code extracts and sorts by `_position` from properties.

See [ordered-edges.md](ordered-edges.md) for ordered edge semantics.

### facets Table

Facet attachments to entities.

```sql
CREATE TABLE facets (
    entity_id BLOB NOT NULL,
    facet_type TEXT NOT NULL,             -- Namespaced (module.FacetName)

    -- Attachment provenance
    attached_at BLOB NOT NULL CHECK (length(attached_at) = 12),  -- HLC of attachment
    attached_by BLOB NOT NULL CHECK (length(attached_by) = 32),  -- Actor who attached
    attached_in_bundle BLOB NOT NULL,
    source_type TEXT NOT NULL,            -- 'user', 'rule', 'import', etc.
    source_id TEXT,                       -- Rule ID if rule-triggered

    -- Detachment state
    detached_at BLOB CHECK (detached_at IS NULL OR length(detached_at) = 12),  -- HLC of detachment (NULL if attached)
    detached_by BLOB CHECK (detached_by IS NULL OR length(detached_by) = 32),
    detached_in_bundle BLOB,
    preserve_values BLOB,                 -- Stashed field values (if preserve=true)

    PRIMARY KEY (entity_id, facet_type),
    FOREIGN KEY (entity_id) REFERENCES entities(entity_id),
    FOREIGN KEY (attached_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (detached_in_bundle) REFERENCES bundles(bundle_id)
);

-- Find all facets for an entity
-- Covered by PRIMARY KEY

-- Find entities with a specific facet
CREATE INDEX idx_facets_type ON facets (facet_type) WHERE detached_at IS NULL;

-- Find rule-attached facets (for rule modification impact analysis)
CREATE INDEX idx_facets_source ON facets (source_type, source_id) WHERE detached_at IS NULL;
```

**Index justifications:**
- Primary key covers entity facet lookups
- `idx_facets_type`: "Find all entities with Contact facet"
- `idx_facets_source`: Impact analysis when rules change

### blobs Table

Content-addressed blob metadata. Blob content stored separately.

```sql
CREATE TABLE blobs (
    blob_hash BLOB PRIMARY KEY,           -- BLAKE3 hash (32 bytes)
    size INTEGER NOT NULL,                -- Uncompressed size in bytes
    mime_type TEXT,                       -- MIME type if known

    -- Storage state
    stored_at INTEGER NOT NULL,           -- Unix timestamp of storage
    storage_path TEXT,                    -- Relative path in blob storage
    compressed_size INTEGER,              -- Size on disk (if compressed)

    -- Reference counting for GC
    ref_count INTEGER NOT NULL DEFAULT 1,

    -- Sync state
    synced_to_peers BLOB                  -- Bitmask or list of peer IDs
);

-- Find blobs by size (for storage analysis)
CREATE INDEX idx_blobs_size ON blobs (size);

-- Find unreferenced blobs (for GC)
CREATE INDEX idx_blobs_gc ON blobs (ref_count, stored_at) WHERE ref_count = 0;
```

**Index justifications:**
- `idx_blobs_size`: Storage analysis, large blob identification
- `idx_blobs_gc`: Garbage collection candidate identification

---

## Table Model

Tables are the user-facing data model. Modules declare tables with schemas; users see tables, records, and fields. Under the hood, table membership maps to facet attachment on entities. See [ARCHITECTURE.md](../ARCHITECTURE.md) for the user-facing vs. internal model mapping.

### tables Table

Table definitions declared by modules.

```sql
CREATE TABLE tables (
    table_id BLOB PRIMARY KEY CHECK (length(table_id) = 16),  -- UUID (16 bytes)
    name TEXT NOT NULL,
    module_id TEXT NOT NULL,              -- Module that declared this table

    -- Schema definition (MessagePack encoded)
    -- Contains field names, types, defaults, validation rules
    schema BLOB NOT NULL,

    -- Metadata
    description TEXT,
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),  -- HLC of creation
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),  -- Actor who created
    created_in_bundle BLOB NOT NULL,

    FOREIGN KEY (module_id) REFERENCES modules(module_id),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id)
);

-- Find tables by module
CREATE INDEX idx_tables_module ON tables (module_id);

-- Find tables by name
CREATE INDEX idx_tables_name ON tables (name);
```

**Index justifications:**
- `idx_tables_module`: Find all tables declared by a module
- `idx_tables_name`: Lookup tables by name for user-facing queries

### table_memberships Table

Per-entity table membership. An entity can belong to multiple tables individually. This is the fundamental mechanism; table-level linking is a convenience built on top.

```sql
CREATE TABLE table_memberships (
    entity_id BLOB NOT NULL,
    table_id BLOB NOT NULL,

    -- Membership provenance
    added_at BLOB NOT NULL CHECK (length(added_at) = 12),  -- HLC of addition
    added_by BLOB NOT NULL CHECK (length(added_by) = 32),  -- Actor who added
    added_in_bundle BLOB NOT NULL,
    source_type TEXT NOT NULL,            -- 'user', 'rule', 'import', 'link'
    source_id TEXT,                       -- Rule ID or link ID if applicable

    -- Removal state
    removed_at BLOB CHECK (removed_at IS NULL OR length(removed_at) = 12),  -- HLC of removal (NULL if active)
    removed_by BLOB CHECK (removed_by IS NULL OR length(removed_by) = 32),
    removed_in_bundle BLOB,

    PRIMARY KEY (entity_id, table_id),
    FOREIGN KEY (entity_id) REFERENCES entities(entity_id),
    FOREIGN KEY (table_id) REFERENCES tables(table_id),
    FOREIGN KEY (added_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (removed_in_bundle) REFERENCES bundles(bundle_id)
);

-- Find all entities in a table (the primary user-facing query)
CREATE INDEX idx_table_memberships_table ON table_memberships (table_id) WHERE removed_at IS NULL;

-- Find all tables an entity belongs to
-- Covered by PRIMARY KEY

-- Find memberships added by a rule (for rule modification impact analysis)
CREATE INDEX idx_table_memberships_source ON table_memberships (source_type, source_id) WHERE removed_at IS NULL;
```

**Index justifications:**
- Primary key covers entity-to-table lookups
- `idx_table_memberships_table`: "Show all records in the Contacts table"
- `idx_table_memberships_source`: Impact analysis when rules change

### table_links Table

Which tables are linked (convenience relationships between tables). Table-level linking is a shortcut; per-entity membership is the fundamental mechanism.

```sql
CREATE TABLE table_links (
    table_a_id BLOB NOT NULL,
    table_b_id BLOB NOT NULL,

    -- Link metadata
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),  -- HLC of link creation
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),  -- Actor who created
    created_in_bundle BLOB NOT NULL,

    -- Removal state
    removed_at BLOB CHECK (removed_at IS NULL OR length(removed_at) = 12),
    removed_by BLOB CHECK (removed_by IS NULL OR length(removed_by) = 32),
    removed_in_bundle BLOB,

    PRIMARY KEY (table_a_id, table_b_id),
    CHECK (table_a_id < table_b_id),      -- Canonical ordering to prevent duplicates
    FOREIGN KEY (table_a_id) REFERENCES tables(table_id),
    FOREIGN KEY (table_b_id) REFERENCES tables(table_id),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (removed_in_bundle) REFERENCES bundles(bundle_id)
);

-- Find links for a specific table
CREATE INDEX idx_table_links_a ON table_links (table_a_id) WHERE removed_at IS NULL;
CREATE INDEX idx_table_links_b ON table_links (table_b_id) WHERE removed_at IS NULL;
```

**Index justifications:**
- `idx_table_links_a`, `idx_table_links_b`: Find all tables linked to a given table (need both since the canonical ordering means a table could be in either column)

### shared_key_suggestions Table

Module-suggested shared key mappings between tables. Modules suggest field mappings based on shared key overlap; users confirm or reject each mapping. Confirmed mappings behave identically to classic shared key semantics. See [ARCHITECTURE.md](../ARCHITECTURE.md) for the suggested-confirmed field mapping model.

```sql
CREATE TABLE shared_key_suggestions (
    source_table BLOB NOT NULL,
    target_table BLOB NOT NULL,
    source_field TEXT NOT NULL,
    target_field TEXT NOT NULL,

    -- Suggestion status
    status TEXT NOT NULL DEFAULT 'suggested',  -- 'suggested', 'confirmed', 'rejected'

    -- Provenance
    suggested_by_module TEXT NOT NULL,     -- Module that suggested this mapping
    suggested_at BLOB NOT NULL CHECK (length(suggested_at) = 12),  -- HLC of suggestion

    -- Confirmation/rejection
    resolved_by BLOB CHECK (resolved_by IS NULL OR length(resolved_by) = 32),  -- Actor who confirmed/rejected
    resolved_at BLOB CHECK (resolved_at IS NULL OR length(resolved_at) = 12),  -- HLC of resolution
    resolved_in_bundle BLOB,

    PRIMARY KEY (source_table, target_table, source_field, target_field),
    FOREIGN KEY (source_table) REFERENCES tables(table_id),
    FOREIGN KEY (target_table) REFERENCES tables(table_id),
    FOREIGN KEY (resolved_in_bundle) REFERENCES bundles(bundle_id)
);

-- Find suggestions by status (for the confirmation UI)
CREATE INDEX idx_shared_key_suggestions_status ON shared_key_suggestions (status);

-- Find confirmed mappings between two tables
CREATE INDEX idx_shared_key_suggestions_confirmed ON shared_key_suggestions (source_table, target_table)
    WHERE status = 'confirmed';

-- Find suggestions from a specific module
CREATE INDEX idx_shared_key_suggestions_module ON shared_key_suggestions (suggested_by_module);
```

**Index justifications:**
- `idx_shared_key_suggestions_status`: Find pending suggestions for the confirmation UI
- `idx_shared_key_suggestions_confirmed`: Lookup active field mappings between linked tables
- `idx_shared_key_suggestions_module`: Find all suggestions from a module (for module removal impact analysis)

---

## Local-Only Tables (Never Synced)

### local_oplog Table

Local-only module operations. Never synced to peers.

```sql
CREATE TABLE local_oplog (
    rowid INTEGER PRIMARY KEY,

    -- Operation identity
    op_id BLOB NOT NULL UNIQUE CHECK (length(op_id) = 16),
    module_id TEXT NOT NULL,              -- Module that owns this data
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),

    -- Operation content (MessagePack encoded)
    payload BLOB NOT NULL,

    -- Target entity (if applicable)
    entity_id BLOB,

    -- Timestamp
    created_at INTEGER NOT NULL DEFAULT (unixepoch('now', 'subsec') * 1000)
);

-- Find local ops by module
CREATE INDEX idx_local_oplog_module ON local_oplog (module_id, hlc);

-- Find local ops for an entity
CREATE INDEX idx_local_oplog_entity ON local_oplog (entity_id) WHERE entity_id IS NOT NULL;
```

**Index justifications:**
- `idx_local_oplog_module`: Module-specific local data queries
- `idx_local_oplog_entity`: Entity-scoped local data

### overlays Table

Staging overlay metadata. See [overlays.md](overlays.md).

```sql
CREATE TABLE overlays (
    overlay_id BLOB PRIMARY KEY,          -- UUID
    display_name TEXT NOT NULL,
    source TEXT NOT NULL,                 -- 'user' or 'script'
    source_id BLOB NOT NULL,              -- Actor ID or script execution ID
    status TEXT NOT NULL,                 -- 'active', 'stashed', 'committed', 'discarded'

    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,

    -- For script overlays
    script_id TEXT,                       -- Script identifier (module.script_name)
    script_execution_id BLOB,             -- Unique execution instance ID

    -- Metadata (MessagePack map)
    meta BLOB
);

-- Find overlays by status
CREATE INDEX idx_overlays_status ON overlays (status, updated_at);

-- Find overlays by source
CREATE INDEX idx_overlays_source ON overlays (source, source_id);
```

### overlay_ops Table

Operations within staging overlays.

```sql
CREATE TABLE overlay_ops (
    rowid INTEGER PRIMARY KEY,
    overlay_id BLOB NOT NULL CHECK (length(overlay_id) = 16),

    -- Operation identity
    op_id BLOB NOT NULL CHECK (length(op_id) = 16),
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),

    -- Operation content (MessagePack encoded)
    payload BLOB NOT NULL,

    -- Target entity
    entity_id BLOB,
    op_type TEXT NOT NULL,

    -- Canonical drift tracking
    canonical_value_at_creation BLOB,     -- Value when overlay op was created
    canonical_drifted INTEGER NOT NULL DEFAULT 0,

    FOREIGN KEY (overlay_id) REFERENCES overlays(overlay_id) ON DELETE CASCADE
);

-- Find ops in an overlay (in order)
CREATE INDEX idx_overlay_ops_overlay ON overlay_ops (overlay_id, rowid);

-- Find overlay ops affecting an entity
CREATE INDEX idx_overlay_ops_entity ON overlay_ops (entity_id) WHERE entity_id IS NOT NULL;
```

### sync_state Table

Vector clocks and peer synchronization state.

```sql
CREATE TABLE sync_state (
    -- Composite key: we track HLC per (peer, actor)
    peer_id BLOB NOT NULL CHECK (length(peer_id) = 32),    -- Peer we sync with (actor ID)
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 32),  -- Actor whose ops we're tracking

    -- Last seen HLC from this actor via this peer
    last_hlc BLOB NOT NULL CHECK (length(last_hlc) = 12),

    -- Sync session info
    last_sync_at INTEGER,                 -- Unix timestamp of last sync

    PRIMARY KEY (peer_id, actor_id)
);

-- Our own vector clock (peer_id = our actor_id)
-- Find peers we sync with
CREATE INDEX idx_sync_state_peer ON sync_state (peer_id, last_sync_at);
```

### local_vector_clock Table

Our local vector clock (what we've seen from each actor).

```sql
CREATE TABLE local_vector_clock (
    actor_id BLOB PRIMARY KEY CHECK (length(actor_id) = 32),  -- Actor (Ed25519 public key)
    last_hlc BLOB NOT NULL CHECK (length(last_hlc) = 12),     -- Last HLC we've integrated from this actor
    op_count INTEGER NOT NULL DEFAULT 0                        -- Total ops from this actor
);
```

### peer_info Table

Known peers and their state.

```sql
CREATE TABLE peer_info (
    peer_id BLOB PRIMARY KEY,             -- Peer's actor ID
    display_name TEXT,

    -- Connection info
    last_address TEXT,                    -- Last known address
    last_seen_at INTEGER,                 -- Unix timestamp

    -- Sync state
    is_leader INTEGER NOT NULL DEFAULT 0,
    leader_epoch INTEGER,

    -- Trust/permission info
    trust_level TEXT,                     -- 'full', 'read_only', etc.

    -- Metadata
    meta BLOB
);

-- Find active peers
CREATE INDEX idx_peer_info_active ON peer_info (last_seen_at);
```

### module_local_data Table

Module-specific local storage (key-value).

```sql
CREATE TABLE module_local_data (
    module_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value BLOB NOT NULL,
    updated_at INTEGER NOT NULL,

    PRIMARY KEY (module_id, key)
);

-- Find all data for a module
-- Covered by PRIMARY KEY
```

### stale_operation_flags Table

Flags for operations that arrived late (stale operations, see [hlc.md](hlc.md)). These flags are **informational only** — stale operations ARE applied to canonical state; the flag helps users notice that old changes arrived.

```sql
CREATE TABLE stale_operation_flags (
    op_id BLOB PRIMARY KEY CHECK (length(op_id) = 16),
    bundle_id BLOB NOT NULL CHECK (length(bundle_id) = 16),

    -- Staleness details
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),
    age_ms INTEGER NOT NULL,              -- How old the operation was when received
    threshold_ms INTEGER NOT NULL,        -- Threshold that was exceeded

    -- Review state (informational, not blocking)
    status TEXT NOT NULL DEFAULT 'unreviewed',  -- 'unreviewed', 'reviewed', 'dismissed'
    reviewed_at INTEGER,                  -- Unix timestamp when user acknowledged
    reviewed_by BLOB,                     -- Actor who reviewed

    -- Metadata
    flagged_at INTEGER NOT NULL DEFAULT (unixepoch('now', 'subsec') * 1000),

    FOREIGN KEY (op_id) REFERENCES oplog(op_id)
);

-- Find unreviewed flags by age
CREATE INDEX idx_stale_flags_status ON stale_operation_flags (status, flagged_at);

-- Find flags by bundle (for bulk review)
CREATE INDEX idx_stale_flags_bundle ON stale_operation_flags (bundle_id) WHERE status = 'unreviewed';
```

**Note:** Stale operations ARE valid operations that are applied to canonical state immediately. This table only tracks the informational flag — users can review to acknowledge they've seen the late-arriving changes, but cannot "reject" them (that would discard legitimate committed work). Actual conflicts between stale ops and newer edits are handled by the normal conflict resolution system.

---

## Migration Strategy

### Schema Versioning

```sql
CREATE TABLE schema_version (
    db_name TEXT PRIMARY KEY,             -- 'oplog'
    version INTEGER NOT NULL,
    migrated_at INTEGER NOT NULL
);
```

### Migration Execution

1. **Check version:** On startup, compare `schema_version` to expected version
2. **Backup:** Before migration, copy database file (or rely on WAL)
3. **Transaction:** Run all migration steps in a single transaction
4. **Update version:** Set new version number
5. **Commit:** Atomic commit of migration

```sql
-- Example migration (version 1 -> 2)
BEGIN IMMEDIATE;

-- Add new column
ALTER TABLE entities ADD COLUMN archived_at BLOB;

-- Create new index
CREATE INDEX idx_entities_archived ON entities (archived_at) WHERE archived_at IS NOT NULL;

-- Update version
UPDATE schema_version SET version = 2, migrated_at = unixepoch('now') WHERE db_name = 'oplog';

COMMIT;
```

### Migration Rules

- Migrations are forward-only (no downgrades)
- Migrations must be idempotent (safe to re-run)
- Migrations must preserve data integrity
- New columns should have defaults or be nullable
- Index changes are safe (can be rebuilt)
- Table renames require data migration

### State Rebuild

Since all tables except `oplog` are materialized views:

```sql
-- Nuclear rebuild option (if state is corrupted)
-- 1. Drop all materialized tables
-- 2. Recreate schemas
-- 3. Replay oplog to rebuild state

-- This is always safe because oplog is the source of truth
```

---

## Constraints Summary

### Foreign Key Relationships

```
-- Core tables
bundles.bundle_id <-- oplog.bundle_id
bundles.bundle_id <-- entities.created_in_bundle
bundles.bundle_id <-- entities.deleted_in_bundle
entities.entity_id <-- entities.redirect_to
entities.entity_id <-- fields.entity_id
oplog.op_id <-- fields.source_op
entities.entity_id <-- edges.source_id
entities.entity_id <-- edges.target_id
bundles.bundle_id <-- edges.created_in_bundle
bundles.bundle_id <-- edges.deleted_in_bundle
entities.entity_id <-- facets.entity_id
bundles.bundle_id <-- facets.attached_in_bundle
bundles.bundle_id <-- facets.detached_in_bundle
overlays.overlay_id <-- overlay_ops.overlay_id (CASCADE DELETE)

-- Conflicts
entities.entity_id <-- conflicts.entity_id
oplog.op_id <-- conflicts.op_id_1
oplog.op_id <-- conflicts.op_id_2
oplog.op_id <-- conflicts.resolution_op_id
bundles.bundle_id <-- conflicts.detected_in_bundle
conflicts.conflict_id <-- conflict_values.conflict_id (CASCADE DELETE)
oplog.op_id <-- conflict_values.op_id

-- Table model
modules.module_id <-- tables.module_id
bundles.bundle_id <-- tables.created_in_bundle
entities.entity_id <-- table_memberships.entity_id
tables.table_id <-- table_memberships.table_id
bundles.bundle_id <-- table_memberships.added_in_bundle
bundles.bundle_id <-- table_memberships.removed_in_bundle
tables.table_id <-- table_links.table_a_id
tables.table_id <-- table_links.table_b_id
bundles.bundle_id <-- table_links.created_in_bundle
bundles.bundle_id <-- table_links.removed_in_bundle
tables.table_id <-- shared_key_suggestions.source_table
tables.table_id <-- shared_key_suggestions.target_table
bundles.bundle_id <-- shared_key_suggestions.resolved_in_bundle

-- Rules and automation
bundles.bundle_id <-- rules.created_in_bundle
bundles.bundle_id <-- scripts.created_in_bundle
scripts.script_id <-- triggers.script_id
bundles.bundle_id <-- triggers.created_in_bundle

-- Identity repair
entities.entity_id <-- merge_resolutions.survivor_id
oplog.op_id <-- merge_resolutions.merge_operation_id
bundles.bundle_id <-- merge_resolutions.merged_in_bundle
entities.entity_id <-- merge_exceptions.entity_a
entities.entity_id <-- merge_exceptions.entity_b
bundles.bundle_id <-- merge_exceptions.created_in_bundle

-- Identity
bundles.bundle_id <-- actors.first_seen_in_bundle
bundles.bundle_id <-- actors.revoked_in_bundle
-- (role_assignments and custom_roles deferred to post-v1)

-- Workspace configuration
bundles.bundle_id <-- access_keys.created_in_bundle
bundles.bundle_id <-- access_keys.revoked_in_bundle
bundles.bundle_id <-- workspace_config.updated_in_bundle
bundles.bundle_id <-- modules.adopted_in_bundle
bundles.bundle_id <-- modules.removed_in_bundle
tables.table_id <-- edge_bindings.source_table
bundles.bundle_id <-- edge_bindings.created_in_bundle
bundles.bundle_id <-- field_overrides.created_in_bundle
bundles.bundle_id <-- custom_shared_keys.created_in_bundle
```

### Uniqueness Constraints

| Table | Unique Columns |
|-------|---------------|
| oplog | op_id |
| bundles | bundle_id |
| entities | entity_id |
| fields | (entity_id, field_key) |
| edges | edge_id |
| facets | (entity_id, facet_type) |
| blobs | blob_hash |
| local_oplog | op_id |
| overlays | overlay_id |
| conflicts | conflict_id |
| conflict_values | (conflict_id, value_index) |
| tables | table_id |
| table_memberships | (entity_id, table_id) |
| table_links | (table_a_id, table_b_id) |
| shared_key_suggestions | (source_table, target_table, source_field, target_field) |
| rules | rule_id, name |
| scripts | script_id, (script_type, module_id, name) |
| triggers | trigger_id, name |
| merge_resolutions | absorbed_id |
| merge_exceptions | (entity_a, entity_b) |
| actors | actor_id |
| access_keys | code |
| workspace_config | key |
| modules | module_id |
| edge_bindings | binding_id, edge_type |
| field_overrides | (module_id, field_name) |
| custom_shared_keys | key_name |

### NOT NULL Constraints

All primary keys and foreign keys are NOT NULL. Optional fields explicitly allow NULL:
- `entities.deleted_at` (NULL = not deleted)
- `entities.redirect_to` (NULL = not redirected)
- `edges.deleted_at` (NULL = not deleted)
- `facets.detached_at` (NULL = attached)
- `conflicts.resolved_at` (NULL = unresolved)
- `conflicts.reopened_at` (NULL = not reopened)
- `table_memberships.removed_at` (NULL = active membership)
- `table_memberships.source_id` (NULL = no associated rule/link)
- `table_links.removed_at` (NULL = active link)
- `shared_key_suggestions.resolved_by` (NULL = not yet resolved)
- `shared_key_suggestions.resolved_at` (NULL = not yet resolved)
- `shared_key_suggestions.resolved_in_bundle` (NULL = not yet resolved)
- `tables.description` (NULL = no description)
- `scripts.module_id` (NULL = user script)
- `scripts.created_in_bundle` (NULL = module-provided script)
- `triggers.when_condition` (NULL = no additional condition)
- `triggers.scope_type` (NULL = workspace-wide)
- `triggers.scope_value` (NULL = workspace-wide)
- `actors.revoked_at` (NULL = not revoked)
- `actors.revoked_in_bundle` (NULL = not revoked)
- `access_keys.max_uses` (NULL = unlimited)
- `access_keys.revoked_at` (NULL = not revoked)
- `access_keys.revoked_in_bundle` (NULL = not revoked)
- `modules.removed_at` (NULL = active)
- `modules.removed_in_bundle` (NULL = active)
- `modules.config` (NULL = default configuration)
- `edge_bindings.target_tables` (NULL when target_mode = 'any')
- `custom_shared_keys.expected_type` (NULL = any type)
- `custom_shared_keys.description` (NULL = no description)

---

## Performance Considerations

### Bulk Inserts

For importing large datasets:

```sql
-- Temporarily disable synchronous writes
PRAGMA synchronous = OFF;

-- Use multi-row INSERT
INSERT INTO oplog (op_id, actor_id, hlc, ...) VALUES
    (?, ?, ?, ...),
    (?, ?, ?, ...),
    ...;

-- Re-enable after import
PRAGMA synchronous = NORMAL;
```

### Query Patterns

Common access patterns and their indexes:

| Query Pattern | Supporting Index |
|--------------|------------------|
| Canonical op ordering | `idx_oplog_canonical_order` |
| Vector clock delta | `idx_oplog_actor_hlc` |
| Entity history | `idx_oplog_entity` |
| Active entities | `idx_entities_active` |
| Entities in a table | `idx_table_memberships_table` |
| Field by value | `idx_fields_key_value` |
| Outgoing edges | `idx_edges_source` |
| Incoming edges | `idx_edges_target` |

### Index Coverage

Indexes are designed for covering queries where possible:
- Most queries return small result sets
- Avoid table scans for common operations
- Partial indexes reduce index size (e.g., `WHERE deleted_at IS NULL`)

---

## Backup and Recovery

### Backup Strategy

```bash
# Hot backup using SQLite backup API
sqlite3 oplog.db ".backup 'oplog_backup.db'"

# Or copy while database is idle (WAL mode makes this safe)
cp oplog.db oplog_backup.db
cp oplog.db-wal oplog_backup.db-wal
```

### Export Format

For workspace export/transfer, the database can be:
1. Copied directly (SQLite file)
2. Dumped to SQL (for debugging)
3. Streamed as oplog entries (for sync)

### Integrity Checks

```sql
-- Verify database integrity
PRAGMA integrity_check;

-- Verify foreign keys
PRAGMA foreign_key_check;

-- Analyze query plans (development)
PRAGMA optimize;
```

---

## Conflicts

### conflicts Table

Field-level conflicts with audit trail. See [conflicts.md](conflicts.md).

```sql
CREATE TABLE conflicts (
    conflict_id BLOB PRIMARY KEY,
    entity_id BLOB NOT NULL,
    field_key TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',  -- 'open', 'resolved'

    -- Competing values (first two; see conflict_values for N-way)
    value_1 BLOB NOT NULL,
    actor_1 BLOB NOT NULL CHECK (length(actor_1) = 32),
    hlc_1 BLOB NOT NULL CHECK (length(hlc_1) = 12),
    op_id_1 BLOB NOT NULL,

    value_2 BLOB NOT NULL,
    actor_2 BLOB NOT NULL CHECK (length(actor_2) = 32),
    hlc_2 BLOB NOT NULL CHECK (length(hlc_2) = 12),
    op_id_2 BLOB NOT NULL,

    -- Detection
    detected_at BLOB NOT NULL CHECK (length(detected_at) = 12),
    detected_in_bundle BLOB NOT NULL,

    -- Resolution
    resolved_at BLOB CHECK (resolved_at IS NULL OR length(resolved_at) = 12),
    resolved_by BLOB CHECK (resolved_by IS NULL OR length(resolved_by) = 32),
    resolved_value BLOB,
    resolution_op_id BLOB,

    -- Late-arriving reopening
    reopened_at BLOB CHECK (reopened_at IS NULL OR length(reopened_at) = 12),
    reopened_by_op BLOB,

    FOREIGN KEY (entity_id) REFERENCES entities(entity_id),
    FOREIGN KEY (op_id_1) REFERENCES oplog(op_id),
    FOREIGN KEY (op_id_2) REFERENCES oplog(op_id),
    FOREIGN KEY (resolution_op_id) REFERENCES oplog(op_id),
    FOREIGN KEY (detected_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_conflicts_open ON conflicts (entity_id, field_key) WHERE status = 'open';
CREATE INDEX idx_conflicts_entity ON conflicts (entity_id);
```

**Index justifications:**
- `idx_conflicts_open`: Find open conflicts for an entity (conflict badge, resolution UI)
- `idx_conflicts_entity`: Entity conflict history queries

### conflict_values Table

Additional values for N-way conflicts (3+ competing values). Values 1 and 2 are stored inline in the conflicts table.

```sql
CREATE TABLE conflict_values (
    conflict_id BLOB NOT NULL,
    value_index INTEGER NOT NULL,  -- 3, 4, 5... (1 and 2 in conflicts table)
    value BLOB NOT NULL,
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 32),
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),
    op_id BLOB NOT NULL,

    PRIMARY KEY (conflict_id, value_index),
    FOREIGN KEY (conflict_id) REFERENCES conflicts(conflict_id) ON DELETE CASCADE,
    FOREIGN KEY (op_id) REFERENCES oplog(op_id)
);
```

---

## Rules and Automation

### rules Table

Unified rule definitions. See [rules.md](rules.md).

```sql
CREATE TABLE rules (
    rule_id BLOB PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,

    -- Definition
    when_clause TEXT NOT NULL,
    action_type TEXT NOT NULL,  -- 'attach_facet', 'merge_entities', 'set_field', 'detach_facet'
    action_params BLOB NOT NULL,  -- MessagePack

    -- Configuration
    auto_accept INTEGER NOT NULL DEFAULT 0,
    on_condition_lost TEXT DEFAULT 'propose_detach',
    cycle_acknowledged INTEGER NOT NULL DEFAULT 0,

    -- Metadata
    description TEXT,
    module_id TEXT,
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL,

    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_rules_action ON rules (action_type);
```

**Index justifications:**
- `idx_rules_action`: Filter rules by action type

### scripts Table

Script source and configuration. See [scripts.md](scripts.md).

```sql
CREATE TABLE scripts (
    script_id BLOB PRIMARY KEY,
    name TEXT NOT NULL,

    -- Source
    script_type TEXT NOT NULL,  -- 'user', 'module'
    module_id TEXT,
    source_code TEXT NOT NULL,
    source_hash BLOB NOT NULL,

    -- Configuration
    execution_mode TEXT NOT NULL DEFAULT 'session',
    on_error TEXT NOT NULL DEFAULT 'skip',
    isolation TEXT NOT NULL DEFAULT 'snapshot',

    -- Capabilities (MessagePack array)
    capabilities BLOB,

    -- Metadata
    description TEXT,
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB,

    UNIQUE (script_type, module_id, name),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_scripts_type ON scripts (script_type);
CREATE INDEX idx_scripts_module ON scripts (module_id) WHERE module_id IS NOT NULL;
```

**Index justifications:**
- `idx_scripts_type`: Filter scripts by type (user vs module)
- `idx_scripts_module`: Find scripts from a specific module

### triggers Table

Automatic script execution triggers. See [scripts.md](scripts.md).

```sql
CREATE TABLE triggers (
    trigger_id BLOB PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,

    -- Definition
    on_field_change TEXT NOT NULL,
    when_condition TEXT,
    scope_type TEXT,  -- 'table', 'entity', 'workspace'
    scope_value TEXT,

    -- Script reference
    script_id BLOB NOT NULL,
    script_params BLOB,
    execution_mode_override TEXT,

    -- State
    enabled INTEGER NOT NULL DEFAULT 1,

    -- Metadata
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL,

    FOREIGN KEY (script_id) REFERENCES scripts(script_id),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_triggers_field ON triggers (on_field_change);
CREATE INDEX idx_triggers_script ON triggers (script_id);
CREATE INDEX idx_triggers_enabled ON triggers (enabled) WHERE enabled = 1;
```

**Index justifications:**
- `idx_triggers_field`: Find triggers watching a specific field
- `idx_triggers_script`: Find triggers using a script (cascade disable)
- `idx_triggers_enabled`: Efficient scan of active triggers only

---

## Identity Repair

### merge_resolutions Table

Identity repair merge tracking. See [data-model.md](data-model.md).

```sql
CREATE TABLE merge_resolutions (
    absorbed_id BLOB PRIMARY KEY,
    survivor_id BLOB NOT NULL,
    merge_operation_id BLOB NOT NULL,
    merge_actor BLOB NOT NULL CHECK (length(merge_actor) = 32),
    merged_at BLOB NOT NULL CHECK (length(merged_at) = 12),
    merged_in_bundle BLOB NOT NULL,

    FOREIGN KEY (survivor_id) REFERENCES entities(entity_id),
    FOREIGN KEY (merge_operation_id) REFERENCES oplog(op_id),
    FOREIGN KEY (merged_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_merge_resolutions_survivor ON merge_resolutions (survivor_id);
```

**Index justifications:**
- `idx_merge_resolutions_survivor`: Find all entities absorbed into a survivor

### merge_exceptions Table

Prevent re-merging of explicitly split entities.

```sql
CREATE TABLE merge_exceptions (
    entity_a BLOB NOT NULL,
    entity_b BLOB NOT NULL,
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    created_in_bundle BLOB NOT NULL,
    reason TEXT,

    PRIMARY KEY (entity_a, entity_b),
    FOREIGN KEY (entity_a) REFERENCES entities(entity_id),
    FOREIGN KEY (entity_b) REFERENCES entities(entity_id),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id)
);
```

---

## Identity and Permissions

### actors Table

Known actors and their metadata.

```sql
CREATE TABLE actors (
    actor_id BLOB PRIMARY KEY CHECK (length(actor_id) = 32),  -- Ed25519 public key (32 bytes)
    display_name TEXT,
    device_name TEXT,

    first_seen_at BLOB NOT NULL,
    first_seen_in_bundle BLOB NOT NULL,

    revoked_at BLOB,
    revoked_by BLOB CHECK (revoked_by IS NULL OR length(revoked_by) = 32),
    revoked_in_bundle BLOB,

    FOREIGN KEY (first_seen_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (revoked_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_actors_revoked ON actors (revoked_at) WHERE revoked_at IS NOT NULL;
```

**Index justifications:**
- `idx_actors_revoked`: Find revoked actors for access control

### role_assignments Table (Deferred to Post-v1)

> **Note:** Role-based access control is deferred to post-v1. For v1, all actors who join a workspace have full read/write access (everyone is effectively an editor). The `role_assignments` and `custom_roles` tables will be introduced when permissions are implemented.

<!--
```sql
CREATE TABLE role_assignments (
    assignment_id BLOB PRIMARY KEY,
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 32),
    role_name TEXT NOT NULL,  -- 'Viewer', 'Proposer', 'Editor', 'Admin', or custom

    -- Scope
    scope_type TEXT NOT NULL DEFAULT 'global',
    scope_value TEXT,

    -- Timeline
    assigned_at BLOB NOT NULL,
    assigned_by BLOB NOT NULL CHECK (length(assigned_by) = 32),
    assigned_in_bundle BLOB NOT NULL,

    revoked_at BLOB,
    revoked_by BLOB CHECK (revoked_by IS NULL OR length(revoked_by) = 32),
    revoked_in_bundle BLOB,

    FOREIGN KEY (actor_id) REFERENCES actors(actor_id),
    FOREIGN KEY (assigned_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (revoked_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_role_assignments_actor ON role_assignments (actor_id) WHERE revoked_at IS NULL;
```
-->

### custom_roles Table (Deferred to Post-v1)

<!--
```sql
CREATE TABLE custom_roles (
    role_id BLOB PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    permissions BLOB NOT NULL,  -- MessagePack map

    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_at BLOB NOT NULL,
    created_in_bundle BLOB NOT NULL,

    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id)
);
```
-->

---

## Workspace Configuration

### access_keys Table

Workspace join codes.

```sql
CREATE TABLE access_keys (
    code TEXT PRIMARY KEY,
    granted_role TEXT NOT NULL,

    expires_at BLOB NOT NULL,
    max_uses INTEGER,
    uses_count INTEGER NOT NULL DEFAULT 0,

    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_at BLOB NOT NULL,
    created_in_bundle BLOB NOT NULL,

    revoked_at BLOB,
    revoked_by BLOB CHECK (revoked_by IS NULL OR length(revoked_by) = 32),
    revoked_in_bundle BLOB,

    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (revoked_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_access_keys_active ON access_keys (expires_at) WHERE revoked_at IS NULL;
```

**Index justifications:**
- `idx_access_keys_active`: Find non-revoked keys for validation and expiry checks

### workspace_config Table

Workspace-level settings.

```sql
CREATE TABLE workspace_config (
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL,  -- MessagePack encoded

    updated_at BLOB NOT NULL,
    updated_by BLOB NOT NULL CHECK (length(updated_by) = 32),
    updated_in_bundle BLOB NOT NULL,

    FOREIGN KEY (updated_in_bundle) REFERENCES bundles(bundle_id)
);
```

### modules Table

Adopted modules in the workspace. See [modules.md](modules.md).

```sql
CREATE TABLE modules (
    module_id TEXT PRIMARY KEY,           -- Module identifier (e.g., "contacts", "lighting")
    version TEXT NOT NULL,                -- Semver version at adoption

    -- Adoption state
    status TEXT NOT NULL DEFAULT 'active',  -- 'active', 'disabled', 'removed'

    -- Adoption metadata
    adopted_at BLOB NOT NULL,
    adopted_by BLOB NOT NULL CHECK (length(adopted_by) = 32),
    adopted_in_bundle BLOB NOT NULL,

    -- Removal tracking
    removed_at BLOB,
    removed_by BLOB CHECK (removed_by IS NULL OR length(removed_by) = 32),
    removed_in_bundle BLOB,

    -- Module configuration (MessagePack map)
    config BLOB,

    FOREIGN KEY (adopted_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (removed_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_modules_status ON modules (status) WHERE status = 'active';
```

**Index justifications:**
- `idx_modules_status`: Find active modules for schema resolution

### edge_bindings Table

User-configured cross-module edge constraints. See [data-model.md](data-model.md). Uses table-based constraints instead of kind-based constraints.

```sql
CREATE TABLE edge_bindings (
    binding_id BLOB PRIMARY KEY,
    edge_type TEXT NOT NULL,              -- Namespaced edge type (e.g., "casting.assigned_to", module.edge_type)

    -- Source constraint (table-based, fixed by module)
    source_table BLOB NOT NULL,           -- Table ID that sources must belong to

    -- Target constraint (user-configured)
    target_mode TEXT NOT NULL,            -- 'tables', 'any'
    target_tables BLOB,                   -- MessagePack array of table IDs (if mode = 'tables')

    -- Metadata
    created_at BLOB NOT NULL,
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL,

    UNIQUE (edge_type),
    FOREIGN KEY (source_table) REFERENCES tables(table_id),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_edge_bindings_type ON edge_bindings (edge_type);
CREATE INDEX idx_edge_bindings_source_table ON edge_bindings (source_table);
```

**Index justifications:**
- `idx_edge_bindings_type`: Lookup binding for an edge type during validation
- `idx_edge_bindings_source_table`: Find edge bindings constrained to a specific source table

### field_overrides Table

User overrides for module field → shared key mappings. See [data-model.md](data-model.md).

```sql
CREATE TABLE field_overrides (
    module_id TEXT NOT NULL,
    field_name TEXT NOT NULL,

    -- Override target
    shared_key TEXT NOT NULL,             -- The shared key this field should map to

    -- Metadata
    created_at BLOB NOT NULL,
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL,

    PRIMARY KEY (module_id, field_name),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id)
);

CREATE INDEX idx_field_overrides_shared_key ON field_overrides (shared_key);
```

**Index justifications:**
- `idx_field_overrides_shared_key`: Find all fields that map to a shared key

### custom_shared_keys Table

User-defined shared keys. See [data-model.md](data-model.md).

```sql
CREATE TABLE custom_shared_keys (
    key_name TEXT PRIMARY KEY,

    -- Type hint for validation
    expected_type TEXT,                   -- 'string', 'number', 'boolean', 'timestamp', etc.

    -- Documentation
    description TEXT,

    -- Metadata
    created_at BLOB NOT NULL,
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL,

    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id)
);
```

---

## Additional Local-Only Tables

The following tables extend the local-only tables section above. They are never synced to peers.

### awareness_events Table

Ephemeral UX notifications for concurrent edits. Cleared on restart. See [conflicts.md](conflicts.md).

```sql
CREATE TABLE awareness_events (
    awareness_id BLOB PRIMARY KEY,
    entity_id BLOB NOT NULL,
    actor_1 BLOB NOT NULL,
    actor_2 BLOB NOT NULL,
    field_key_1 TEXT NOT NULL,
    field_key_2 TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX idx_awareness_expiry ON awareness_events (expires_at);
```

**Index justifications:**
- `idx_awareness_expiry`: Efficient cleanup of expired events

### rule_dependencies Table

Pre-computed rule dependency graph for O(1) cycle detection. See [rules.md](rules.md).

```sql
CREATE TABLE rule_dependencies (
    from_rule_id BLOB NOT NULL,
    to_rule_id BLOB NOT NULL,
    trigger_field TEXT NOT NULL,
    computed_at INTEGER NOT NULL,

    PRIMARY KEY (from_rule_id, to_rule_id)
);

CREATE INDEX idx_rule_deps_to ON rule_dependencies (to_rule_id);
```

**Index justifications:**
- `idx_rule_deps_to`: Find rules that depend on a given rule

### trigger_dependencies Table

Pre-computed trigger dependency graph. See [scripts.md](scripts.md).

```sql
CREATE TABLE trigger_dependencies (
    from_trigger_id BLOB NOT NULL,
    to_trigger_id BLOB NOT NULL,
    trigger_field TEXT NOT NULL,
    computed_at INTEGER NOT NULL,

    PRIMARY KEY (from_trigger_id, to_trigger_id)
);
```

### rule_execution_state Table

Per-entity rule application tracking. See [rules.md](rules.md).

```sql
CREATE TABLE rule_execution_state (
    entity_id BLOB NOT NULL,
    rule_id BLOB NOT NULL,
    last_applied_at BLOB,
    last_applied_op BLOB,
    is_applicable INTEGER,

    PRIMARY KEY (entity_id, rule_id)
);

CREATE INDEX idx_rule_exec_rule ON rule_execution_state (rule_id);
```

**Index justifications:**
- `idx_rule_exec_rule`: Find all entities affected by a rule (impact analysis)

### script_executions Table

Script execution audit log. See [scripts.md](scripts.md).

```sql
CREATE TABLE script_executions (
    execution_id BLOB PRIMARY KEY,
    script_id BLOB NOT NULL,
    trigger_id BLOB,

    executed_by BLOB NOT NULL,
    executed_at INTEGER NOT NULL,

    status TEXT NOT NULL,
    duration_ms INTEGER,
    operations_count INTEGER,
    error_message TEXT,

    overlay_id BLOB
);

CREATE INDEX idx_script_exec_script ON script_executions (script_id, executed_at);
CREATE INDEX idx_script_exec_status ON script_executions (status) WHERE status = 'error';
```

**Index justifications:**
- `idx_script_exec_script`: Script execution history
- `idx_script_exec_status`: Find failed executions for debugging

### script_capabilities Table

Capability grants for scripts. See [scripts.md](scripts.md).

```sql
CREATE TABLE script_capabilities (
    script_id BLOB NOT NULL,
    capability TEXT NOT NULL,

    approval_status TEXT NOT NULL DEFAULT 'pending',
    approved_by BLOB,
    approved_at INTEGER,

    PRIMARY KEY (script_id, capability)
);

CREATE INDEX idx_script_caps_pending ON script_capabilities (approval_status) WHERE approval_status = 'pending';
```

**Index justifications:**
- `idx_script_caps_pending`: Find unapproved capabilities for security review

### leader_state Table

Leader election state. See [sync.md](sync.md).

```sql
CREATE TABLE leader_state (
    workspace_id BLOB PRIMARY KEY,
    leader_id BLOB,
    leader_epoch INTEGER,
    last_heartbeat_at INTEGER,
    current_sequence INTEGER DEFAULT 0
);
```

### join_requests Table

Pending workspace join requests.

```sql
CREATE TABLE join_requests (
    request_id BLOB PRIMARY KEY,
    actor_id BLOB NOT NULL,
    display_name TEXT NOT NULL,

    status TEXT NOT NULL DEFAULT 'pending',
    requested_at INTEGER NOT NULL,

    reviewed_by BLOB,
    reviewed_at INTEGER,
    granted_role TEXT
);

CREATE INDEX idx_join_requests_status ON join_requests (status);
```

**Index justifications:**
- `idx_join_requests_status`: Find pending requests for admin review

---

## Open Questions

- Sharding strategy for very large workspaces (millions of entities)
- Archiving old operations (keep oplog bounded?)
- Full-text search integration (FTS5 for field values?)
- Geospatial indexes (R-tree for location-aware modules?)
- Encryption at rest (SQLCipher integration?)
- Connection pooling strategy for multi-threaded access
