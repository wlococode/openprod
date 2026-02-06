use std::collections::BTreeMap;

use rusqlite::Connection;

use openprod_core::{
    field_value::FieldValue,
    hlc::Hlc,
    ids::*,
    operations::{Bundle, Operation, OperationPayload},
    vector_clock::VectorClock,
};

use crate::error::StorageError;
use crate::traits::{EdgeRecord, EntityRecord, FacetRecord, Storage};

/// Convert Vec<u8> to fixed-size array with proper error handling.
fn to_array<const N: usize>(v: Vec<u8>, label: &str) -> Result<[u8; N], StorageError> {
    v.try_into()
        .map_err(|_| StorageError::Serialization(format!("invalid {label} length")))
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
    let hlc = Hlc::from_bytes(&to_array::<12>(hlc_bytes, "hlc")?)?;
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

fn materialize_op(
    tx: &rusqlite::Transaction,
    op: &Operation,
    bundle: &Bundle,
) -> Result<(), StorageError> {
    match &op.payload {
        OperationPayload::CreateEntity {
            entity_id,
            initial_table,
        } => {
            let result = tx.execute(
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
                tx.execute(
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
            tx.execute(
                "UPDATE entities SET deleted_at = ?1, deleted_by = ?2, deleted_in_bundle = ?3 WHERE entity_id = ?4",
                rusqlite::params![
                    &op.hlc.to_bytes()[..],
                    op.actor_id.as_bytes().as_slice(),
                    bundle.bundle_id.as_bytes().as_slice(),
                    entity_id.as_bytes().as_slice(),
                ],
            )?;
            for edge_id in cascade_edges {
                tx.execute(
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
            tx.execute(
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

        OperationPayload::DetachFacet {
            entity_id,
            facet_type,
            preserve_values,
        } => {
            if *preserve_values {
                let mut stmt =
                    tx.prepare("SELECT field_key, value FROM fields WHERE entity_id = ?1")?;
                let fields: Vec<(String, Vec<u8>)> = stmt
                    .query_map(
                        rusqlite::params![entity_id.as_bytes().as_slice()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                let preserved = rmp_serde::to_vec(&fields)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                tx.execute(
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
                tx.execute(
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
            tx.execute(
                "INSERT INTO fields (entity_id, field_key, value, source_op, source_actor, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(entity_id, field_key) DO UPDATE SET value = excluded.value, source_op = excluded.source_op, source_actor = excluded.source_actor, updated_at = excluded.updated_at",
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
            tx.execute(
                "DELETE FROM fields WHERE entity_id = ?1 AND field_key = ?2",
                rusqlite::params![entity_id.as_bytes().as_slice(), field_key,],
            )?;
        }

        OperationPayload::CreateEdge {
            edge_id,
            edge_type,
            source_id,
            target_id,
            properties,
        } => {
            tx.execute(
                "INSERT INTO edges (edge_id, edge_type, source_id, target_id, properties, created_at, created_by, created_in_bundle) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    edge_id.as_bytes().as_slice(),
                    edge_type,
                    source_id.as_bytes().as_slice(),
                    target_id.as_bytes().as_slice(),
                    properties,
                    &op.hlc.to_bytes()[..],
                    op.actor_id.as_bytes().as_slice(),
                    bundle.bundle_id.as_bytes().as_slice(),
                ],
            )?;
        }

        OperationPayload::DeleteEdge { edge_id } => {
            tx.execute(
                "UPDATE edges SET deleted_at = ?1, deleted_by = ?2, deleted_in_bundle = ?3 WHERE edge_id = ?4",
                rusqlite::params![
                    &op.hlc.to_bytes()[..],
                    op.actor_id.as_bytes().as_slice(),
                    bundle.bundle_id.as_bytes().as_slice(),
                    edge_id.as_bytes().as_slice(),
                ],
            )?;
        }

        // Operations not materialized in Phase 1 -- stored in oplog only
        _ => {}
    }
    Ok(())
}

impl Storage for SqliteStorage {
    fn append_bundle(
        &mut self,
        bundle: &Bundle,
        operations: &[Operation],
    ) -> Result<(), StorageError> {
        let tx = self.conn.transaction()?;

        tx.execute(
            "INSERT INTO bundles (bundle_id, actor_id, hlc, bundle_type, op_count, checksum, creates, deletes, meta, signature) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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

            tx.execute(
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

            materialize_op(&tx, op, bundle)?;

            tx.execute(
                "INSERT OR IGNORE INTO actors (actor_id, display_name, first_seen_at) VALUES (?1, NULL, ?2)",
                rusqlite::params![
                    op.actor_id.as_bytes().as_slice(),
                    &op.hlc.to_bytes()[..],
                ],
            )?;

            tx.execute(
                "INSERT INTO vector_clock (actor_id, max_hlc) VALUES (?1, ?2)
                 ON CONFLICT(actor_id) DO UPDATE SET max_hlc = excluded.max_hlc
                 WHERE excluded.max_hlc > vector_clock.max_hlc",
                rusqlite::params![
                    op.actor_id.as_bytes().as_slice(),
                    &op.hlc.to_bytes()[..],
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
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
                    Hlc::from_bytes(&to_array::<12>(created_at_bytes, "created_at")?)?;
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
            .prepare("SELECT field_key, value FROM fields WHERE entity_id = ?1")?;
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
            .prepare("SELECT value FROM fields WHERE entity_id = ?1 AND field_key = ?2")?;
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
                Hlc::from_bytes(&to_array::<12>(attached_at_bytes, "attached_at")?)?;
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
            "SELECT edge_id, edge_type, source_id, target_id, properties, created_at, created_by, (deleted_at IS NOT NULL) FROM edges WHERE source_id = ?1",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![entity_id.as_bytes().as_slice()],
            |row| {
                let edge_id_bytes: Vec<u8> = row.get(0)?;
                let edge_type: String = row.get(1)?;
                let source_id_bytes: Vec<u8> = row.get(2)?;
                let target_id_bytes: Vec<u8> = row.get(3)?;
                let properties: Option<Vec<u8>> = row.get(4)?;
                let created_at_bytes: Vec<u8> = row.get(5)?;
                let created_by_bytes: Vec<u8> = row.get(6)?;
                let deleted: bool = row.get(7)?;
                Ok((
                    edge_id_bytes,
                    edge_type,
                    source_id_bytes,
                    target_id_bytes,
                    properties,
                    created_at_bytes,
                    created_by_bytes,
                    deleted,
                ))
            },
        )?;

        let mut result = Vec::new();
        for row in rows {
            let (
                edge_id_bytes,
                edge_type,
                source_id_bytes,
                target_id_bytes,
                properties,
                created_at_bytes,
                created_by_bytes,
                deleted,
            ) = row?;
            let edge_id = EdgeId::from_bytes(to_array::<16>(edge_id_bytes, "edge_id")?);
            let source_id = EntityId::from_bytes(to_array::<16>(source_id_bytes, "source_id")?);
            let target_id = EntityId::from_bytes(to_array::<16>(target_id_bytes, "target_id")?);
            let created_at =
                Hlc::from_bytes(&to_array::<12>(created_at_bytes, "created_at")?)?;
            let created_by =
                ActorId::from_bytes(to_array::<32>(created_by_bytes, "created_by")?);
            result.push(EdgeRecord {
                edge_id,
                edge_type,
                source_id,
                target_id,
                properties: properties.unwrap_or_default(),
                created_at,
                created_by,
                deleted,
            });
        }
        Ok(result)
    }

    fn get_edges_to(&self, entity_id: EntityId) -> Result<Vec<EdgeRecord>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT edge_id, edge_type, source_id, target_id, properties, created_at, created_by, (deleted_at IS NOT NULL) FROM edges WHERE target_id = ?1",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![entity_id.as_bytes().as_slice()],
            |row| {
                let edge_id_bytes: Vec<u8> = row.get(0)?;
                let edge_type: String = row.get(1)?;
                let source_id_bytes: Vec<u8> = row.get(2)?;
                let target_id_bytes: Vec<u8> = row.get(3)?;
                let properties: Option<Vec<u8>> = row.get(4)?;
                let created_at_bytes: Vec<u8> = row.get(5)?;
                let created_by_bytes: Vec<u8> = row.get(6)?;
                let deleted: bool = row.get(7)?;
                Ok((
                    edge_id_bytes,
                    edge_type,
                    source_id_bytes,
                    target_id_bytes,
                    properties,
                    created_at_bytes,
                    created_by_bytes,
                    deleted,
                ))
            },
        )?;

        let mut result = Vec::new();
        for row in rows {
            let (
                edge_id_bytes,
                edge_type,
                source_id_bytes,
                target_id_bytes,
                properties,
                created_at_bytes,
                created_by_bytes,
                deleted,
            ) = row?;
            let edge_id = EdgeId::from_bytes(to_array::<16>(edge_id_bytes, "edge_id")?);
            let source_id = EntityId::from_bytes(to_array::<16>(source_id_bytes, "source_id")?);
            let target_id = EntityId::from_bytes(to_array::<16>(target_id_bytes, "target_id")?);
            let created_at =
                Hlc::from_bytes(&to_array::<12>(created_at_bytes, "created_at")?)?;
            let created_by =
                ActorId::from_bytes(to_array::<32>(created_by_bytes, "created_by")?);
            result.push(EdgeRecord {
                edge_id,
                edge_type,
                source_id,
                target_id,
                properties: properties.unwrap_or_default(),
                created_at,
                created_by,
                deleted,
            });
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
            let hlc = Hlc::from_bytes(&to_array::<12>(hlc_bytes, "max_hlc")?)?;
            vc.update(actor_id, hlc);
        }
        Ok(vc)
    }
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
