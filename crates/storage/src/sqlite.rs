use std::collections::BTreeMap;

use rusqlite::Connection;

use openprod_core::{
    field_value::FieldValue,
    hlc::Hlc,
    ids::*,
    operations::{Bundle, BundleType, Operation, OperationPayload},
    vector_clock::VectorClock,
};

use crate::error::StorageError;
use crate::traits::{ConflictRecord, ConflictStatus, ConflictValue, EdgeRecord, EntityRecord, FacetRecord, Storage};

/// Convert Vec<u8> to fixed-size array with proper error handling.
fn to_array<const N: usize>(v: Vec<u8>, label: &str) -> Result<[u8; N], StorageError> {
    v.try_into()
        .map_err(|_| StorageError::Serialization(format!("invalid {label} length")))
}

type RawEdgeRow = (Vec<u8>, String, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, bool);

fn extract_edge_row(row: &rusqlite::Row) -> rusqlite::Result<RawEdgeRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

fn parse_edge_row(raw: RawEdgeRow) -> Result<EdgeRecord, StorageError> {
    let (edge_id_bytes, edge_type, source_id_bytes, target_id_bytes, created_at_bytes, created_by_bytes, deleted) = raw;
    Ok(EdgeRecord {
        edge_id: EdgeId::from_bytes(to_array::<16>(edge_id_bytes, "edge_id")?),
        edge_type,
        source_id: EntityId::from_bytes(to_array::<16>(source_id_bytes, "source_id")?),
        target_id: EntityId::from_bytes(to_array::<16>(target_id_bytes, "target_id")?),
        created_at: Hlc::from_bytes(&to_array::<12>(created_at_bytes, "created_at")?),
        created_by: ActorId::from_bytes(to_array::<32>(created_by_bytes, "created_by")?),
        deleted,
    })
}

pub struct SqliteStorage {
    conn: Connection,
}

impl SqliteStorage {
    pub fn open(path: &str) -> Result<Self, StorageError> {
        let conn = Connection::open(path)?;
        crate::schema::init_schema(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        crate::schema::init_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Get the source actor, HLC, op_id, and the creator vector clock of the bundle
    /// that last wrote a particular field. Used for conflict detection.
    #[allow(clippy::type_complexity)]
    pub fn get_field_source_bundle_vc(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<(ActorId, Hlc, OpId, Option<VectorClock>)>, StorageError> {
        let result = self.conn.query_row(
            "SELECT f.source_actor, f.updated_at, f.source_op, b.creator_vector_clock
             FROM fields f
             JOIN oplog o ON o.op_id = f.source_op
             JOIN bundles b ON b.bundle_id = o.bundle_id
             WHERE f.entity_id = ?1 AND f.field_key = ?2",
            rusqlite::params![entity_id.as_bytes().as_slice(), field_key],
            |row| {
                let actor_bytes: Vec<u8> = row.get(0)?;
                let hlc_bytes: Vec<u8> = row.get(1)?;
                let op_id_bytes: Vec<u8> = row.get(2)?;
                let vc_bytes: Option<Vec<u8>> = row.get(3)?;
                Ok((actor_bytes, hlc_bytes, op_id_bytes, vc_bytes))
            },
        );
        match result {
            Ok((actor_bytes, hlc_bytes, op_id_bytes, vc_bytes)) => {
                let actor = ActorId::from_bytes(to_array::<32>(actor_bytes, "source_actor")?);
                let hlc = Hlc::from_bytes(&to_array::<12>(hlc_bytes, "updated_at")?);
                let op_id = OpId::from_bytes(to_array::<16>(op_id_bytes, "source_op")?);
                let vc = match vc_bytes {
                    Some(bytes) => Some(VectorClock::from_msgpack(&bytes)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?),
                    None => None,
                };
                Ok(Some((actor, hlc, op_id, vc)))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    /// Expose the connection for use in transactions from Engine.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Get the field value bytes from an oplog operation by op_id.
    /// Returns Some(bytes) for SetField/ResolveConflict with value, None for ClearField/tombstone.
    pub fn get_op_field_value(&self, op_id: OpId) -> Result<Option<Vec<u8>>, StorageError> {
        let result = self.conn.query_row(
            "SELECT payload FROM oplog WHERE op_id = ?1",
            rusqlite::params![op_id.as_bytes().as_slice()],
            |row| {
                let payload_bytes: Vec<u8> = row.get(0)?;
                Ok(payload_bytes)
            },
        );
        match result {
            Ok(payload_bytes) => {
                let payload = OperationPayload::from_msgpack(&payload_bytes)?;
                match payload {
                    OperationPayload::SetField { value, .. } => {
                        let bytes = value.to_msgpack()
                            .map_err(|e| StorageError::Serialization(e.to_string()))?;
                        Ok(Some(bytes))
                    }
                    OperationPayload::ClearField { .. } => Ok(None),
                    OperationPayload::ResolveConflict { chosen_value: Some(v), .. } => {
                        let bytes = v.to_msgpack()
                            .map_err(|e| StorageError::Serialization(e.to_string()))?;
                        Ok(Some(bytes))
                    }
                    OperationPayload::ResolveConflict { chosen_value: None, .. } => Ok(None),
                    _ => Ok(None),
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }
}

impl SqliteStorage {
    pub fn rebuild_from_oplog(&mut self) -> Result<u64, StorageError> {
        self.conn.execute_batch("SAVEPOINT sp_rebuild")?;

        let result = (|| -> Result<u64, StorageError> {
            // Clear all materialized tables (children before parents to respect FK constraints)
            self.conn.execute_batch(
                "DELETE FROM conflict_values;
                 DELETE FROM conflicts;
                 DELETE FROM edge_properties;
                 DELETE FROM fields;
                 DELETE FROM facets;
                 DELETE FROM edges;
                 DELETE FROM entities;
                 DELETE FROM actors;
                 DELETE FROM vector_clock;",
            )?;

            // Read all ops in canonical order
            let mut op_stmt = self.conn.prepare(
                "SELECT op_id, actor_id, hlc, bundle_id, payload, module_versions, signature FROM oplog ORDER BY hlc, op_id",
            )?;
            let ops: Vec<Operation> = op_stmt
                .query_map([], |row| {
                    read_op(row).map_err(|e| match e {
                        StorageError::Sqlite(sq) => sq,
                        other => rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Blob,
                            Box::new(OpaqueStorageError(other.to_string())),
                        ),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(op_stmt);

            let op_count = ops.len() as u64;

            // Group ops by bundle_id and replay
            // We need bundle info for materialization, so read bundles
            let mut bundle_cache: std::collections::HashMap<[u8; 16], Bundle> =
                std::collections::HashMap::new();

            for op in &ops {
                let bundle_key = *op.bundle_id.as_bytes();
                if let std::collections::hash_map::Entry::Vacant(e) = bundle_cache.entry(bundle_key) {
                    let bundle = read_bundle(&self.conn, op.bundle_id)?;
                    e.insert(bundle);
                }
                let bundle = &bundle_cache[&bundle_key];

                materialize_op(&self.conn, op, bundle)?;

                // Track actor
                self.conn.execute(
                    "INSERT OR IGNORE INTO actors (actor_id, display_name, first_seen_at) VALUES (?1, NULL, ?2)",
                    rusqlite::params![
                        op.actor_id.as_bytes().as_slice(),
                        &op.hlc.to_bytes()[..],
                    ],
                )?;

                // Update vector clock
                self.conn.execute(
                    "INSERT INTO vector_clock (actor_id, max_hlc) VALUES (?1, ?2)
                     ON CONFLICT(actor_id) DO UPDATE SET max_hlc = excluded.max_hlc
                     WHERE excluded.max_hlc > vector_clock.max_hlc",
                    rusqlite::params![
                        op.actor_id.as_bytes().as_slice(),
                        &op.hlc.to_bytes()[..],
                    ],
                )?;
            }

            Ok(op_count)
        })();

        match result {
            Ok(count) => {
                self.conn.execute_batch("RELEASE sp_rebuild")?;
                Ok(count)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK TO sp_rebuild; RELEASE sp_rebuild");
                Err(e)
            }
        }
    }
}

fn read_op(row: &rusqlite::Row) -> Result<Operation, StorageError> {
    let op_id_bytes: Vec<u8> = row.get(0)?;
    let actor_id_bytes: Vec<u8> = row.get(1)?;
    let hlc_bytes: Vec<u8> = row.get(2)?;
    let bundle_id_bytes: Vec<u8> = row.get(3)?;
    let payload_bytes: Vec<u8> = row.get(4)?;
    let module_versions_bytes: Vec<u8> = row.get(5)?;
    let signature_bytes: Vec<u8> = row.get(6)?;

    let op_id = OpId::from_bytes(to_array::<16>(op_id_bytes, "op_id")?);
    let actor_id = ActorId::from_bytes(to_array::<32>(actor_id_bytes, "actor_id")?);
    let hlc = Hlc::from_bytes(&to_array::<12>(hlc_bytes, "hlc")?);
    let bundle_id = BundleId::from_bytes(to_array::<16>(bundle_id_bytes, "bundle_id")?);
    let payload = OperationPayload::from_msgpack(&payload_bytes)?;
    let module_versions: BTreeMap<String, String> = rmp_serde::from_slice(&module_versions_bytes)
        .map_err(|e| StorageError::Serialization(e.to_string()))?;
    let signature = Signature::from_bytes(to_array::<64>(signature_bytes, "signature")?);

    Ok(Operation {
        op_id,
        actor_id,
        hlc,
        bundle_id,
        module_versions,
        payload,
        signature,
    })
}

fn read_bundle(conn: &Connection, bundle_id: BundleId) -> Result<Bundle, StorageError> {
    conn.query_row(
        "SELECT bundle_id, actor_id, hlc, bundle_type, op_count, checksum, creates, deletes, meta, signature, creator_vector_clock FROM bundles WHERE bundle_id = ?1",
        rusqlite::params![bundle_id.as_bytes().as_slice()],
        |row| {
            let bundle_id_bytes: Vec<u8> = row.get(0)?;
            let actor_id_bytes: Vec<u8> = row.get(1)?;
            let hlc_bytes: Vec<u8> = row.get(2)?;
            let bundle_type_int: i32 = row.get(3)?;
            let op_count: i64 = row.get(4)?;
            let checksum_bytes: Vec<u8> = row.get(5)?;
            let creates_bytes: Vec<u8> = row.get(6)?;
            let deletes_bytes: Vec<u8> = row.get(7)?;
            let meta: Option<Vec<u8>> = row.get(8)?;
            let signature_bytes: Vec<u8> = row.get(9)?;
            let creator_vc_bytes: Option<Vec<u8>> = row.get(10)?;
            Ok((bundle_id_bytes, actor_id_bytes, hlc_bytes, bundle_type_int, op_count, checksum_bytes, creates_bytes, deletes_bytes, meta, signature_bytes, creator_vc_bytes))
        },
    )
    .map_err(StorageError::Sqlite)
    .and_then(|(bundle_id_bytes, actor_id_bytes, hlc_bytes, bundle_type_int, op_count, checksum_bytes, creates_bytes, deletes_bytes, meta, signature_bytes, creator_vc_bytes)| {
        let bundle_id = BundleId::from_bytes(to_array::<16>(bundle_id_bytes, "bundle_id")?);
        let actor_id = ActorId::from_bytes(to_array::<32>(actor_id_bytes, "actor_id")?);
        let hlc = Hlc::from_bytes(&to_array::<12>(hlc_bytes, "hlc")?);
        let bundle_type = match bundle_type_int {
            1 => BundleType::UserEdit,
            2 => BundleType::ScriptOutput,
            3 => BundleType::Import,
            4 => BundleType::System,
            _ => return Err(StorageError::Serialization(format!("unknown bundle_type: {bundle_type_int}"))),
        };
        let checksum: [u8; 32] = to_array::<32>(checksum_bytes, "checksum")?;
        let creates: Vec<EntityId> = rmp_serde::from_slice(&creates_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let deletes: Vec<EntityId> = rmp_serde::from_slice(&deletes_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let signature = Signature::from_bytes(to_array::<64>(signature_bytes, "signature")?);

        let creator_vc = match creator_vc_bytes {
            Some(bytes) => Some(openprod_core::vector_clock::VectorClock::from_msgpack(&bytes)
                .map_err(|e| StorageError::Serialization(e.to_string()))?),
            None => None,
        };

        Ok(Bundle {
            bundle_id,
            actor_id,
            hlc,
            bundle_type,
            op_count: op_count as u32,
            checksum,
            creates,
            deletes,
            meta,
            signature,
            creator_vc,
        })
    })
}

fn materialize_op(
    conn: &Connection,
    op: &Operation,
    bundle: &Bundle,
) -> Result<(), StorageError> {
    match &op.payload {
        OperationPayload::CreateEntity {
            entity_id,
            initial_table,
        } => {
            let result = conn.execute(
                "INSERT INTO entities (entity_id, created_at, created_by, created_in_bundle) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    entity_id.as_bytes().as_slice(),
                    &op.hlc.to_bytes()[..],
                    op.actor_id.as_bytes().as_slice(),
                    bundle.bundle_id.as_bytes().as_slice(),
                ],
            );
            match result {
                Ok(_) => {}
                Err(rusqlite::Error::SqliteFailure(err, _))
                    if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    return Err(StorageError::EntityCollision {
                        entity_id: entity_id.to_string(),
                    });
                }
                Err(e) => return Err(StorageError::Sqlite(e)),
            }

            if let Some(facet_type) = initial_table {
                conn.execute(
                    "INSERT INTO facets (entity_id, facet_type, attached_at, attached_by, attached_in_bundle) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        entity_id.as_bytes().as_slice(),
                        facet_type,
                        &op.hlc.to_bytes()[..],
                        op.actor_id.as_bytes().as_slice(),
                        bundle.bundle_id.as_bytes().as_slice(),
                    ],
                )?;
            }
        }

        OperationPayload::DeleteEntity {
            entity_id,
            cascade_edges,
        } => {
            conn.execute(
                "UPDATE entities SET deleted_at = ?1, deleted_by = ?2, deleted_in_bundle = ?3 WHERE entity_id = ?4",
                rusqlite::params![
                    &op.hlc.to_bytes()[..],
                    op.actor_id.as_bytes().as_slice(),
                    bundle.bundle_id.as_bytes().as_slice(),
                    entity_id.as_bytes().as_slice(),
                ],
            )?;
            for edge_id in cascade_edges {
                conn.execute(
                    "UPDATE edges SET deleted_at = ?1, deleted_by = ?2, deleted_in_bundle = ?3 WHERE edge_id = ?4",
                    rusqlite::params![
                        &op.hlc.to_bytes()[..],
                        op.actor_id.as_bytes().as_slice(),
                        bundle.bundle_id.as_bytes().as_slice(),
                        edge_id.as_bytes().as_slice(),
                    ],
                )?;
            }
        }

        OperationPayload::AttachFacet {
            entity_id,
            facet_type,
        } => {
            conn.execute(
                "INSERT INTO facets (entity_id, facet_type, attached_at, attached_by, attached_in_bundle) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(entity_id, facet_type) DO UPDATE SET attached_at = excluded.attached_at, attached_by = excluded.attached_by, attached_in_bundle = excluded.attached_in_bundle, detached_at = NULL, detached_by = NULL, detached_in_bundle = NULL, preserve_values = NULL",
                rusqlite::params![
                    entity_id.as_bytes().as_slice(),
                    facet_type,
                    &op.hlc.to_bytes()[..],
                    op.actor_id.as_bytes().as_slice(),
                    bundle.bundle_id.as_bytes().as_slice(),
                ],
            )?;
        }

        OperationPayload::DetachFacet {
            entity_id,
            facet_type,
            preserve_values,
        } => {
            if *preserve_values {
                let mut stmt =
                    conn.prepare("SELECT field_key, value FROM fields WHERE entity_id = ?1 AND value IS NOT NULL")?;
                let fields: Vec<(String, Vec<u8>)> = stmt
                    .query_map(
                        rusqlite::params![entity_id.as_bytes().as_slice()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                let preserved = rmp_serde::to_vec(&fields)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                conn.execute(
                    "UPDATE facets SET detached_at = ?1, detached_by = ?2, detached_in_bundle = ?3, preserve_values = ?4 WHERE entity_id = ?5 AND facet_type = ?6",
                    rusqlite::params![
                        &op.hlc.to_bytes()[..],
                        op.actor_id.as_bytes().as_slice(),
                        bundle.bundle_id.as_bytes().as_slice(),
                        preserved,
                        entity_id.as_bytes().as_slice(),
                        facet_type,
                    ],
                )?;
            } else {
                conn.execute(
                    "UPDATE facets SET detached_at = ?1, detached_by = ?2, detached_in_bundle = ?3 WHERE entity_id = ?4 AND facet_type = ?5",
                    rusqlite::params![
                        &op.hlc.to_bytes()[..],
                        op.actor_id.as_bytes().as_slice(),
                        bundle.bundle_id.as_bytes().as_slice(),
                        entity_id.as_bytes().as_slice(),
                        facet_type,
                    ],
                )?;
            }
        }

        OperationPayload::SetField {
            entity_id,
            field_key,
            value,
        } => {
            let value_bytes = value
                .to_msgpack()
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            conn.execute(
                "INSERT INTO fields (entity_id, field_key, value, source_op, source_actor, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(entity_id, field_key) DO UPDATE SET value = excluded.value, source_op = excluded.source_op, source_actor = excluded.source_actor, updated_at = excluded.updated_at
                 WHERE excluded.updated_at > fields.updated_at OR (excluded.updated_at = fields.updated_at AND excluded.source_op > fields.source_op)",
                rusqlite::params![
                    entity_id.as_bytes().as_slice(),
                    field_key,
                    value_bytes,
                    op.op_id.as_bytes().as_slice(),
                    op.actor_id.as_bytes().as_slice(),
                    &op.hlc.to_bytes()[..],
                ],
            )?;
        }

        OperationPayload::ClearField {
            entity_id,
            field_key,
        } => {
            // ClearField writes a tombstone (value = NULL) with LWW guard
            conn.execute(
                "INSERT INTO fields (entity_id, field_key, value, source_op, source_actor, updated_at) VALUES (?1, ?2, NULL, ?3, ?4, ?5)
                 ON CONFLICT(entity_id, field_key) DO UPDATE SET value = NULL, source_op = excluded.source_op, source_actor = excluded.source_actor, updated_at = excluded.updated_at
                 WHERE excluded.updated_at > fields.updated_at OR (excluded.updated_at = fields.updated_at AND excluded.source_op > fields.source_op)",
                rusqlite::params![
                    entity_id.as_bytes().as_slice(),
                    field_key,
                    op.op_id.as_bytes().as_slice(),
                    op.actor_id.as_bytes().as_slice(),
                    &op.hlc.to_bytes()[..],
                ],
            )?;
        }

        OperationPayload::ResolveConflict {
            entity_id,
            field_key,
            chosen_value,
            ..
        } => {
            // ResolveConflict materializes like SetField (with value) or ClearField (without)
            match chosen_value {
                Some(value) => {
                    let value_bytes = value
                        .to_msgpack()
                        .map_err(|e| StorageError::Serialization(e.to_string()))?;
                    conn.execute(
                        "INSERT INTO fields (entity_id, field_key, value, source_op, source_actor, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                         ON CONFLICT(entity_id, field_key) DO UPDATE SET value = excluded.value, source_op = excluded.source_op, source_actor = excluded.source_actor, updated_at = excluded.updated_at
                         WHERE excluded.updated_at > fields.updated_at OR (excluded.updated_at = fields.updated_at AND excluded.source_op > fields.source_op)",
                        rusqlite::params![
                            entity_id.as_bytes().as_slice(),
                            field_key,
                            value_bytes,
                            op.op_id.as_bytes().as_slice(),
                            op.actor_id.as_bytes().as_slice(),
                            &op.hlc.to_bytes()[..],
                        ],
                    )?;
                }
                None => {
                    conn.execute(
                        "INSERT INTO fields (entity_id, field_key, value, source_op, source_actor, updated_at) VALUES (?1, ?2, NULL, ?3, ?4, ?5)
                         ON CONFLICT(entity_id, field_key) DO UPDATE SET value = NULL, source_op = excluded.source_op, source_actor = excluded.source_actor, updated_at = excluded.updated_at
                         WHERE excluded.updated_at > fields.updated_at OR (excluded.updated_at = fields.updated_at AND excluded.source_op > fields.source_op)",
                        rusqlite::params![
                            entity_id.as_bytes().as_slice(),
                            field_key,
                            op.op_id.as_bytes().as_slice(),
                            op.actor_id.as_bytes().as_slice(),
                            &op.hlc.to_bytes()[..],
                        ],
                    )?;
                }
            }
        }

        OperationPayload::CreateEdge {
            edge_id,
            edge_type,
            source_id,
            target_id,
            properties,
        } => {
            conn.execute(
                "INSERT INTO edges (edge_id, edge_type, source_id, target_id, created_at, created_by, created_in_bundle) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    edge_id.as_bytes().as_slice(),
                    edge_type,
                    source_id.as_bytes().as_slice(),
                    target_id.as_bytes().as_slice(),
                    &op.hlc.to_bytes()[..],
                    op.actor_id.as_bytes().as_slice(),
                    bundle.bundle_id.as_bytes().as_slice(),
                ],
            )?;
            for (key, value) in properties {
                let value_bytes = value
                    .to_msgpack()
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                conn.execute(
                    "INSERT INTO edge_properties (edge_id, property_key, value, source_op, source_actor, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        edge_id.as_bytes().as_slice(),
                        key,
                        value_bytes,
                        op.op_id.as_bytes().as_slice(),
                        op.actor_id.as_bytes().as_slice(),
                        &op.hlc.to_bytes()[..],
                    ],
                )?;
            }
        }

        OperationPayload::SetEdgeProperty {
            edge_id,
            property_key,
            value,
        } => {
            let value_bytes = value
                .to_msgpack()
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            conn.execute(
                "INSERT INTO edge_properties (edge_id, property_key, value, source_op, source_actor, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(edge_id, property_key) DO UPDATE SET value = excluded.value, source_op = excluded.source_op, source_actor = excluded.source_actor, updated_at = excluded.updated_at
                 WHERE excluded.updated_at > edge_properties.updated_at OR (excluded.updated_at = edge_properties.updated_at AND excluded.source_op > edge_properties.source_op)",
                rusqlite::params![
                    edge_id.as_bytes().as_slice(),
                    property_key,
                    value_bytes,
                    op.op_id.as_bytes().as_slice(),
                    op.actor_id.as_bytes().as_slice(),
                    &op.hlc.to_bytes()[..],
                ],
            )?;
        }

        OperationPayload::ClearEdgeProperty {
            edge_id,
            property_key,
        } => {
            // ClearEdgeProperty writes a tombstone (value = NULL) with LWW guard
            // (mirrors ClearField pattern for correct out-of-order sync)
            conn.execute(
                "INSERT INTO edge_properties (edge_id, property_key, value, source_op, source_actor, updated_at) VALUES (?1, ?2, NULL, ?3, ?4, ?5)
                 ON CONFLICT(edge_id, property_key) DO UPDATE SET value = NULL, source_op = excluded.source_op, source_actor = excluded.source_actor, updated_at = excluded.updated_at
                 WHERE excluded.updated_at > edge_properties.updated_at OR (excluded.updated_at = edge_properties.updated_at AND excluded.source_op > edge_properties.source_op)",
                rusqlite::params![
                    edge_id.as_bytes().as_slice(),
                    property_key,
                    op.op_id.as_bytes().as_slice(),
                    op.actor_id.as_bytes().as_slice(),
                    &op.hlc.to_bytes()[..],
                ],
            )?;
        }

        OperationPayload::DeleteEdge { edge_id } => {
            conn.execute(
                "UPDATE edges SET deleted_at = ?1, deleted_by = ?2, deleted_in_bundle = ?3 WHERE edge_id = ?4",
                rusqlite::params![
                    &op.hlc.to_bytes()[..],
                    op.actor_id.as_bytes().as_slice(),
                    bundle.bundle_id.as_bytes().as_slice(),
                    edge_id.as_bytes().as_slice(),
                ],
            )?;
        }

        OperationPayload::RestoreEntity { entity_id } => {
            conn.execute(
                "UPDATE entities SET deleted_at = NULL, deleted_by = NULL, deleted_in_bundle = NULL WHERE entity_id = ?1",
                rusqlite::params![entity_id.as_bytes().as_slice()],
            )?;
        }

        OperationPayload::RestoreEdge { edge_id } => {
            conn.execute(
                "UPDATE edges SET deleted_at = NULL, deleted_by = NULL, deleted_in_bundle = NULL WHERE edge_id = ?1",
                rusqlite::params![edge_id.as_bytes().as_slice()],
            )?;
        }

        OperationPayload::RestoreFacet {
            entity_id,
            facet_type,
        } => {
            conn.execute(
                "UPDATE facets SET detached_at = NULL, detached_by = NULL, detached_in_bundle = NULL, preserve_values = NULL WHERE entity_id = ?1 AND facet_type = ?2",
                rusqlite::params![entity_id.as_bytes().as_slice(), facet_type],
            )?;
        }

        // Operations not yet materialized -- stored in oplog only
        OperationPayload::ApplyCrdt { .. }
        | OperationPayload::ClearAndAdd { .. }
        | OperationPayload::CreateOrderedEdge { .. }
        | OperationPayload::MoveOrderedEdge { .. }
        | OperationPayload::LinkTables { .. }
        | OperationPayload::UnlinkTables { .. }
        | OperationPayload::AddToTable { .. }
        | OperationPayload::RemoveFromTable { .. }
        | OperationPayload::ConfirmFieldMapping { .. }
        | OperationPayload::MergeEntities { .. }
        | OperationPayload::SplitEntity { .. }
        | OperationPayload::CreateRule { .. } => {}
    }
    Ok(())
}

impl Storage for SqliteStorage {
    fn append_bundle(
        &mut self,
        bundle: &Bundle,
        operations: &[Operation],
    ) -> Result<(), StorageError> {
        // Idempotent: skip if bundle already ingested
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM bundles WHERE bundle_id = ?1)",
            rusqlite::params![bundle.bundle_id.as_bytes().as_slice()],
            |row| row.get(0),
        )?;
        if exists {
            return Ok(());
        }

        self.conn.execute_batch("SAVEPOINT sp_append")?;

        let result = (|| -> Result<(), StorageError> {
            let creator_vc_bytes = bundle.creator_vc.as_ref().map(|vc| {
                vc.to_msgpack()
                    .map_err(|e| StorageError::Serialization(e.to_string()))
            }).transpose()?;

            self.conn.execute(
                "INSERT INTO bundles (bundle_id, actor_id, hlc, bundle_type, op_count, checksum, creates, deletes, meta, signature, creator_vector_clock) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    bundle.bundle_id.as_bytes().as_slice(),
                    bundle.actor_id.as_bytes().as_slice(),
                    &bundle.hlc.to_bytes()[..],
                    bundle.bundle_type as i32,
                    bundle.op_count as i64,
                    &bundle.checksum[..],
                    rmp_serde::to_vec(&bundle.creates)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                    rmp_serde::to_vec(&bundle.deletes)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                    bundle.meta.as_deref(),
                    bundle.signature.as_bytes().as_slice(),
                    creator_vc_bytes.as_deref(),
                ],
            )?;

            for op in operations {
                let payload_bytes = op.payload.to_msgpack()?;
                let mv_bytes = rmp_serde::to_vec(&op.module_versions)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                let entity_id_blob = op
                    .payload
                    .entity_id()
                    .map(|eid| eid.as_bytes().to_vec());

                self.conn.execute(
                    "INSERT INTO oplog (op_id, actor_id, hlc, bundle_id, payload, module_versions, signature, op_type, entity_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        op.op_id.as_bytes().as_slice(),
                        op.actor_id.as_bytes().as_slice(),
                        &op.hlc.to_bytes()[..],
                        op.bundle_id.as_bytes().as_slice(),
                        payload_bytes,
                        mv_bytes,
                        op.signature.as_bytes().as_slice(),
                        op.payload.op_type_name(),
                        entity_id_blob,
                    ],
                )?;

                materialize_op(&self.conn, op, bundle)?;

                self.conn.execute(
                    "INSERT OR IGNORE INTO actors (actor_id, display_name, first_seen_at) VALUES (?1, NULL, ?2)",
                    rusqlite::params![
                        op.actor_id.as_bytes().as_slice(),
                        &op.hlc.to_bytes()[..],
                    ],
                )?;

                self.conn.execute(
                    "INSERT INTO vector_clock (actor_id, max_hlc) VALUES (?1, ?2)
                     ON CONFLICT(actor_id) DO UPDATE SET max_hlc = excluded.max_hlc
                     WHERE excluded.max_hlc > vector_clock.max_hlc",
                    rusqlite::params![
                        op.actor_id.as_bytes().as_slice(),
                        &op.hlc.to_bytes()[..],
                    ],
                )?;
            }

            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute_batch("RELEASE sp_append")?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK TO sp_append; RELEASE sp_append");
                Err(e)
            }
        }
    }

    fn get_ops_canonical(&self) -> Result<Vec<Operation>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT op_id, actor_id, hlc, bundle_id, payload, module_versions, signature FROM oplog ORDER BY hlc, op_id",
        )?;
        let ops = stmt
            .query_map([], |row| {
                read_op(row).map_err(|e| match e {
                    StorageError::Sqlite(sq) => sq,
                    other => rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(OpaqueStorageError(other.to_string())),
                    ),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ops)
    }

    fn get_ops_by_bundle(&self, bundle_id: BundleId) -> Result<Vec<Operation>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT op_id, actor_id, hlc, bundle_id, payload, module_versions, signature FROM oplog WHERE bundle_id = ?1",
        )?;
        let ops = stmt
            .query_map(rusqlite::params![bundle_id.as_bytes().as_slice()], |row| {
                read_op(row).map_err(|e| match e {
                    StorageError::Sqlite(sq) => sq,
                    other => rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(OpaqueStorageError(other.to_string())),
                    ),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ops)
    }

    fn get_ops_by_actor_after(
        &self,
        actor_id: ActorId,
        after: Hlc,
    ) -> Result<Vec<Operation>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT op_id, actor_id, hlc, bundle_id, payload, module_versions, signature FROM oplog WHERE actor_id = ?1 AND hlc > ?2 ORDER BY hlc, op_id",
        )?;
        let ops = stmt
            .query_map(
                rusqlite::params![actor_id.as_bytes().as_slice(), &after.to_bytes()[..]],
                |row| {
                    read_op(row).map_err(|e| match e {
                        StorageError::Sqlite(sq) => sq,
                        other => rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Blob,
                            Box::new(OpaqueStorageError(other.to_string())),
                        ),
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ops)
    }

    fn op_count(&self) -> Result<u64, StorageError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM oplog", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    fn get_entity(&self, entity_id: EntityId) -> Result<Option<EntityRecord>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT entity_id, created_at, created_by, (deleted_at IS NOT NULL) FROM entities WHERE entity_id = ?1",
        )?;
        let mut rows = stmt.query_map(
            rusqlite::params![entity_id.as_bytes().as_slice()],
            |row| {
                let eid_bytes: Vec<u8> = row.get(0)?;
                let created_at_bytes: Vec<u8> = row.get(1)?;
                let created_by_bytes: Vec<u8> = row.get(2)?;
                let deleted: bool = row.get(3)?;
                Ok((eid_bytes, created_at_bytes, created_by_bytes, deleted))
            },
        )?;

        match rows.next() {
            Some(Ok((eid_bytes, created_at_bytes, created_by_bytes, deleted))) => {
                let entity_id =
                    EntityId::from_bytes(to_array::<16>(eid_bytes, "entity_id")?);
                let created_at =
                    Hlc::from_bytes(&to_array::<12>(created_at_bytes, "created_at")?);
                let created_by =
                    ActorId::from_bytes(to_array::<32>(created_by_bytes, "created_by")?);
                Ok(Some(EntityRecord {
                    entity_id,
                    created_at,
                    created_by,
                    deleted,
                }))
            }
            Some(Err(e)) => Err(StorageError::Sqlite(e)),
            None => Ok(None),
        }
    }

    fn get_fields(
        &self,
        entity_id: EntityId,
    ) -> Result<Vec<(String, FieldValue)>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT field_key, value FROM fields WHERE entity_id = ?1 AND value IS NOT NULL")?;
        let rows = stmt.query_map(
            rusqlite::params![entity_id.as_bytes().as_slice()],
            |row| {
                let key: String = row.get(0)?;
                let val_bytes: Vec<u8> = row.get(1)?;
                Ok((key, val_bytes))
            },
        )?;

        let mut result = Vec::new();
        for row in rows {
            let (key, val_bytes) = row?;
            let value = FieldValue::from_msgpack(&val_bytes)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            result.push((key, value));
        }
        Ok(result)
    }

    fn get_field(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<FieldValue>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM fields WHERE entity_id = ?1 AND field_key = ?2 AND value IS NOT NULL")?;
        let mut rows = stmt.query_map(
            rusqlite::params![entity_id.as_bytes().as_slice(), field_key],
            |row| {
                let val_bytes: Vec<u8> = row.get(0)?;
                Ok(val_bytes)
            },
        )?;

        match rows.next() {
            Some(Ok(val_bytes)) => {
                let value = FieldValue::from_msgpack(&val_bytes)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                Ok(Some(value))
            }
            Some(Err(e)) => Err(StorageError::Sqlite(e)),
            None => Ok(None),
        }
    }

    fn get_facets(&self, entity_id: EntityId) -> Result<Vec<FacetRecord>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT entity_id, facet_type, attached_at, attached_by, (detached_at IS NOT NULL) FROM facets WHERE entity_id = ?1",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![entity_id.as_bytes().as_slice()],
            |row| {
                let eid_bytes: Vec<u8> = row.get(0)?;
                let facet_type: String = row.get(1)?;
                let attached_at_bytes: Vec<u8> = row.get(2)?;
                let attached_by_bytes: Vec<u8> = row.get(3)?;
                let detached: bool = row.get(4)?;
                Ok((
                    eid_bytes,
                    facet_type,
                    attached_at_bytes,
                    attached_by_bytes,
                    detached,
                ))
            },
        )?;

        let mut result = Vec::new();
        for row in rows {
            let (eid_bytes, facet_type, attached_at_bytes, attached_by_bytes, detached) = row?;
            let entity_id = EntityId::from_bytes(to_array::<16>(eid_bytes, "entity_id")?);
            let attached_at =
                Hlc::from_bytes(&to_array::<12>(attached_at_bytes, "attached_at")?);
            let attached_by =
                ActorId::from_bytes(to_array::<32>(attached_by_bytes, "attached_by")?);
            result.push(FacetRecord {
                entity_id,
                facet_type,
                attached_at,
                attached_by,
                detached,
            });
        }
        Ok(result)
    }

    fn get_entities_by_facet(&self, facet_type: &str) -> Result<Vec<EntityId>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT entity_id FROM facets WHERE facet_type = ?1 AND detached_at IS NULL")?;
        let rows = stmt.query_map(rusqlite::params![facet_type], |row| {
            let eid_bytes: Vec<u8> = row.get(0)?;
            Ok(eid_bytes)
        })?;

        let mut result = Vec::new();
        for row in rows {
            let eid_bytes = row?;
            let entity_id = EntityId::from_bytes(to_array::<16>(eid_bytes, "entity_id")?);
            result.push(entity_id);
        }
        Ok(result)
    }

    fn get_edges_from(&self, entity_id: EntityId) -> Result<Vec<EdgeRecord>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT edge_id, edge_type, source_id, target_id, created_at, created_by, (deleted_at IS NOT NULL) FROM edges WHERE source_id = ?1",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![entity_id.as_bytes().as_slice()],
            extract_edge_row,
        )?;
        let mut result = Vec::new();
        for row in rows {
            result.push(parse_edge_row(row?)?);
        }
        Ok(result)
    }

    fn get_edges_to(&self, entity_id: EntityId) -> Result<Vec<EdgeRecord>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT edge_id, edge_type, source_id, target_id, created_at, created_by, (deleted_at IS NOT NULL) FROM edges WHERE target_id = ?1",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![entity_id.as_bytes().as_slice()],
            extract_edge_row,
        )?;
        let mut result = Vec::new();
        for row in rows {
            result.push(parse_edge_row(row?)?);
        }
        Ok(result)
    }

    fn get_vector_clock(&self) -> Result<VectorClock, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT actor_id, max_hlc FROM vector_clock")?;
        let rows = stmt.query_map([], |row| {
            let actor_id_bytes: Vec<u8> = row.get(0)?;
            let hlc_bytes: Vec<u8> = row.get(1)?;
            Ok((actor_id_bytes, hlc_bytes))
        })?;

        let mut vc = VectorClock::new();
        for row in rows {
            let (actor_id_bytes, hlc_bytes) = row?;
            let actor_id = ActorId::from_bytes(to_array::<32>(actor_id_bytes, "actor_id")?);
            let hlc = Hlc::from_bytes(&to_array::<12>(hlc_bytes, "max_hlc")?);
            vc.update(actor_id, hlc);
        }
        Ok(vc)
    }

    fn get_field_metadata(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<(ActorId, Hlc)>, StorageError> {
        let result = self.conn.query_row(
            "SELECT source_actor, updated_at FROM fields WHERE entity_id = ?1 AND field_key = ?2",
            rusqlite::params![entity_id.as_bytes().as_slice(), field_key],
            |row| {
                let actor_bytes: Vec<u8> = row.get(0)?;
                let hlc_bytes: Vec<u8> = row.get(1)?;
                Ok((actor_bytes, hlc_bytes))
            },
        );
        match result {
            Ok((actor_bytes, hlc_bytes)) => {
                let actor = ActorId::from_bytes(to_array::<32>(actor_bytes, "source_actor")?);
                let hlc = Hlc::from_bytes(&to_array::<12>(hlc_bytes, "updated_at")?);
                Ok(Some((actor, hlc)))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn get_edge(&self, edge_id: EdgeId) -> Result<Option<EdgeRecord>, StorageError> {
        let result = self.conn.query_row(
            "SELECT edge_id, edge_type, source_id, target_id, created_at, created_by, (deleted_at IS NOT NULL) FROM edges WHERE edge_id = ?1",
            rusqlite::params![edge_id.as_bytes().as_slice()],
            extract_edge_row,
        );
        match result {
            Ok(raw) => Ok(Some(parse_edge_row(raw)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn get_edge_properties(
        &self,
        edge_id: EdgeId,
    ) -> Result<Vec<(String, FieldValue)>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT property_key, value FROM edge_properties WHERE edge_id = ?1 AND value IS NOT NULL",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![edge_id.as_bytes().as_slice()],
            |row| {
                let key: String = row.get(0)?;
                let val_bytes: Vec<u8> = row.get(1)?;
                Ok((key, val_bytes))
            },
        )?;
        let mut result = Vec::new();
        for row in rows {
            let (key, val_bytes) = row?;
            let value = FieldValue::from_msgpack(&val_bytes)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            result.push((key, value));
        }
        Ok(result)
    }

    fn get_edge_property(
        &self,
        edge_id: EdgeId,
        key: &str,
    ) -> Result<Option<FieldValue>, StorageError> {
        let result = self.conn.query_row(
            "SELECT value FROM edge_properties WHERE edge_id = ?1 AND property_key = ?2 AND value IS NOT NULL",
            rusqlite::params![edge_id.as_bytes().as_slice(), key],
            |row| {
                let val_bytes: Vec<u8> = row.get(0)?;
                Ok(val_bytes)
            },
        );
        match result {
            Ok(val_bytes) => {
                let value = FieldValue::from_msgpack(&val_bytes)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                Ok(Some(value))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn get_edge_property_metadata(
        &self,
        edge_id: EdgeId,
        key: &str,
    ) -> Result<Option<(ActorId, Hlc)>, StorageError> {
        let result = self.conn.query_row(
            "SELECT source_actor, updated_at FROM edge_properties WHERE edge_id = ?1 AND property_key = ?2",
            rusqlite::params![edge_id.as_bytes().as_slice(), key],
            |row| {
                let actor_bytes: Vec<u8> = row.get(0)?;
                let hlc_bytes: Vec<u8> = row.get(1)?;
                Ok((actor_bytes, hlc_bytes))
            },
        );
        match result {
            Ok((actor_bytes, hlc_bytes)) => {
                let actor = ActorId::from_bytes(to_array::<32>(actor_bytes, "source_actor")?);
                let hlc = Hlc::from_bytes(&to_array::<12>(hlc_bytes, "updated_at")?);
                Ok(Some((actor, hlc)))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn insert_conflict(&mut self, record: &ConflictRecord) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO conflicts (conflict_id, entity_id, field_key, status, detected_at, detected_in_bundle) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                record.conflict_id.as_bytes().as_slice(),
                record.entity_id.as_bytes().as_slice(),
                record.field_key,
                record.status.as_str(),
                &record.detected_at.to_bytes()[..],
                record.detected_in_bundle.as_bytes().as_slice(),
            ],
        )?;
        for val in &record.values {
            self.conn.execute(
                "INSERT INTO conflict_values (conflict_id, actor_id, hlc, op_id, value) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    record.conflict_id.as_bytes().as_slice(),
                    val.actor_id.as_bytes().as_slice(),
                    &val.hlc.to_bytes()[..],
                    val.op_id.as_bytes().as_slice(),
                    val.value.as_deref(),
                ],
            )?;
        }
        Ok(())
    }

    fn update_conflict_resolved(
        &mut self,
        conflict_id: ConflictId,
        resolved_at: Hlc,
        resolved_by: ActorId,
        resolved_op: OpId,
        resolved_value: Option<Vec<u8>>,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "UPDATE conflicts SET status = 'resolved', resolved_at = ?1, resolved_by = ?2, resolved_op_id = ?3, resolved_value = ?4 WHERE conflict_id = ?5",
            rusqlite::params![
                &resolved_at.to_bytes()[..],
                resolved_by.as_bytes().as_slice(),
                resolved_op.as_bytes().as_slice(),
                resolved_value.as_deref(),
                conflict_id.as_bytes().as_slice(),
            ],
        )?;
        Ok(())
    }

    fn get_open_conflicts_for_entity(
        &self,
        entity_id: EntityId,
    ) -> Result<Vec<ConflictRecord>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT conflict_id, entity_id, field_key, status, detected_at, detected_in_bundle, resolved_at, resolved_by, resolved_op_id, resolved_value, reopened_at, reopened_by_op FROM conflicts WHERE entity_id = ?1 AND status = 'open'",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![entity_id.as_bytes().as_slice()],
            parse_conflict_row,
        )?;
        let mut result = Vec::new();
        for row in rows {
            let mut record = row.map_err(StorageError::Sqlite).and_then(|r| r)?;
            record.values = load_conflict_values(&self.conn, record.conflict_id)?;
            result.push(record);
        }
        Ok(result)
    }

    fn get_conflict(
        &self,
        conflict_id: ConflictId,
    ) -> Result<Option<ConflictRecord>, StorageError> {
        let result = self.conn.query_row(
            "SELECT conflict_id, entity_id, field_key, status, detected_at, detected_in_bundle, resolved_at, resolved_by, resolved_op_id, resolved_value, reopened_at, reopened_by_op FROM conflicts WHERE conflict_id = ?1",
            rusqlite::params![conflict_id.as_bytes().as_slice()],
            parse_conflict_row,
        );
        match result {
            Ok(record) => {
                let mut record = record?;
                record.values = load_conflict_values(&self.conn, record.conflict_id)?;
                Ok(Some(record))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn get_open_conflict_for_field(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<ConflictRecord>, StorageError> {
        let result = self.conn.query_row(
            "SELECT conflict_id, entity_id, field_key, status, detected_at, detected_in_bundle, resolved_at, resolved_by, resolved_op_id, resolved_value, reopened_at, reopened_by_op FROM conflicts WHERE entity_id = ?1 AND field_key = ?2 AND status = 'open'",
            rusqlite::params![entity_id.as_bytes().as_slice(), field_key],
            parse_conflict_row,
        );
        match result {
            Ok(record) => {
                let mut record = record?;
                record.values = load_conflict_values(&self.conn, record.conflict_id)?;
                Ok(Some(record))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn get_latest_conflict_for_field(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<ConflictRecord>, StorageError> {
        let result = self.conn.query_row(
            "SELECT conflict_id, entity_id, field_key, status, detected_at, detected_in_bundle, resolved_at, resolved_by, resolved_op_id, resolved_value, reopened_at, reopened_by_op FROM conflicts WHERE entity_id = ?1 AND field_key = ?2 ORDER BY detected_at DESC LIMIT 1",
            rusqlite::params![entity_id.as_bytes().as_slice(), field_key],
            parse_conflict_row,
        );
        match result {
            Ok(record) => {
                let mut record = record?;
                record.values = load_conflict_values(&self.conn, record.conflict_id)?;
                Ok(Some(record))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn reopen_conflict(
        &mut self,
        conflict_id: ConflictId,
        reopened_at: Hlc,
        reopened_by_op: OpId,
        new_values: &[ConflictValue],
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "UPDATE conflicts SET status = 'open', reopened_at = ?1, reopened_by_op = ?2 WHERE conflict_id = ?3",
            rusqlite::params![
                &reopened_at.to_bytes()[..],
                reopened_by_op.as_bytes().as_slice(),
                conflict_id.as_bytes().as_slice(),
            ],
        )?;
        // Replace all branch tips with the new values
        self.conn.execute(
            "DELETE FROM conflict_values WHERE conflict_id = ?1",
            rusqlite::params![conflict_id.as_bytes().as_slice()],
        )?;
        for val in new_values {
            self.conn.execute(
                "INSERT INTO conflict_values (conflict_id, actor_id, hlc, op_id, value) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    conflict_id.as_bytes().as_slice(),
                    val.actor_id.as_bytes().as_slice(),
                    &val.hlc.to_bytes()[..],
                    val.op_id.as_bytes().as_slice(),
                    val.value.as_deref(),
                ],
            )?;
        }
        Ok(())
    }

    fn add_conflict_value(
        &mut self,
        conflict_id: ConflictId,
        value: &ConflictValue,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO conflict_values (conflict_id, actor_id, hlc, op_id, value) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(conflict_id, actor_id) DO UPDATE SET hlc = excluded.hlc, op_id = excluded.op_id, value = excluded.value",
            rusqlite::params![
                conflict_id.as_bytes().as_slice(),
                value.actor_id.as_bytes().as_slice(),
                &value.hlc.to_bytes()[..],
                value.op_id.as_bytes().as_slice(),
                value.value.as_deref(),
            ],
        )?;
        Ok(())
    }

    fn get_bundle_vector_clock(
        &self,
        bundle_id: BundleId,
    ) -> Result<Option<VectorClock>, StorageError> {
        let result = self.conn.query_row(
            "SELECT creator_vector_clock FROM bundles WHERE bundle_id = ?1",
            rusqlite::params![bundle_id.as_bytes().as_slice()],
            |row| {
                let bytes: Option<Vec<u8>> = row.get(0)?;
                Ok(bytes)
            },
        );
        match result {
            Ok(Some(bytes)) => {
                let vc = VectorClock::from_msgpack(&bytes)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                Ok(Some(vc))
            }
            Ok(None) => Ok(None),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }
}

/// Parse a conflict row from the conflicts table (no value columns — values loaded separately).
/// Expected columns: conflict_id, entity_id, field_key, status, detected_at, detected_in_bundle,
///   resolved_at, resolved_by, resolved_op_id, resolved_value, reopened_at, reopened_by_op
fn parse_conflict_row(row: &rusqlite::Row) -> rusqlite::Result<Result<ConflictRecord, StorageError>> {
    let conflict_id_bytes: Vec<u8> = row.get(0)?;
    let entity_id_bytes: Vec<u8> = row.get(1)?;
    let field_key: String = row.get(2)?;
    let status_str: String = row.get(3)?;
    let detected_at_bytes: Vec<u8> = row.get(4)?;
    let detected_in_bundle_bytes: Vec<u8> = row.get(5)?;
    let resolved_at_bytes: Option<Vec<u8>> = row.get(6)?;
    let resolved_by_bytes: Option<Vec<u8>> = row.get(7)?;
    let resolved_op_bytes: Option<Vec<u8>> = row.get(8)?;
    let resolved_value: Option<Vec<u8>> = row.get(9)?;
    let reopened_at_bytes: Option<Vec<u8>> = row.get(10)?;
    let reopened_by_op_bytes: Option<Vec<u8>> = row.get(11)?;

    Ok((|| -> Result<ConflictRecord, StorageError> {
        Ok(ConflictRecord {
            conflict_id: ConflictId::from_bytes(to_array::<16>(conflict_id_bytes, "conflict_id")?),
            entity_id: EntityId::from_bytes(to_array::<16>(entity_id_bytes, "entity_id")?),
            field_key,
            status: ConflictStatus::parse(&status_str)?,
            values: Vec::new(), // loaded separately via load_conflict_values
            detected_at: Hlc::from_bytes(&to_array::<12>(detected_at_bytes, "detected_at")?),
            detected_in_bundle: BundleId::from_bytes(to_array::<16>(detected_in_bundle_bytes, "detected_in_bundle")?),
            resolved_at: resolved_at_bytes.map(|b| -> Result<_, StorageError> {
                Ok(Hlc::from_bytes(&to_array::<12>(b, "resolved_at")?))
            }).transpose()?,
            resolved_by: resolved_by_bytes.map(|b| -> Result<_, StorageError> {
                Ok(ActorId::from_bytes(to_array::<32>(b, "resolved_by")?))
            }).transpose()?,
            resolved_op_id: resolved_op_bytes.map(|b| -> Result<_, StorageError> {
                Ok(OpId::from_bytes(to_array::<16>(b, "resolved_op_id")?))
            }).transpose()?,
            resolved_value,
            reopened_at: reopened_at_bytes.map(|b| -> Result<_, StorageError> {
                Ok(Hlc::from_bytes(&to_array::<12>(b, "reopened_at")?))
            }).transpose()?,
            reopened_by_op: reopened_by_op_bytes.map(|b| -> Result<_, StorageError> {
                Ok(OpId::from_bytes(to_array::<16>(b, "reopened_by_op")?))
            }).transpose()?,
        })
    })())
}

/// Load all competing values for a conflict from the conflict_values table.
fn load_conflict_values(conn: &Connection, conflict_id: ConflictId) -> Result<Vec<ConflictValue>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT actor_id, hlc, op_id, value FROM conflict_values WHERE conflict_id = ?1",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![conflict_id.as_bytes().as_slice()],
        |row| {
            let actor_bytes: Vec<u8> = row.get(0)?;
            let hlc_bytes: Vec<u8> = row.get(1)?;
            let op_id_bytes: Vec<u8> = row.get(2)?;
            let value: Option<Vec<u8>> = row.get(3)?;
            Ok((actor_bytes, hlc_bytes, op_id_bytes, value))
        },
    )?;
    let mut values = Vec::new();
    for row in rows {
        let (actor_bytes, hlc_bytes, op_id_bytes, value) = row?;
        values.push(ConflictValue {
            actor_id: ActorId::from_bytes(to_array::<32>(actor_bytes, "actor_id")?),
            hlc: Hlc::from_bytes(&to_array::<12>(hlc_bytes, "hlc")?),
            op_id: OpId::from_bytes(to_array::<16>(op_id_bytes, "op_id")?),
            value,
        });
    }
    Ok(values)
}

/// Wrapper error type used to tunnel StorageError through rusqlite's error system
/// in query_map closures that must return rusqlite::Error.
#[derive(Debug)]
struct OpaqueStorageError(String);

impl std::fmt::Display for OpaqueStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for OpaqueStorageError {}

// ============================================================================
// Overlay CRUD (local-only, not on Storage trait)
// ============================================================================

impl SqliteStorage {
    pub fn insert_overlay(
        &mut self,
        overlay_id: OverlayId,
        display_name: &str,
        source: &str,
        status: &str,
        created_at: &Hlc,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO overlays (overlay_id, display_name, source, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            rusqlite::params![
                overlay_id.as_bytes().as_slice(),
                display_name,
                source,
                status,
                &created_at.to_bytes()[..],
            ],
        )?;
        Ok(())
    }

    pub fn update_overlay_status(
        &mut self,
        overlay_id: OverlayId,
        status: &str,
        updated_at: &Hlc,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "UPDATE overlays SET status = ?1, updated_at = ?2 WHERE overlay_id = ?3",
            rusqlite::params![
                status,
                &updated_at.to_bytes()[..],
                overlay_id.as_bytes().as_slice(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_overlay(&mut self, overlay_id: OverlayId) -> Result<(), StorageError> {
        // Delete overlay ops first (FK constraint)
        self.conn.execute(
            "DELETE FROM overlay_ops WHERE overlay_id = ?1",
            rusqlite::params![overlay_id.as_bytes().as_slice()],
        )?;
        self.conn.execute(
            "DELETE FROM overlays WHERE overlay_id = ?1",
            rusqlite::params![overlay_id.as_bytes().as_slice()],
        )?;
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub fn get_overlay(
        &self,
        overlay_id: OverlayId,
    ) -> Result<Option<(OverlayId, String, String, String, Hlc, Hlc)>, StorageError> {
        let result = self.conn.query_row(
            "SELECT overlay_id, display_name, source, status, created_at, updated_at FROM overlays WHERE overlay_id = ?1",
            rusqlite::params![overlay_id.as_bytes().as_slice()],
            |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let name: String = row.get(1)?;
                let source: String = row.get(2)?;
                let status: String = row.get(3)?;
                let created_bytes: Vec<u8> = row.get(4)?;
                let updated_bytes: Vec<u8> = row.get(5)?;
                Ok((id_bytes, name, source, status, created_bytes, updated_bytes))
            },
        );
        match result {
            Ok((id_bytes, name, source, status, created_bytes, updated_bytes)) => {
                let id = OverlayId::from_bytes(to_array::<16>(id_bytes, "overlay_id")?);
                let created = Hlc::from_bytes(&to_array::<12>(created_bytes, "created_at")?);
                let updated = Hlc::from_bytes(&to_array::<12>(updated_bytes, "updated_at")?);
                Ok(Some((id, name, source, status, created, updated)))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    pub fn list_overlays_by_status(
        &self,
        status: &str,
    ) -> Result<Vec<(OverlayId, String, String, Hlc)>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT overlay_id, display_name, source, created_at FROM overlays WHERE status = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map(rusqlite::params![status], |row| {
            let id_bytes: Vec<u8> = row.get(0)?;
            let name: String = row.get(1)?;
            let source: String = row.get(2)?;
            let created_bytes: Vec<u8> = row.get(3)?;
            Ok((id_bytes, name, source, created_bytes))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (id_bytes, name, source, created_bytes) = row?;
            let id = OverlayId::from_bytes(to_array::<16>(id_bytes, "overlay_id")?);
            let created = Hlc::from_bytes(&to_array::<12>(created_bytes, "created_at")?);
            result.push((id, name, source, created));
        }
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_overlay_op(
        &mut self,
        overlay_id: OverlayId,
        op_id: OpId,
        hlc: &Hlc,
        payload_bytes: &[u8],
        entity_id: Option<EntityId>,
        field_key: Option<&str>,
        op_type: &str,
        canonical_value_at_creation: Option<&[u8]>,
    ) -> Result<i64, StorageError> {
        let entity_id_blob = entity_id.map(|eid| eid.as_bytes().to_vec());
        self.conn.execute(
            "INSERT INTO overlay_ops (overlay_id, op_id, hlc, payload, entity_id, field_key, op_type, canonical_value_at_creation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                overlay_id.as_bytes().as_slice(),
                op_id.as_bytes().as_slice(),
                &hlc.to_bytes()[..],
                payload_bytes,
                entity_id_blob,
                field_key,
                op_type,
                canonical_value_at_creation,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_overlay_op(&mut self, rowid: i64) -> Result<(), StorageError> {
        self.conn.execute(
            "DELETE FROM overlay_ops WHERE rowid = ?1",
            rusqlite::params![rowid],
        )?;
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub fn get_overlay_ops(
        &self,
        overlay_id: OverlayId,
    ) -> Result<Vec<(i64, Vec<u8>, Vec<u8>, Vec<u8>, Option<Vec<u8>>, String, Option<Vec<u8>>, bool, Option<String>)>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT rowid, op_id, hlc, payload, entity_id, op_type, canonical_value_at_creation, canonical_drifted, field_key FROM overlay_ops WHERE overlay_id = ?1 ORDER BY rowid",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![overlay_id.as_bytes().as_slice()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Get the latest overlay op for a specific field on a specific entity.
    /// Returns (rowid, payload_bytes) or None.
    pub fn get_latest_overlay_field_op(
        &self,
        overlay_id: OverlayId,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<(i64, Vec<u8>)>, StorageError> {
        let result = self.conn.query_row(
            "SELECT rowid, payload FROM overlay_ops WHERE overlay_id = ?1 AND entity_id = ?2 AND field_key = ?3 ORDER BY rowid DESC LIMIT 1",
            rusqlite::params![
                overlay_id.as_bytes().as_slice(),
                entity_id.as_bytes().as_slice(),
                field_key,
            ],
            |row| {
                let rowid: i64 = row.get(0)?;
                let payload_bytes: Vec<u8> = row.get(1)?;
                Ok((rowid, payload_bytes))
            },
        );
        match result {
            Ok((rowid, payload_bytes)) => Ok(Some((rowid, payload_bytes))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    /// Count overlay ops for an overlay.
    pub fn count_overlay_ops(&self, overlay_id: OverlayId) -> Result<u64, StorageError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM overlay_ops WHERE overlay_id = ?1",
            rusqlite::params![overlay_id.as_bytes().as_slice()],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Mark SetField/ClearField overlay ops for an entity+field as drifted (across all overlays).
    /// Returns the number of rows updated.
    pub fn mark_overlay_ops_drifted(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<u64, StorageError> {
        let rows_affected = self.conn.execute(
            "UPDATE overlay_ops SET canonical_drifted = 1 WHERE entity_id = ?1 AND field_key = ?2 AND canonical_drifted = 0",
            rusqlite::params![entity_id.as_bytes().as_slice(), field_key],
        )?;
        Ok(rows_affected as u64)
    }

    /// Clear the canonical_drifted flag for overlay ops matching a specific field
    /// in a specific overlay+entity.
    pub fn clear_drift_flag(
        &self,
        overlay_id: OverlayId,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "UPDATE overlay_ops SET canonical_drifted = 0 WHERE overlay_id = ?1 AND entity_id = ?2 AND field_key = ?3 AND canonical_drifted = 1",
            rusqlite::params![
                overlay_id.as_bytes().as_slice(),
                entity_id.as_bytes().as_slice(),
                field_key,
            ],
        )?;
        Ok(())
    }

    /// Update canonical_value_at_creation for overlay ops matching a specific field
    /// in a specific overlay+entity.
    pub fn update_canonical_value_at_creation(
        &self,
        overlay_id: OverlayId,
        entity_id: EntityId,
        field_key: &str,
        new_value: Option<&[u8]>,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "UPDATE overlay_ops SET canonical_value_at_creation = ?4 WHERE overlay_id = ?1 AND entity_id = ?2 AND field_key = ?3",
            rusqlite::params![
                overlay_id.as_bytes().as_slice(),
                entity_id.as_bytes().as_slice(),
                field_key,
                new_value,
            ],
        )?;
        Ok(())
    }

    /// Get overlay ops where canonical_drifted = 1 for a specific overlay.
    /// Returns the same tuple type as `get_overlay_ops`.
    #[allow(clippy::type_complexity)]
    pub fn get_drifted_overlay_ops(
        &self,
        overlay_id: OverlayId,
    ) -> Result<Vec<(i64, Vec<u8>, Vec<u8>, Vec<u8>, Option<Vec<u8>>, String, Option<Vec<u8>>, bool, Option<String>)>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT rowid, op_id, hlc, payload, entity_id, op_type, canonical_value_at_creation, canonical_drifted, field_key FROM overlay_ops WHERE overlay_id = ?1 AND canonical_drifted = 1 ORDER BY rowid",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![overlay_id.as_bytes().as_slice()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Count overlay ops with canonical_drifted = 1 for a specific overlay.
    pub fn count_unresolved_drift(
        &self,
        overlay_id: OverlayId,
    ) -> Result<u64, StorageError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM overlay_ops WHERE overlay_id = ?1 AND canonical_drifted = 1",
            rusqlite::params![overlay_id.as_bytes().as_slice()],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Delete overlay ops for a specific field (used for knockout).
    /// Returns the number of rows deleted.
    pub fn delete_overlay_ops_for_field(
        &self,
        overlay_id: OverlayId,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<u64, StorageError> {
        let rows_affected = self.conn.execute(
            "DELETE FROM overlay_ops WHERE overlay_id = ?1 AND entity_id = ?2 AND field_key = ?3",
            rusqlite::params![
                overlay_id.as_bytes().as_slice(),
                entity_id.as_bytes().as_slice(),
                field_key,
            ],
        )?;
        Ok(rows_affected as u64)
    }
}
