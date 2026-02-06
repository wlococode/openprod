pub mod error;
pub mod overlay;
pub mod undo;

pub use error::EngineError;
pub use overlay::{DriftRecord, OverlayManager, OverlayOpRecord, OverlayRecord, OverlaySource, OverlayStatus};

use std::collections::BTreeMap;

use openprod_core::{
    field_value::FieldValue,
    hlc::{Hlc, HlcClock},
    identity::ActorIdentity,
    ids::*,
    operations::{Bundle, BundleType, Operation, OperationPayload},
    vector_clock::VectorClock,
};
use openprod_storage::{
    ConflictRecord, ConflictStatus, ConflictValue,
    EdgeRecord, EntityRecord, FacetRecord, SqliteStorage, Storage,
};

use crate::undo::UndoManager;

const DEFAULT_UNDO_DEPTH: usize = 100;

#[derive(Debug)]
pub enum UndoResult {
    Applied(BundleId),
    Skipped { conflicts: Vec<UndoConflict> },
    Empty,
}

#[derive(Debug)]
pub struct UndoConflict {
    pub entity_id: EntityId,
    pub field_key: String,
    pub modified_by: ActorId,
}

pub struct Engine {
    identity: ActorIdentity,
    clock: HlcClock,
    storage: SqliteStorage,
    undo_manager: UndoManager,
    overlay_manager: OverlayManager,
}

impl Engine {
    pub fn new(identity: ActorIdentity, storage: SqliteStorage) -> Self {
        Self {
            identity,
            clock: HlcClock::new(),
            storage,
            undo_manager: UndoManager::new(DEFAULT_UNDO_DEPTH),
            overlay_manager: OverlayManager::new(),
        }
    }

    pub fn actor_id(&self) -> ActorId {
        self.identity.actor_id()
    }

    pub fn identity(&self) -> &ActorIdentity {
        &self.identity
    }

    pub fn storage(&self) -> &SqliteStorage {
        &self.storage
    }

    pub fn storage_mut(&mut self) -> &mut SqliteStorage {
        &mut self.storage
    }

    /// Execute a batch SQL statement on the underlying connection, mapping errors.
    fn exec_batch(&self, sql: &str) -> Result<(), EngineError> {
        self.storage.conn().execute_batch(sql)
            .map_err(|e| EngineError::Storage(openprod_storage::StorageError::Sqlite(e)))
    }

    /// Core internal method for executing a bundle of operations.
    /// If `is_undoable`, captures a pre-execution snapshot and pushes to undo stack.
    /// If an overlay is active, routes writes to overlay_ops instead of canonical storage.
    /// Returns (BundleId, Hlc).
    pub(crate) fn execute_internal(
        &mut self,
        bundle_type: BundleType,
        payloads: Vec<OperationPayload>,
        is_undoable: bool,
    ) -> Result<(BundleId, Hlc), EngineError> {
        // Check for active overlay — if present, route to overlay storage
        if let Some(overlay_id) = self.overlay_manager.active_overlay_id() {
            return self.execute_overlay(overlay_id, payloads);
        }

        let bundle_id = BundleId::new();
        let hlc = self.clock.tick()?;
        let module_versions = BTreeMap::new();

        // Capture pre-execution snapshot if undoable
        let snapshot = if is_undoable {
            Some(self.undo_manager.capture_snapshot(&self.storage, &payloads)?)
        } else {
            None
        };

        // Create signed operations
        let mut operations = Vec::new();
        for payload in &payloads {
            let op = Operation::new_signed(
                &self.identity,
                hlc,
                bundle_id,
                module_versions.clone(),
                payload.clone(),
            )?;
            operations.push(op);
        }

        // Get current vector clock for causal tracking
        let creator_vc = Some(self.storage.get_vector_clock()?);

        // Create and sign bundle
        let bundle = Bundle::new_signed(
            bundle_id,
            &self.identity,
            hlc,
            bundle_type,
            &operations,
            creator_vc,
        )?;

        // Append to storage
        self.storage.append_bundle(&bundle, &operations)?;

        // Push to undo stack if undoable
        if let Some(snapshot) = snapshot {
            self.undo_manager.push_undo(bundle_id, hlc, payloads.clone(), snapshot);
            self.undo_manager.clear_redo();
        }

        Ok((bundle_id, hlc))
    }

    /// Route operations to overlay storage instead of canonical.
    /// No signing, no bundle creation, no broadcast.
    fn execute_overlay(
        &mut self,
        overlay_id: OverlayId,
        payloads: Vec<OperationPayload>,
    ) -> Result<(BundleId, Hlc), EngineError> {
        let hlc = self.clock.tick()?;
        // Use a synthetic BundleId for tracking (not a real bundle)
        let synthetic_bundle_id = BundleId::new();

        for payload in &payloads {
            let op_id = OpId::new();
            let payload_bytes = payload.to_msgpack()?;
            let entity_id = payload.entity_id();
            let op_type = payload.op_type_name();

            // Capture canonical value and field_key at creation time for drift tracking
            let (canonical_value, field_key) = match payload {
                OperationPayload::SetField { entity_id, field_key, .. }
                | OperationPayload::ClearField { entity_id, field_key } => {
                    let cv = match self.storage.get_field(*entity_id, field_key)? {
                        Some(v) => {
                            let bytes = v.to_msgpack()
                                .map_err(|e| EngineError::Core(openprod_core::CoreError::Serialization(e.to_string())))?;
                            Some(bytes)
                        }
                        None => None,
                    };
                    (cv, Some(field_key.as_str()))
                }
                _ => (None, None),
            };

            let rowid = self.storage.insert_overlay_op(
                overlay_id,
                op_id,
                &hlc,
                &payload_bytes,
                entity_id,
                field_key,
                op_type,
                canonical_value.as_deref(),
            )?;

            // Push to overlay undo stack
            self.overlay_manager.push_overlay_undo(OverlayOpRecord {
                rowid,
                overlay_id,
                op_id,
                hlc,
                payload: payload.clone(),
                entity_id,
                field_key: field_key.map(|s| s.to_string()),
                op_type: op_type.to_string(),
                canonical_value_at_creation: canonical_value,
                canonical_drifted: false,
            });
        }

        Ok((synthetic_bundle_id, hlc))
    }

    /// Check that an entity exists and is not deleted.
    fn require_live_entity(&self, entity_id: EntityId) -> Result<(), EngineError> {
        match self.storage.get_entity(entity_id)? {
            None => Err(EngineError::EntityNotFound(entity_id.to_string())),
            Some(e) if e.deleted => Err(EngineError::EntityAlreadyDeleted(entity_id.to_string())),
            Some(_) => Ok(()),
        }
    }

    // ========================================================================
    // Typed Commands (all undoable)
    // ========================================================================

    /// Create a new entity with an optional initial table/facet.
    pub fn create_entity(
        &mut self,
        initial_table: Option<&str>,
    ) -> Result<(EntityId, BundleId), EngineError> {
        let entity_id = EntityId::new();
        let payloads = vec![OperationPayload::CreateEntity {
            entity_id,
            initial_table: initial_table.map(|s| s.to_string()),
        }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok((entity_id, bundle_id))
    }

    /// Create an entity with a facet and initial fields.
    pub fn create_entity_with_fields(
        &mut self,
        facet_type: &str,
        fields: Vec<(&str, FieldValue)>,
    ) -> Result<(EntityId, BundleId), EngineError> {
        let entity_id = EntityId::new();
        let mut payloads = vec![OperationPayload::CreateEntity {
            entity_id,
            initial_table: Some(facet_type.to_string()),
        }];
        for (key, value) in fields {
            payloads.push(OperationPayload::SetField {
                entity_id,
                field_key: key.to_string(),
                value,
            });
        }
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok((entity_id, bundle_id))
    }

    /// Set a field value on an entity.
    pub fn set_field(
        &mut self,
        entity_id: EntityId,
        field_key: &str,
        value: FieldValue,
    ) -> Result<BundleId, EngineError> {
        self.require_live_entity(entity_id)?;
        let payloads = vec![OperationPayload::SetField {
            entity_id,
            field_key: field_key.to_string(),
            value,
        }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok(bundle_id)
    }

    /// Clear a field on an entity.
    pub fn clear_field(
        &mut self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<BundleId, EngineError> {
        self.require_live_entity(entity_id)?;
        let payloads = vec![OperationPayload::ClearField {
            entity_id,
            field_key: field_key.to_string(),
        }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok(bundle_id)
    }

    /// Delete an entity, cascading to connected edges.
    pub fn delete_entity(
        &mut self,
        entity_id: EntityId,
    ) -> Result<BundleId, EngineError> {
        self.require_live_entity(entity_id)?;
        // Compute cascade edges
        let edges_from = self.storage.get_edges_from(entity_id)?;
        let edges_to = self.storage.get_edges_to(entity_id)?;
        let cascade_edges: Vec<EdgeId> = edges_from
            .iter()
            .chain(edges_to.iter())
            .filter(|e| !e.deleted)
            .map(|e| e.edge_id)
            .collect();

        let payloads = vec![OperationPayload::DeleteEntity {
            entity_id,
            cascade_edges,
        }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok(bundle_id)
    }

    /// Attach a facet to an entity.
    pub fn attach_facet(
        &mut self,
        entity_id: EntityId,
        facet_type: &str,
    ) -> Result<BundleId, EngineError> {
        self.require_live_entity(entity_id)?;
        let payloads = vec![OperationPayload::AttachFacet {
            entity_id,
            facet_type: facet_type.to_string(),
        }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok(bundle_id)
    }

    /// Detach a facet from an entity.
    pub fn detach_facet(
        &mut self,
        entity_id: EntityId,
        facet_type: &str,
        preserve_values: bool,
    ) -> Result<BundleId, EngineError> {
        self.require_live_entity(entity_id)?;
        let payloads = vec![OperationPayload::DetachFacet {
            entity_id,
            facet_type: facet_type.to_string(),
            preserve_values,
        }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok(bundle_id)
    }

    /// Create an edge between two entities.
    pub fn create_edge(
        &mut self,
        edge_type: &str,
        source_id: EntityId,
        target_id: EntityId,
    ) -> Result<(EdgeId, BundleId), EngineError> {
        self.require_live_entity(source_id)?;
        self.require_live_entity(target_id)?;
        let edge_id = EdgeId::new();
        let payloads = vec![OperationPayload::CreateEdge {
            edge_id,
            edge_type: edge_type.to_string(),
            source_id,
            target_id,
            properties: Vec::new(),
        }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok((edge_id, bundle_id))
    }

    /// Create an edge between two entities with initial properties.
    pub fn create_edge_with_properties(
        &mut self,
        edge_type: &str,
        source_id: EntityId,
        target_id: EntityId,
        properties: Vec<(&str, FieldValue)>,
    ) -> Result<(EdgeId, BundleId), EngineError> {
        self.require_live_entity(source_id)?;
        self.require_live_entity(target_id)?;
        let edge_id = EdgeId::new();
        let payloads = vec![OperationPayload::CreateEdge {
            edge_id,
            edge_type: edge_type.to_string(),
            source_id,
            target_id,
            properties: properties.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok((edge_id, bundle_id))
    }

    /// Set a property on an edge.
    pub fn set_edge_property(
        &mut self,
        edge_id: EdgeId,
        property_key: &str,
        value: FieldValue,
    ) -> Result<BundleId, EngineError> {
        let payloads = vec![OperationPayload::SetEdgeProperty {
            edge_id,
            property_key: property_key.to_string(),
            value,
        }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok(bundle_id)
    }

    /// Clear a property on an edge.
    pub fn clear_edge_property(
        &mut self,
        edge_id: EdgeId,
        property_key: &str,
    ) -> Result<BundleId, EngineError> {
        let payloads = vec![OperationPayload::ClearEdgeProperty {
            edge_id,
            property_key: property_key.to_string(),
        }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok(bundle_id)
    }

    /// Delete an edge.
    pub fn delete_edge(
        &mut self,
        edge_id: EdgeId,
    ) -> Result<BundleId, EngineError> {
        let payloads = vec![OperationPayload::DeleteEdge { edge_id }];
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, payloads, true)?;
        Ok(bundle_id)
    }

    /// Execute a raw batch of operation payloads as a single bundle.
    /// Only `UserEdit` bundles are pushed to the undo stack.
    pub fn execute(
        &mut self,
        bundle_type: BundleType,
        payloads: Vec<OperationPayload>,
    ) -> Result<BundleId, EngineError> {
        let is_undoable = matches!(bundle_type, BundleType::UserEdit);
        let (bundle_id, _) = self.execute_internal(bundle_type, payloads, is_undoable)?;
        Ok(bundle_id)
    }

    // ========================================================================
    // Undo / Redo
    // ========================================================================

    /// Undo the most recent undoable command.
    /// Returns `Applied(bundle_id)` if undo was successful.
    /// Returns `Skipped { conflicts }` if another actor modified the same fields (skip-and-advance).
    /// Returns `Empty` if there's nothing to undo.
    pub fn undo(&mut self) -> Result<UndoResult, EngineError> {
        let entry = match self.undo_manager.pop_undo() {
            Some(entry) => entry,
            None => return Ok(UndoResult::Empty),
        };

        // Check for conflicts: for each field in the snapshot, see if another actor
        // modified it after the original bundle was executed
        let my_actor = self.actor_id();
        let mut conflicts = Vec::new();

        for field_snap in &entry.snapshot.field_states {
            if let Some((actor, hlc)) = self.storage.get_field_metadata(
                field_snap.entity_id,
                &field_snap.field_key,
            )?
                && actor != my_actor && hlc > entry.bundle_hlc
            {
                conflicts.push(UndoConflict {
                    entity_id: field_snap.entity_id,
                    field_key: field_snap.field_key.clone(),
                    modified_by: actor,
                });
            }
        }

        // Also check entity-level conflicts: if undoing a CreateEntity,
        // check if any other actor wrote fields to it
        for entity_snap in &entry.snapshot.entity_states {
            // If entity didn't exist before (we're undoing a create), check if others wrote to it
            if entity_snap.existed.is_none() {
                let fields = self.storage.get_fields(entity_snap.entity_id)?;
                for (field_key, _) in &fields {
                    if let Some((actor, _)) = self.storage.get_field_metadata(
                        entity_snap.entity_id,
                        field_key,
                    )?
                        && actor != my_actor
                    {
                        conflicts.push(UndoConflict {
                            entity_id: entity_snap.entity_id,
                            field_key: field_key.clone(),
                            modified_by: actor,
                        });
                    }
                }
            }
        }

        // If conflicts, skip and advance (entry is consumed)
        if !conflicts.is_empty() {
            return Ok(UndoResult::Skipped { conflicts });
        }

        // Compute inverse operations
        let mut inverse = self.undo_manager.compute_inverse(&entry);

        // For CreateEntity undo -> DeleteEntity, compute fresh cascade_edges from storage
        for payload in &mut inverse {
            if let OperationPayload::DeleteEntity { entity_id, cascade_edges } = payload {
                let edges_from = self.storage.get_edges_from(*entity_id)?;
                let edges_to = self.storage.get_edges_to(*entity_id)?;
                *cascade_edges = edges_from
                    .iter()
                    .chain(edges_to.iter())
                    .filter(|e| !e.deleted)
                    .map(|e| e.edge_id)
                    .collect();
            }
        }

        // Execute inverse as non-undoable
        let (bundle_id, _) = self.execute_internal(BundleType::UserEdit, inverse, false)?;

        // Push original entry to redo stack
        self.undo_manager.push_redo(entry);

        Ok(UndoResult::Applied(bundle_id))
    }

    /// Redo the most recently undone command.
    /// Returns `Applied(bundle_id)` if redo was successful.
    /// Returns `Empty` if there's nothing to redo.
    pub fn redo(&mut self) -> Result<UndoResult, EngineError> {
        let entry = match self.undo_manager.pop_redo() {
            Some(entry) => entry,
            None => return Ok(UndoResult::Empty),
        };

        // Fix up payloads for current DB state (soft-deleted entities/edges
        // need RestoreEntity/RestoreEdge instead of CreateEntity/CreateEdge)
        let mut fixed_payloads = Vec::new();
        for payload in &entry.payloads {
            match payload {
                OperationPayload::CreateEntity { entity_id, initial_table } => {
                    // Check if entity exists and is soft-deleted
                    if let Some(entity) = self.storage.get_entity(*entity_id)?
                        && entity.deleted
                    {
                        // Entity exists but is soft-deleted — restore it
                        fixed_payloads.push(OperationPayload::RestoreEntity {
                            entity_id: *entity_id,
                        });
                        // Re-attach facet if there was an initial_table
                        if let Some(facet_type) = initial_table {
                            // Check if the facet is detached and needs reattaching
                            let facets = self.storage.get_facets(*entity_id)?;
                            let facet_exists = facets.iter().any(|f| f.facet_type == *facet_type);
                            if !facet_exists {
                                fixed_payloads.push(OperationPayload::AttachFacet {
                                    entity_id: *entity_id,
                                    facet_type: facet_type.clone(),
                                });
                            }
                        }
                        continue;
                    }
                    // Entity doesn't exist or isn't deleted — use original payload
                    fixed_payloads.push(payload.clone());
                }

                OperationPayload::CreateEdge { edge_id, .. } => {
                    // Check if edge exists and is soft-deleted
                    if let Some(edge) = self.storage.get_edge(*edge_id)?
                        && edge.deleted
                    {
                        // Edge exists but is soft-deleted — restore it
                        fixed_payloads.push(OperationPayload::RestoreEdge {
                            edge_id: *edge_id,
                        });
                        continue;
                    }
                    // Edge doesn't exist or isn't deleted — use original payload
                    fixed_payloads.push(payload.clone());
                }

                _ => {
                    fixed_payloads.push(payload.clone());
                }
            }
        }

        // Capture snapshot for the fixed payloads (so this redo can be undone)
        let snapshot = self.undo_manager.capture_snapshot(&self.storage, &fixed_payloads)?;

        // Execute the fixed payloads (not self-undoable — we manage stack manually)
        let (bundle_id, hlc) = self.execute_internal(BundleType::UserEdit, fixed_payloads.clone(), false)?;

        // Push new undo entry so this redo can be undone
        self.undo_manager.push_undo(bundle_id, hlc, fixed_payloads, snapshot);

        Ok(UndoResult::Applied(bundle_id))
    }

    // ========================================================================
    // Query Pass-Through
    // ========================================================================

    pub fn get_entity(&self, entity_id: EntityId) -> Result<Option<EntityRecord>, EngineError> {
        Ok(self.storage.get_entity(entity_id)?)
    }

    pub fn get_fields(&self, entity_id: EntityId) -> Result<Vec<(String, FieldValue)>, EngineError> {
        let mut fields = self.storage.get_fields(entity_id)?;

        // If overlay is active, merge overlay deltas (overlay wins)
        if let Some(overlay_id) = self.overlay_manager.active_overlay_id() {
            let overlay_ops = self.storage.get_overlay_ops(overlay_id)?;
            for (_rowid, _op_id, _hlc, payload_bytes, eid, _op_type, _canon, _drifted, _field_key) in &overlay_ops {
                if eid.as_ref().and_then(|b| <[u8; 16]>::try_from(b.as_slice()).ok().map(EntityId::from_bytes)) == Some(entity_id)
                    && let Ok(payload) = OperationPayload::from_msgpack(payload_bytes)
                {
                    match payload {
                        OperationPayload::SetField { field_key, value, .. } => {
                            // Remove existing entry for this key, then add overlay value
                            fields.retain(|(k, _)| k != &field_key);
                            fields.push((field_key, value));
                        }
                        OperationPayload::ClearField { field_key, .. } => {
                            // Remove from results (cleared in overlay)
                            fields.retain(|(k, _)| k != &field_key);
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(fields)
    }

    pub fn get_field(&self, entity_id: EntityId, field_key: &str) -> Result<Option<FieldValue>, EngineError> {
        // If overlay is active, check overlay first
        if let Some(overlay_id) = self.overlay_manager.active_overlay_id()
            && let Some((_rowid, payload_bytes)) = self.storage.get_latest_overlay_field_op(overlay_id, entity_id, field_key)?
        {
            let payload = OperationPayload::from_msgpack(&payload_bytes)?;
            return match payload {
                OperationPayload::SetField { value, .. } => Ok(Some(value)),
                OperationPayload::ClearField { .. } => Ok(None),
                _ => Ok(self.storage.get_field(entity_id, field_key)?),
            };
        }
        // Fall through to canonical
        Ok(self.storage.get_field(entity_id, field_key)?)
    }

    pub fn get_facets(&self, entity_id: EntityId) -> Result<Vec<FacetRecord>, EngineError> {
        Ok(self.storage.get_facets(entity_id)?)
    }

    pub fn get_entities_by_facet(&self, facet_type: &str) -> Result<Vec<EntityId>, EngineError> {
        Ok(self.storage.get_entities_by_facet(facet_type)?)
    }

    pub fn get_edges_from(&self, entity_id: EntityId) -> Result<Vec<EdgeRecord>, EngineError> {
        Ok(self.storage.get_edges_from(entity_id)?)
    }

    pub fn get_edges_to(&self, entity_id: EntityId) -> Result<Vec<EdgeRecord>, EngineError> {
        Ok(self.storage.get_edges_to(entity_id)?)
    }

    pub fn get_edge(&self, edge_id: EdgeId) -> Result<Option<EdgeRecord>, EngineError> {
        Ok(self.storage.get_edge(edge_id)?)
    }

    pub fn get_edge_properties(
        &self,
        edge_id: EdgeId,
    ) -> Result<Vec<(String, FieldValue)>, EngineError> {
        Ok(self.storage.get_edge_properties(edge_id)?)
    }

    pub fn get_edge_property(
        &self,
        edge_id: EdgeId,
        key: &str,
    ) -> Result<Option<FieldValue>, EngineError> {
        Ok(self.storage.get_edge_property(edge_id, key)?)
    }

    pub fn get_edge_property_metadata(
        &self,
        edge_id: EdgeId,
        key: &str,
    ) -> Result<Option<(ActorId, Hlc)>, EngineError> {
        Ok(self.storage.get_edge_property_metadata(edge_id, key)?)
    }

    pub fn get_vector_clock(&self) -> Result<VectorClock, EngineError> {
        Ok(self.storage.get_vector_clock()?)
    }

    pub fn get_ops_canonical(&self) -> Result<Vec<Operation>, EngineError> {
        Ok(self.storage.get_ops_canonical()?)
    }

    pub fn get_ops_by_bundle(&self, bundle_id: BundleId) -> Result<Vec<Operation>, EngineError> {
        Ok(self.storage.get_ops_by_bundle(bundle_id)?)
    }

    pub fn get_ops_by_actor_after(
        &self,
        actor_id: ActorId,
        after: Hlc,
    ) -> Result<Vec<Operation>, EngineError> {
        Ok(self.storage.get_ops_by_actor_after(actor_id, after)?)
    }

    pub fn op_count(&self) -> Result<u64, EngineError> {
        Ok(self.storage.op_count()?)
    }

    pub fn get_field_metadata(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<(ActorId, Hlc)>, EngineError> {
        Ok(self.storage.get_field_metadata(entity_id, field_key)?)
    }

    // ========================================================================
    // Ingest (Sync / Testing)
    // ========================================================================

    /// Ingest a foreign bundle and its operations into this engine's storage.
    /// Used for sync and testing — does NOT push to undo stack.
    /// Detects field-level conflicts via vector clock comparison.
    /// Returns any detected conflicts.
    pub fn ingest_bundle(
        &mut self,
        bundle: &Bundle,
        operations: &[Operation],
    ) -> Result<Vec<ConflictRecord>, EngineError> {
        self.exec_batch("BEGIN IMMEDIATE")?;

        let result = (|| -> Result<Vec<ConflictRecord>, EngineError> {
            // 1. Snapshot field metadata for all SetField/ClearField ops BEFORE materialization
            let pre_snapshots = self.snapshot_field_metadata(operations)?;

            // 2. Append bundle (materializes ops via SAVEPOINT, nests correctly)
            self.storage.append_bundle(bundle, operations)?;

            // 3. Detect conflicts using pre-materialization snapshots
            let conflicts = self.detect_conflicts(bundle, operations, &pre_snapshots)?;

            // 4. Scan for overlay drift on modified fields
            let modified_fields: Vec<(EntityId, String)> = operations.iter().filter_map(|op| {
                match &op.payload {
                    OperationPayload::SetField { entity_id, field_key, .. }
                    | OperationPayload::ClearField { entity_id, field_key } => {
                        Some((*entity_id, field_key.clone()))
                    }
                    _ => None,
                }
            }).collect();
            self.scan_overlay_drift(&modified_fields)?;

            Ok(conflicts)
        })();

        match result {
            Ok(conflicts) => {
                self.exec_batch("COMMIT")?;
                Ok(conflicts)
            }
            Err(e) => {
                let _ = self.exec_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Pre-materialization snapshot of field metadata for conflict detection.
    fn snapshot_field_metadata(
        &self,
        operations: &[Operation],
    ) -> Result<Vec<FieldMetadataSnapshot>, EngineError> {
        let mut snapshots = Vec::new();
        for op in operations {
            match &op.payload {
                OperationPayload::SetField { entity_id, field_key, value } => {
                    let current = self.storage.get_field_source_bundle_vc(*entity_id, field_key)?;
                    let value_bytes = value.to_msgpack()
                        .map_err(|e| EngineError::Core(openprod_core::CoreError::Serialization(e.to_string())))?;
                    snapshots.push(FieldMetadataSnapshot {
                        entity_id: *entity_id,
                        field_key: field_key.clone(),
                        current_actor: current.as_ref().map(|(a, _, _, _)| *a),
                        current_hlc: current.as_ref().map(|(_, h, _, _)| *h),
                        current_op_id: current.as_ref().map(|(_, _, o, _)| *o),
                        current_bundle_vc: current.and_then(|(_, _, _, vc)| vc),
                        ingested_op_id: op.op_id,
                        ingested_value: Some(value_bytes),
                    });
                }
                OperationPayload::ClearField { entity_id, field_key } => {
                    let current = self.storage.get_field_source_bundle_vc(*entity_id, field_key)?;
                    snapshots.push(FieldMetadataSnapshot {
                        entity_id: *entity_id,
                        field_key: field_key.clone(),
                        current_actor: current.as_ref().map(|(a, _, _, _)| *a),
                        current_hlc: current.as_ref().map(|(_, h, _, _)| *h),
                        current_op_id: current.as_ref().map(|(_, _, o, _)| *o),
                        current_bundle_vc: current.and_then(|(_, _, _, vc)| vc),
                        ingested_op_id: op.op_id,
                        ingested_value: None,
                    });
                }
                _ => {}
            }
        }
        Ok(snapshots)
    }

    /// Detect field-level conflicts by comparing the ingested bundle's vector clock
    /// against the pre-materialization field state.
    fn detect_conflicts(
        &mut self,
        bundle: &Bundle,
        operations: &[Operation],
        pre_snapshots: &[FieldMetadataSnapshot],
    ) -> Result<Vec<ConflictRecord>, EngineError> {
        let ingested_actor = bundle.actor_id;
        let ingested_vc = bundle.creator_vc.as_ref();

        let mut conflicts = Vec::new();

        for snap in pre_snapshots {
            // 1. No prior value → no conflict
            let current_actor = match snap.current_actor {
                Some(a) => a,
                None => continue,
            };
            let current_hlc = snap.current_hlc.unwrap(); // safe: actor implies hlc
            let current_op_id = snap.current_op_id.unwrap();

            // 2. Same actor → no conflict
            if current_actor == ingested_actor {
                continue;
            }

            // Find the ingested op's HLC
            let ingested_op = operations.iter().find(|o| o.op_id == snap.ingested_op_id);
            let ingested_hlc = match ingested_op {
                Some(op) => op.hlc,
                None => continue,
            };

            // 3. Did ingested actor know about the current value?
            //    creator_vc.get(current_actor) >= current_hlc?
            if let Some(vc) = ingested_vc
                && let Some(known_hlc) = vc.get(&current_actor)
                && *known_hlc >= current_hlc
            {
                continue; // ingested saw the current value → not concurrent
            }

            // 4. Did the current writer know about the ingested actor?
            //    current_bundle_vc.get(ingested_actor) >= ingested_hlc?
            if let Some(ref current_vc) = snap.current_bundle_vc
                && let Some(known_hlc) = current_vc.get(&ingested_actor)
                && *known_hlc >= ingested_hlc
            {
                continue; // current writer saw the ingested actor → not concurrent
            }

            // Both didn't see each other → CONFLICT
            // Check for existing conflict on this (entity, field) — open or resolved
            let existing = self.storage.get_latest_conflict_for_field(snap.entity_id, &snap.field_key)?;

            // Get the current field's value bytes for the conflict record
            let current_value_bytes: Option<Vec<u8>> = {
                self.get_field_value_from_oplog(current_op_id)?
            };

            let incoming_tip = ConflictValue {
                value: snap.ingested_value.clone(),
                actor_id: ingested_actor,
                hlc: ingested_hlc,
                op_id: snap.ingested_op_id,
            };

            if let Some(existing) = existing {
                if existing.status == ConflictStatus::Resolved {
                    // Resolved conflict being reopened by a new concurrent edit.
                    // Build fresh branch tips from resolution + late-arriving edit.
                    let resolution_tip = ConflictValue {
                        value: existing.resolved_value.clone(),
                        actor_id: existing.resolved_by.unwrap(),
                        hlc: existing.resolved_at.unwrap(),
                        op_id: existing.resolved_op_id.unwrap(),
                    };
                    self.storage.reopen_conflict(
                        existing.conflict_id,
                        ingested_hlc,
                        snap.ingested_op_id,
                        &[resolution_tip, incoming_tip],
                    )?;
                    conflicts.push(self.storage.get_conflict(existing.conflict_id)?.unwrap());
                } else {
                    // Already open — extend to N-way by adding the new branch tip
                    self.storage.add_conflict_value(existing.conflict_id, &incoming_tip)?;
                    conflicts.push(self.storage.get_conflict(existing.conflict_id)?.unwrap());
                }
                continue;
            }

            // Create new conflict
            let conflict_id = ConflictId::new();
            let record = ConflictRecord {
                conflict_id,
                entity_id: snap.entity_id,
                field_key: snap.field_key.clone(),
                status: ConflictStatus::Open,
                values: vec![
                    ConflictValue {
                        value: current_value_bytes,
                        actor_id: current_actor,
                        hlc: current_hlc,
                        op_id: current_op_id,
                    },
                    incoming_tip,
                ],
                detected_at: ingested_hlc,
                detected_in_bundle: bundle.bundle_id,
                resolved_at: None,
                resolved_by: None,
                resolved_op_id: None,
                resolved_value: None,
                reopened_at: None,
                reopened_by_op: None,
            };
            self.storage.insert_conflict(&record)?;
            conflicts.push(record);
        }

        Ok(conflicts)
    }

    /// Extract a field value from an oplog operation by op_id.
    fn get_field_value_from_oplog(&self, op_id: OpId) -> Result<Option<Vec<u8>>, EngineError> {
        Ok(self.storage.get_op_field_value(op_id)?)
    }

    // ========================================================================
    // Conflict Resolution
    // ========================================================================

    /// Resolve a conflict by choosing a value.
    /// `chosen_value: None` means resolve to cleared (tombstone).
    /// Resolution is NOT undoable per spec.
    pub fn resolve_conflict(
        &mut self,
        conflict_id: ConflictId,
        chosen_value: Option<FieldValue>,
    ) -> Result<BundleId, EngineError> {
        // Load conflict
        let conflict = self.storage.get_conflict(conflict_id)?
            .ok_or_else(|| EngineError::ConflictNotFound(conflict_id.to_string()))?;

        if conflict.status != ConflictStatus::Open {
            return Err(EngineError::ConflictAlreadyResolved(conflict_id.to_string()));
        }

        self.exec_batch("BEGIN IMMEDIATE")?;

        let result = (|| -> Result<BundleId, EngineError> {
            // Create ResolveConflict operation payload
            let payloads = vec![OperationPayload::ResolveConflict {
                conflict_id,
                entity_id: conflict.entity_id,
                field_key: conflict.field_key.clone(),
                chosen_value: chosen_value.clone(),
            }];

            // Execute as non-undoable
            let (bundle_id, hlc) = self.execute_internal(BundleType::UserEdit, payloads, false)?;

            // Update conflict record to resolved
            let resolved_value_bytes = match &chosen_value {
                Some(v) => Some(v.to_msgpack()
                    .map_err(|e| EngineError::Core(openprod_core::CoreError::Serialization(e.to_string())))?),
                None => None,
            };
            // Get the op_id from the bundle we just created
            let ops = self.storage.get_ops_by_bundle(bundle_id)?;
            let resolve_op_id = ops.first().map(|o| o.op_id)
                .ok_or_else(|| EngineError::ConflictNotFound("no ops in resolve bundle".into()))?;

            self.storage.update_conflict_resolved(
                conflict_id,
                hlc,
                self.identity.actor_id(),
                resolve_op_id,
                resolved_value_bytes,
            )?;

            Ok(bundle_id)
        })();

        match result {
            Ok(bundle_id) => {
                self.exec_batch("COMMIT")?;
                Ok(bundle_id)
            }
            Err(e) => {
                let _ = self.exec_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // ========================================================================
    // Conflict Queries
    // ========================================================================

    pub fn get_open_conflicts_for_entity(
        &self,
        entity_id: EntityId,
    ) -> Result<Vec<ConflictRecord>, EngineError> {
        Ok(self.storage.get_open_conflicts_for_entity(entity_id)?)
    }

    pub fn get_conflict(
        &self,
        conflict_id: ConflictId,
    ) -> Result<Option<ConflictRecord>, EngineError> {
        Ok(self.storage.get_conflict(conflict_id)?)
    }

    // ========================================================================
    // State Rebuild
    // ========================================================================

    /// Rebuild materialized state from the oplog. Returns the number of operations replayed.
    pub fn rebuild_state(&mut self) -> Result<u64, EngineError> {
        Ok(self.storage.rebuild_from_oplog()?)
    }

    // ========================================================================
    // Overlay Lifecycle
    // ========================================================================

    /// Create a new overlay and make it active.
    /// If another overlay is currently active, it is auto-stashed.
    pub fn create_overlay(&mut self, name: &str) -> Result<OverlayId, EngineError> {
        // Auto-stash current active overlay
        if let Some(current) = self.overlay_manager.active_overlay_id() {
            self.stash_overlay(current)?;
        }

        let overlay_id = OverlayId::new();
        let hlc = self.clock.tick()?;
        self.storage.insert_overlay(
            overlay_id,
            name,
            OverlaySource::User.as_str(),
            OverlayStatus::Active.as_str(),
            &hlc,
        )?;
        self.overlay_manager.set_active(Some(overlay_id));
        Ok(overlay_id)
    }

    /// Activate an existing overlay (must be stashed).
    /// If another overlay is currently active, it is auto-stashed.
    pub fn activate_overlay(&mut self, overlay_id: OverlayId) -> Result<(), EngineError> {
        let overlay = self.storage.get_overlay(overlay_id)?
            .ok_or_else(|| EngineError::OverlayNotFound(overlay_id.to_string()))?;
        let (_id, _name, _source, status, _created, _updated) = overlay;
        if status != OverlayStatus::Stashed.as_str() {
            return Err(EngineError::OverlayNotFound(
                format!("overlay {} is not stashed (status: {})", overlay_id, status),
            ));
        }

        // Auto-stash current active overlay
        if let Some(current) = self.overlay_manager.active_overlay_id() {
            self.stash_overlay(current)?;
        }

        let hlc = self.clock.tick()?;
        self.storage.update_overlay_status(overlay_id, OverlayStatus::Active.as_str(), &hlc)?;
        self.overlay_manager.set_active(Some(overlay_id));
        Ok(())
    }

    /// Stash an overlay (deactivate without discarding).
    pub fn stash_overlay(&mut self, overlay_id: OverlayId) -> Result<(), EngineError> {
        let hlc = self.clock.tick()?;
        self.storage.update_overlay_status(overlay_id, OverlayStatus::Stashed.as_str(), &hlc)?;
        if self.overlay_manager.active_overlay_id() == Some(overlay_id) {
            self.overlay_manager.set_active(None);
        }
        Ok(())
    }

    /// Discard an overlay — removes all overlay ops and the overlay record.
    pub fn discard_overlay(&mut self, overlay_id: OverlayId) -> Result<(), EngineError> {
        self.storage.delete_overlay(overlay_id)?;
        if self.overlay_manager.active_overlay_id() == Some(overlay_id) {
            self.overlay_manager.set_active(None);
        }
        Ok(())
    }

    /// Get the currently active overlay ID, if any.
    pub fn active_overlay(&self) -> Option<OverlayId> {
        self.overlay_manager.active_overlay_id()
    }

    /// List stashed overlays.
    pub fn stashed_overlays(&self) -> Result<Vec<(OverlayId, String)>, EngineError> {
        let raw = self.storage.list_overlays_by_status(OverlayStatus::Stashed.as_str())?;
        Ok(raw.into_iter().map(|(id, name, _source, _created)| (id, name)).collect())
    }

    /// Undo the most recent operation in the active overlay.
    /// Removes the op from overlay_ops and pushes to overlay redo stack.
    pub fn overlay_undo(&mut self) -> Result<bool, EngineError> {
        let overlay_id = self.overlay_manager.active_overlay_id()
            .ok_or(EngineError::NoActiveOverlay)?;

        let op = match self.overlay_manager.pop_overlay_undo() {
            Some(op) => op,
            None => return Ok(false),
        };

        self.storage.delete_overlay_op(op.rowid)?;
        self.overlay_manager.push_overlay_redo(op);
        // Verify overlay_id matches (should always be true for active overlay)
        let _ = overlay_id;
        Ok(true)
    }

    /// Redo the most recently undone overlay operation.
    /// Re-inserts the op into overlay_ops.
    pub fn overlay_redo(&mut self) -> Result<bool, EngineError> {
        let overlay_id = self.overlay_manager.active_overlay_id()
            .ok_or(EngineError::NoActiveOverlay)?;

        let mut op = match self.overlay_manager.pop_overlay_redo() {
            Some(op) => op,
            None => return Ok(false),
        };

        let payload_bytes = op.payload.to_msgpack()?;
        let rowid = self.storage.insert_overlay_op(
            overlay_id,
            op.op_id,
            &op.hlc,
            &payload_bytes,
            op.entity_id,
            op.field_key.as_deref(),
            &op.op_type,
            op.canonical_value_at_creation.as_deref(),
        )?;
        op.rowid = rowid;
        self.overlay_manager.push_overlay_undo(op);
        Ok(true)
    }

    // ========================================================================
    // Overlay Commit & Canonical Drift
    // ========================================================================

    /// Scan all active/stashed overlays for drift on the given modified fields.
    /// Called after canonical state changes (ingest_bundle, commit_overlay).
    fn scan_overlay_drift(&mut self, modified_fields: &[(EntityId, String)]) -> Result<(), EngineError> {
        for (entity_id, _field_key) in modified_fields {
            self.storage.mark_overlay_ops_drifted(*entity_id, _field_key)?;
        }
        Ok(())
    }

    /// Commit an overlay — atomically move all overlay ops to canonical storage.
    /// Returns the BundleId of the committed bundle.
    /// Fails if there is unresolved drift.
    pub fn commit_overlay(&mut self, overlay_id: OverlayId) -> Result<BundleId, EngineError> {
        // Check for unresolved drift
        let drift_count = self.storage.count_unresolved_drift(overlay_id)?;
        if drift_count > 0 {
            return Err(EngineError::UnresolvedDrift(
                format!("{} drifted field(s) on overlay {}", drift_count, overlay_id),
            ));
        }

        // Read all overlay ops ordered by rowid
        let overlay_ops = self.storage.get_overlay_ops(overlay_id)?;
        if overlay_ops.is_empty() {
            // Empty overlay — just discard
            self.discard_overlay(overlay_id)?;
            return Err(EngineError::EmptyOverlay(
                format!("overlay {} has no ops to commit", overlay_id),
            ));
        }

        // Deserialize payloads
        let mut payloads = Vec::new();
        for (_rowid, _op_id, _hlc, payload_bytes, _entity_id, _op_type, _canon, _drifted, _field_key) in &overlay_ops {
            let payload = OperationPayload::from_msgpack(payload_bytes)?;
            payloads.push(payload);
        }

        // Collect modified fields for drift scanning
        let modified_fields: Vec<(EntityId, String)> = payloads.iter().filter_map(|p| {
            match p {
                OperationPayload::SetField { entity_id, field_key, .. }
                | OperationPayload::ClearField { entity_id, field_key } => {
                    Some((*entity_id, field_key.clone()))
                }
                _ => None,
            }
        }).collect();

        // Deactivate overlay to avoid routing the execute_internal call back to overlay
        if self.overlay_manager.active_overlay_id() == Some(overlay_id) {
            self.overlay_manager.set_active(None);
        }

        // Wrap commit in transaction for atomicity
        self.exec_batch("BEGIN IMMEDIATE")?;

        let result = (|| -> Result<BundleId, EngineError> {
            // Execute as canonical (non-undoable)
            let (bundle_id, _hlc) = self.execute_internal(BundleType::UserEdit, payloads, false)?;

            // Update overlay status to committed
            let hlc = self.clock.tick()?;
            self.storage.update_overlay_status(overlay_id, OverlayStatus::Committed.as_str(), &hlc)?;

            // Scan for drift on stashed overlays
            self.scan_overlay_drift(&modified_fields)?;

            Ok(bundle_id)
        })();

        match result {
            Ok(bundle_id) => {
                self.exec_batch("COMMIT")?;
                Ok(bundle_id)
            }
            Err(e) => {
                let _ = self.exec_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Check for drifted fields on an overlay.
    /// Returns a list of DriftRecord entries showing overlay vs canonical values.
    pub fn check_drift(&self, overlay_id: OverlayId) -> Result<Vec<DriftRecord>, EngineError> {
        let drifted_ops = self.storage.get_drifted_overlay_ops(overlay_id)?;
        let mut records = Vec::new();

        for (_rowid, _op_id, _hlc, payload_bytes, _entity_id_bytes, _op_type, _canon_bytes, _drifted, _field_key) in &drifted_ops {
            let payload = OperationPayload::from_msgpack(payload_bytes)?;
            match payload {
                OperationPayload::SetField { entity_id, field_key, value, .. } => {
                    let canonical_value = self.storage.get_field(entity_id, &field_key)?;
                    records.push(DriftRecord {
                        entity_id,
                        field_key,
                        overlay_value: Some(value),
                        canonical_value,
                    });
                }
                OperationPayload::ClearField { entity_id, field_key } => {
                    let canonical_value = self.storage.get_field(entity_id, &field_key)?;
                    records.push(DriftRecord {
                        entity_id,
                        field_key,
                        overlay_value: None,
                        canonical_value,
                    });
                }
                _ => {}
            }
        }

        Ok(records)
    }

    /// Acknowledge drift on a field — "Keep Mine".
    /// Clears the drift flag and updates canonical_value_at_creation to new canonical value.
    pub fn acknowledge_drift(
        &mut self,
        overlay_id: OverlayId,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<(), EngineError> {
        // Get current canonical value for this field
        let canonical_value = match self.storage.get_field(entity_id, field_key)? {
            Some(v) => {
                let bytes = v.to_msgpack()
                    .map_err(|e| EngineError::Core(openprod_core::CoreError::Serialization(e.to_string())))?;
                Some(bytes)
            }
            None => None,
        };

        self.storage.update_canonical_value_at_creation(overlay_id, entity_id, field_key, canonical_value.as_deref())?;
        self.storage.clear_drift_flag(overlay_id, entity_id, field_key)?;
        Ok(())
    }

    /// Knockout a field from the overlay — "Use Canonical".
    /// Removes the overlay op for this field, so it falls through to canonical.
    pub fn knockout_field(
        &mut self,
        overlay_id: OverlayId,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<(), EngineError> {
        self.storage.delete_overlay_ops_for_field(overlay_id, entity_id, field_key)?;
        Ok(())
    }

    /// Check if an overlay has any unresolved drift.
    pub fn has_unresolved_drift(&self, overlay_id: OverlayId) -> Result<bool, EngineError> {
        Ok(self.storage.count_unresolved_drift(overlay_id)? > 0)
    }
}

/// Pre-materialization snapshot of a field's metadata for conflict detection.
struct FieldMetadataSnapshot {
    entity_id: EntityId,
    field_key: String,
    current_actor: Option<ActorId>,
    current_hlc: Option<Hlc>,
    current_op_id: Option<OpId>,
    current_bundle_vc: Option<VectorClock>,
    ingested_op_id: OpId,
    ingested_value: Option<Vec<u8>>,
}
