use rusqlite::Connection;

use crate::error::StorageError;

pub const SCHEMA_VERSION: i32 = 1;

pub fn init_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA foreign_keys = ON;
        PRAGMA cache_size = -32000;
        PRAGMA mmap_size = 268435456;
        PRAGMA busy_timeout = 5000;
    ",
    )?;
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
);
INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (1, unixepoch());

CREATE TABLE IF NOT EXISTS oplog (
    rowid INTEGER PRIMARY KEY,
    op_id BLOB NOT NULL UNIQUE CHECK (length(op_id) = 16),
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 32),
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),
    bundle_id BLOB NOT NULL CHECK (length(bundle_id) = 16),
    payload BLOB NOT NULL,
    module_versions BLOB NOT NULL,
    signature BLOB NOT NULL CHECK (length(signature) = 64),
    op_type TEXT NOT NULL,
    entity_id BLOB,
    received_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('now','subsec') * 1000 AS INTEGER))
);
CREATE INDEX IF NOT EXISTS idx_oplog_canonical ON oplog (hlc, op_id);
CREATE INDEX IF NOT EXISTS idx_oplog_actor_hlc ON oplog (actor_id, hlc);
CREATE INDEX IF NOT EXISTS idx_oplog_entity ON oplog (entity_id, hlc);
CREATE INDEX IF NOT EXISTS idx_oplog_bundle ON oplog (bundle_id);

CREATE TABLE IF NOT EXISTS bundles (
    bundle_id BLOB PRIMARY KEY CHECK (length(bundle_id) = 16),
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 32),
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),
    bundle_type INTEGER NOT NULL,
    op_count INTEGER NOT NULL,
    checksum BLOB NOT NULL CHECK (length(checksum) = 32),
    creates BLOB,
    deletes BLOB,
    meta BLOB,
    signature BLOB NOT NULL CHECK (length(signature) = 64),
    received_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('now','subsec') * 1000 AS INTEGER))
);

CREATE TABLE IF NOT EXISTS entities (
    entity_id BLOB PRIMARY KEY CHECK (length(entity_id) = 16),
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL CHECK (length(created_in_bundle) = 16),
    deleted_at BLOB CHECK (deleted_at IS NULL OR length(deleted_at) = 12),
    deleted_by BLOB CHECK (deleted_by IS NULL OR length(deleted_by) = 32),
    deleted_in_bundle BLOB,
    redirect_to BLOB,
    redirect_at BLOB CHECK (redirect_at IS NULL OR length(redirect_at) = 12)
);

CREATE TABLE IF NOT EXISTS fields (
    entity_id BLOB NOT NULL CHECK (length(entity_id) = 16),
    field_key TEXT NOT NULL,
    value BLOB NOT NULL,
    source_op BLOB NOT NULL CHECK (length(source_op) = 16),
    source_actor BLOB NOT NULL CHECK (length(source_actor) = 32),
    updated_at BLOB NOT NULL CHECK (length(updated_at) = 12),
    PRIMARY KEY (entity_id, field_key)
);

CREATE TABLE IF NOT EXISTS facets (
    entity_id BLOB NOT NULL CHECK (length(entity_id) = 16),
    facet_type TEXT NOT NULL,
    attached_at BLOB NOT NULL CHECK (length(attached_at) = 12),
    attached_by BLOB NOT NULL CHECK (length(attached_by) = 32),
    attached_in_bundle BLOB NOT NULL CHECK (length(attached_in_bundle) = 16),
    source_type TEXT NOT NULL DEFAULT 'user',
    detached_at BLOB CHECK (detached_at IS NULL OR length(detached_at) = 12),
    detached_by BLOB CHECK (detached_by IS NULL OR length(detached_by) = 32),
    detached_in_bundle BLOB,
    preserve_values BLOB,
    PRIMARY KEY (entity_id, facet_type)
);

CREATE TABLE IF NOT EXISTS edges (
    edge_id BLOB PRIMARY KEY CHECK (length(edge_id) = 16),
    edge_type TEXT NOT NULL,
    source_id BLOB NOT NULL CHECK (length(source_id) = 16),
    target_id BLOB NOT NULL CHECK (length(target_id) = 16),
    properties BLOB,
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL CHECK (length(created_in_bundle) = 16),
    deleted_at BLOB CHECK (deleted_at IS NULL OR length(deleted_at) = 12),
    deleted_by BLOB CHECK (deleted_by IS NULL OR length(deleted_by) = 32),
    deleted_in_bundle BLOB
);
CREATE INDEX IF NOT EXISTS idx_edges_source ON edges (source_id, edge_type) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_edges_target ON edges (target_id, edge_type) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS actors (
    actor_id BLOB PRIMARY KEY CHECK (length(actor_id) = 32),
    display_name TEXT,
    first_seen_at BLOB NOT NULL CHECK (length(first_seen_at) = 12)
);

CREATE TABLE IF NOT EXISTS vector_clock (
    actor_id BLOB PRIMARY KEY CHECK (length(actor_id) = 32),
    max_hlc BLOB NOT NULL CHECK (length(max_hlc) = 12)
);
";
