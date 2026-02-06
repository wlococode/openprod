use openprod_core::{
    field_value::FieldValue,
    identity::ActorIdentity,
    ids::*,
    operations::*,
};
use openprod_engine::Engine;
use openprod_storage::{SqliteStorage, StorageError};

pub struct TestPeer {
    pub engine: Engine,
}

impl TestPeer {
    pub fn new() -> Result<Self, StorageError> {
        let identity = ActorIdentity::generate();
        let storage = SqliteStorage::open_in_memory()?;
        Ok(Self {
            engine: Engine::new(identity, storage),
        })
    }

    pub fn actor_id(&self) -> ActorId {
        self.engine.actor_id()
    }

    pub fn identity(&self) -> &ActorIdentity {
        self.engine.identity()
    }

    /// Execute a list of operation payloads as a single bundle.
    pub fn execute_bundle(
        &mut self,
        bundle_type: BundleType,
        payloads: Vec<OperationPayload>,
    ) -> Result<BundleId, Box<dyn std::error::Error>> {
        let bundle_id = self.engine.execute(bundle_type, payloads)?;
        Ok(bundle_id)
    }

    /// Create an entity with a facet and optional fields.
    pub fn create_record(
        &mut self,
        facet_type: &str,
        fields: Vec<(&str, FieldValue)>,
    ) -> Result<EntityId, Box<dyn std::error::Error>> {
        let (entity_id, _) = self.engine.create_entity_with_fields(facet_type, fields)?;
        Ok(entity_id)
    }

    /// Set a field on an entity.
    pub fn set_field(
        &mut self,
        entity_id: EntityId,
        field_key: &str,
        value: FieldValue,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.set_field(entity_id, field_key, value)?;
        Ok(())
    }

    /// Clear a field on an entity.
    pub fn clear_field(
        &mut self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.clear_field(entity_id, field_key)?;
        Ok(())
    }

    /// Delete an entity. Engine computes cascade_edges automatically.
    pub fn delete_entity(
        &mut self,
        entity_id: EntityId,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.delete_entity(entity_id)?;
        Ok(())
    }

    /// Create an edge between two entities.
    pub fn create_edge(
        &mut self,
        edge_type: &str,
        source_id: EntityId,
        target_id: EntityId,
    ) -> Result<EdgeId, Box<dyn std::error::Error>> {
        let (edge_id, _) = self.engine.create_edge(edge_type, source_id, target_id)?;
        Ok(edge_id)
    }

    /// Delete an edge.
    pub fn delete_edge(&mut self, edge_id: EdgeId) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.delete_edge(edge_id)?;
        Ok(())
    }

    /// Create an edge with initial properties.
    pub fn create_edge_with_properties(
        &mut self,
        edge_type: &str,
        source_id: EntityId,
        target_id: EntityId,
        properties: Vec<(&str, FieldValue)>,
    ) -> Result<EdgeId, Box<dyn std::error::Error>> {
        let (edge_id, _) = self.engine.create_edge_with_properties(edge_type, source_id, target_id, properties)?;
        Ok(edge_id)
    }

    /// Set a property on an edge.
    pub fn set_edge_property(
        &mut self,
        edge_id: EdgeId,
        property_key: &str,
        value: FieldValue,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.set_edge_property(edge_id, property_key, value)?;
        Ok(())
    }

    /// Clear a property on an edge.
    pub fn clear_edge_property(
        &mut self,
        edge_id: EdgeId,
        property_key: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.clear_edge_property(edge_id, property_key)?;
        Ok(())
    }

    /// Detach a facet from an entity.
    pub fn detach_facet(
        &mut self,
        entity_id: EntityId,
        facet_type: &str,
        preserve: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.detach_facet(entity_id, facet_type, preserve)?;
        Ok(())
    }

    // Overlay convenience methods

    /// Create a new overlay and make it active.
    pub fn create_overlay(&mut self, name: &str) -> Result<OverlayId, Box<dyn std::error::Error>> {
        Ok(self.engine.create_overlay(name)?)
    }

    /// Commit an overlay to canonical storage.
    pub fn commit_overlay(&mut self, overlay_id: OverlayId) -> Result<BundleId, Box<dyn std::error::Error>> {
        Ok(self.engine.commit_overlay(overlay_id)?)
    }

    /// Discard an overlay and all its ops.
    pub fn discard_overlay(&mut self, overlay_id: OverlayId) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.discard_overlay(overlay_id)?;
        Ok(())
    }

    /// Stash an overlay.
    pub fn stash_overlay(&mut self, overlay_id: OverlayId) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.stash_overlay(overlay_id)?;
        Ok(())
    }

    /// Check for drifted fields on an overlay.
    pub fn check_drift(&self, overlay_id: OverlayId) -> Result<Vec<openprod_engine::DriftRecord>, Box<dyn std::error::Error>> {
        Ok(self.engine.check_drift(overlay_id)?)
    }

    /// Acknowledge drift on a field ("Keep Mine").
    pub fn acknowledge_drift(
        &mut self,
        overlay_id: OverlayId,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.acknowledge_drift(overlay_id, entity_id, field_key)?;
        Ok(())
    }

    /// Knockout a field from an overlay ("Use Canonical").
    pub fn knockout_field(
        &mut self,
        overlay_id: OverlayId,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.knockout_field(overlay_id, entity_id, field_key)?;
        Ok(())
    }

    // Conflict convenience methods

    /// Get open conflicts for an entity.
    pub fn get_open_conflicts(
        &self,
        entity_id: EntityId,
    ) -> Result<Vec<openprod_storage::ConflictRecord>, Box<dyn std::error::Error>> {
        Ok(self.engine.get_open_conflicts_for_entity(entity_id)?)
    }

    /// Resolve a conflict with a chosen value (None = clear field).
    pub fn resolve_conflict(
        &mut self,
        conflict_id: ConflictId,
        chosen_value: Option<FieldValue>,
    ) -> Result<BundleId, Box<dyn std::error::Error>> {
        Ok(self.engine.resolve_conflict(conflict_id, chosen_value)?)
    }
}
