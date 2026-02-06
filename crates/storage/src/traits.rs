use openprod_core::{
    field_value::FieldValue,
    hlc::Hlc,
    ids::*,
    operations::{Bundle, Operation},
    vector_clock::VectorClock,
};

use crate::error::StorageError;

#[derive(Debug, Clone)]
pub struct EntityRecord {
    pub entity_id: EntityId,
    pub created_at: Hlc,
    pub created_by: ActorId,
    pub deleted: bool,
}

#[derive(Debug, Clone)]
pub struct FacetRecord {
    pub entity_id: EntityId,
    pub facet_type: String,
    pub attached_at: Hlc,
    pub attached_by: ActorId,
    pub detached: bool,
}

#[derive(Debug, Clone)]
pub struct EdgeRecord {
    pub edge_id: EdgeId,
    pub edge_type: String,
    pub source_id: EntityId,
    pub target_id: EntityId,
    pub properties: Vec<u8>,
    pub created_at: Hlc,
    pub created_by: ActorId,
    pub deleted: bool,
}

pub trait Storage {
    fn append_bundle(
        &mut self,
        bundle: &Bundle,
        operations: &[Operation],
    ) -> Result<(), StorageError>;

    fn get_ops_canonical(&self) -> Result<Vec<Operation>, StorageError>;

    fn get_ops_by_bundle(&self, bundle_id: BundleId) -> Result<Vec<Operation>, StorageError>;

    fn get_ops_by_actor_after(
        &self,
        actor_id: ActorId,
        after: Hlc,
    ) -> Result<Vec<Operation>, StorageError>;

    fn op_count(&self) -> Result<u64, StorageError>;

    fn get_entity(&self, entity_id: EntityId) -> Result<Option<EntityRecord>, StorageError>;

    fn get_fields(
        &self,
        entity_id: EntityId,
    ) -> Result<Vec<(String, FieldValue)>, StorageError>;

    fn get_field(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<FieldValue>, StorageError>;

    fn get_facets(&self, entity_id: EntityId) -> Result<Vec<FacetRecord>, StorageError>;

    fn get_entities_by_facet(&self, facet_type: &str) -> Result<Vec<EntityId>, StorageError>;

    fn get_edges_from(&self, entity_id: EntityId) -> Result<Vec<EdgeRecord>, StorageError>;

    fn get_edges_to(&self, entity_id: EntityId) -> Result<Vec<EdgeRecord>, StorageError>;

    fn get_vector_clock(&self) -> Result<VectorClock, StorageError>;
}
