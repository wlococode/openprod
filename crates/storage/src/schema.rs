use rusqlite::Connection;

use crate::error::StorageError;

pub const SCHEMA_VERSION: i32 = 2;

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
INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (2, unixepoch());

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
CREATE INDEX IF NOT EXISTS idx_oplog_canonical_order ON oplog (hlc, op_id);
CREATE INDEX IF NOT EXISTS idx_oplog_actor_hlc ON oplog (actor_id, hlc);
CREATE INDEX IF NOT EXISTS idx_oplog_entity ON oplog (entity_id, hlc) WHERE entity_id IS NOT NULL;
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
    creator_vector_clock BLOB,
    received_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('now','subsec') * 1000 AS INTEGER))
);
CREATE INDEX IF NOT EXISTS idx_bundles_hlc ON bundles (hlc);
CREATE INDEX IF NOT EXISTS idx_bundles_actor ON bundles (actor_id, hlc);
CREATE INDEX IF NOT EXISTS idx_bundles_type ON bundles (bundle_type, hlc);

CREATE TABLE IF NOT EXISTS entities (
    entity_id BLOB PRIMARY KEY CHECK (length(entity_id) = 16),
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL CHECK (length(created_in_bundle) = 16),
    deleted_at BLOB CHECK (deleted_at IS NULL OR length(deleted_at) = 12),
    deleted_by BLOB CHECK (deleted_by IS NULL OR length(deleted_by) = 32),
    deleted_in_bundle BLOB,
    redirect_to BLOB,
    redirect_at BLOB CHECK (redirect_at IS NULL OR length(redirect_at) = 12),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (deleted_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (redirect_to) REFERENCES entities(entity_id)
);
CREATE INDEX IF NOT EXISTS idx_entities_active ON entities (created_at) WHERE deleted_at IS NULL AND redirect_to IS NULL;
CREATE INDEX IF NOT EXISTS idx_entities_deleted ON entities (deleted_at) WHERE deleted_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_entities_redirects ON entities (redirect_to) WHERE redirect_to IS NOT NULL;

CREATE TABLE IF NOT EXISTS fields (
    entity_id BLOB NOT NULL CHECK (length(entity_id) = 16),
    field_key TEXT NOT NULL,
    value BLOB,
    source_op BLOB NOT NULL CHECK (length(source_op) = 16),
    source_actor BLOB NOT NULL CHECK (length(source_actor) = 32),
    updated_at BLOB NOT NULL CHECK (length(updated_at) = 12),
    PRIMARY KEY (entity_id, field_key),
    FOREIGN KEY (entity_id) REFERENCES entities(entity_id)
);
CREATE INDEX IF NOT EXISTS idx_fields_key_value ON fields (field_key, value);
CREATE INDEX IF NOT EXISTS idx_fields_source_op ON fields (source_op);

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
    PRIMARY KEY (entity_id, facet_type),
    FOREIGN KEY (entity_id) REFERENCES entities(entity_id),
    FOREIGN KEY (attached_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (detached_in_bundle) REFERENCES bundles(bundle_id)
);
CREATE INDEX IF NOT EXISTS idx_facets_type ON facets (facet_type) WHERE detached_at IS NULL;

CREATE TABLE IF NOT EXISTS edges (
    edge_id BLOB PRIMARY KEY CHECK (length(edge_id) = 16),
    edge_type TEXT NOT NULL,
    source_id BLOB NOT NULL CHECK (length(source_id) = 16),
    target_id BLOB NOT NULL CHECK (length(target_id) = 16),
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL CHECK (length(created_in_bundle) = 16),
    deleted_at BLOB CHECK (deleted_at IS NULL OR length(deleted_at) = 12),
    deleted_by BLOB CHECK (deleted_by IS NULL OR length(deleted_by) = 32),
    deleted_in_bundle BLOB,
    FOREIGN KEY (source_id) REFERENCES entities(entity_id),
    FOREIGN KEY (target_id) REFERENCES entities(entity_id),
    FOREIGN KEY (created_in_bundle) REFERENCES bundles(bundle_id),
    FOREIGN KEY (deleted_in_bundle) REFERENCES bundles(bundle_id)
);
CREATE INDEX IF NOT EXISTS idx_edges_source ON edges (source_id, edge_type) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_edges_target ON edges (target_id, edge_type) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_edges_type ON edges (edge_type) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_edges_deleted ON edges (deleted_in_bundle) WHERE deleted_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS edge_properties (
    edge_id BLOB NOT NULL CHECK (length(edge_id) = 16),
    property_key TEXT NOT NULL,
    value BLOB,
    source_op BLOB NOT NULL CHECK (length(source_op) = 16),
    source_actor BLOB NOT NULL CHECK (length(source_actor) = 32),
    updated_at BLOB NOT NULL CHECK (length(updated_at) = 12),
    PRIMARY KEY (edge_id, property_key),
    FOREIGN KEY (edge_id) REFERENCES edges(edge_id)
);
CREATE INDEX IF NOT EXISTS idx_edge_properties_source_op ON edge_properties (source_op);

CREATE TABLE IF NOT EXISTS actors (
    actor_id BLOB PRIMARY KEY CHECK (length(actor_id) = 32),
    display_name TEXT,
    first_seen_at BLOB NOT NULL CHECK (length(first_seen_at) = 12)
);

CREATE TABLE IF NOT EXISTS vector_clock (
    actor_id BLOB PRIMARY KEY CHECK (length(actor_id) = 32),
    max_hlc BLOB NOT NULL CHECK (length(max_hlc) = 12)
);

CREATE TABLE IF NOT EXISTS conflicts (
    conflict_id BLOB PRIMARY KEY CHECK (length(conflict_id) = 16),
    entity_id BLOB NOT NULL CHECK (length(entity_id) = 16),
    field_key TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'resolved')),
    detected_at BLOB NOT NULL CHECK (length(detected_at) = 12),
    detected_in_bundle BLOB NOT NULL CHECK (length(detected_in_bundle) = 16),
    resolved_at BLOB CHECK (resolved_at IS NULL OR length(resolved_at) = 12),
    resolved_by BLOB CHECK (resolved_by IS NULL OR length(resolved_by) = 32),
    resolved_op_id BLOB CHECK (resolved_op_id IS NULL OR length(resolved_op_id) = 16),
    resolved_value BLOB,
    reopened_at BLOB CHECK (reopened_at IS NULL OR length(reopened_at) = 12),
    reopened_by_op BLOB CHECK (reopened_by_op IS NULL OR length(reopened_by_op) = 16),
    FOREIGN KEY (entity_id) REFERENCES entities(entity_id),
    FOREIGN KEY (detected_in_bundle) REFERENCES bundles(bundle_id)
);
CREATE INDEX IF NOT EXISTS idx_conflicts_entity ON conflicts (entity_id, field_key) WHERE status = 'open';
CREATE INDEX IF NOT EXISTS idx_conflicts_status ON conflicts (status);

CREATE TABLE IF NOT EXISTS conflict_values (
    conflict_id BLOB NOT NULL CHECK (length(conflict_id) = 16),
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 32),
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),
    op_id BLOB NOT NULL CHECK (length(op_id) = 16),
    value BLOB,
    PRIMARY KEY (conflict_id, actor_id),
    FOREIGN KEY (conflict_id) REFERENCES conflicts(conflict_id)
);

CREATE TABLE IF NOT EXISTS overlays (
    overlay_id BLOB PRIMARY KEY CHECK (length(overlay_id) = 16),
    display_name TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'user' CHECK (source IN ('user', 'script')),
    source_id TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'stashed', 'committed', 'discarded')),
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    updated_at BLOB NOT NULL CHECK (length(updated_at) = 12),
    script_id TEXT,
    script_execution_id TEXT,
    meta BLOB
);
CREATE INDEX IF NOT EXISTS idx_overlays_status ON overlays (status);

CREATE TABLE IF NOT EXISTS overlay_ops (
    rowid INTEGER PRIMARY KEY,
    overlay_id BLOB NOT NULL CHECK (length(overlay_id) = 16),
    op_id BLOB NOT NULL CHECK (length(op_id) = 16),
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),
    payload BLOB NOT NULL,
    entity_id BLOB CHECK (entity_id IS NULL OR length(entity_id) = 16),
    field_key TEXT,
    op_type TEXT NOT NULL,
    canonical_value_at_creation BLOB,
    canonical_drifted INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (overlay_id) REFERENCES overlays(overlay_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_overlay_ops_overlay ON overlay_ops (overlay_id);
CREATE INDEX IF NOT EXISTS idx_overlay_ops_entity ON overlay_ops (overlay_id, entity_id, field_key);
";
