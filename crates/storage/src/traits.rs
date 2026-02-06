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
    pub created_at: Hlc,
    pub created_by: ActorId,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStatus {
    Open,
    Resolved,
}

impl ConflictStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
        }
    }

    pub fn parse(s: &str) -> Result<Self, crate::error::StorageError> {
        match s {
            "open" => Ok(Self::Open),
            "resolved" => Ok(Self::Resolved),
            _ => Err(crate::error::StorageError::Serialization(format!("unknown conflict status: {s}"))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConflictValue {
    pub value: Option<Vec<u8>>,
    pub actor_id: ActorId,
    pub hlc: Hlc,
    pub op_id: OpId,
}

#[derive(Debug, Clone)]
pub struct ConflictRecord {
    pub conflict_id: ConflictId,
    pub entity_id: EntityId,
    pub field_key: String,
    pub status: ConflictStatus,
    pub values: Vec<ConflictValue>,
    pub detected_at: Hlc,
    pub detected_in_bundle: BundleId,
    pub resolved_at: Option<Hlc>,
    pub resolved_by: Option<ActorId>,
    pub resolved_op_id: Option<OpId>,
    pub resolved_value: Option<Vec<u8>>,
    pub reopened_at: Option<Hlc>,
    pub reopened_by_op: Option<OpId>,
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

    fn get_field_metadata(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<(ActorId, Hlc)>, StorageError>;

    fn get_edge(&self, edge_id: EdgeId) -> Result<Option<EdgeRecord>, StorageError>;

    fn get_edge_properties(
        &self,
        edge_id: EdgeId,
    ) -> Result<Vec<(String, FieldValue)>, StorageError>;

    fn get_edge_property(
        &self,
        edge_id: EdgeId,
        key: &str,
    ) -> Result<Option<FieldValue>, StorageError>;

    fn get_edge_property_metadata(
        &self,
        edge_id: EdgeId,
        key: &str,
    ) -> Result<Option<(ActorId, Hlc)>, StorageError>;

    fn insert_conflict(&mut self, record: &ConflictRecord) -> Result<(), StorageError>;

    fn update_conflict_resolved(
        &mut self,
        conflict_id: ConflictId,
        resolved_at: Hlc,
        resolved_by: ActorId,
        resolved_op: OpId,
        resolved_value: Option<Vec<u8>>,
    ) -> Result<(), StorageError>;

    fn get_open_conflicts_for_entity(
        &self,
        entity_id: EntityId,
    ) -> Result<Vec<ConflictRecord>, StorageError>;

    fn get_conflict(
        &self,
        conflict_id: ConflictId,
    ) -> Result<Option<ConflictRecord>, StorageError>;

    fn get_open_conflict_for_field(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<ConflictRecord>, StorageError>;

    fn get_latest_conflict_for_field(
        &self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<Option<ConflictRecord>, StorageError>;

    fn reopen_conflict(
        &mut self,
        conflict_id: ConflictId,
        reopened_at: Hlc,
        reopened_by_op: OpId,
        new_values: &[ConflictValue],
    ) -> Result<(), StorageError>;

    fn add_conflict_value(
        &mut self,
        conflict_id: ConflictId,
        value: &ConflictValue,
    ) -> Result<(), StorageError>;

    fn get_bundle_vector_clock(
        &self,
        bundle_id: BundleId,
    ) -> Result<Option<VectorClock>, StorageError>;
}
