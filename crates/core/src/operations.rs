use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::CoreError;
use crate::field_value::FieldValue;
use crate::hlc::Hlc;
use crate::identity::{verify_signature, ActorIdentity};
use crate::ids::*;
use crate::vector_clock::VectorClock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrdtType {
    Text,
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationPayload {
    CreateEntity {
        entity_id: EntityId,
        initial_table: Option<String>,
    },
    DeleteEntity {
        entity_id: EntityId,
        cascade_edges: Vec<EdgeId>,
    },
    AttachFacet {
        entity_id: EntityId,
        facet_type: String,
    },
    DetachFacet {
        entity_id: EntityId,
        facet_type: String,
        preserve_values: bool,
    },
    RestoreFacet {
        entity_id: EntityId,
        facet_type: String,
    },
    SetField {
        entity_id: EntityId,
        field_key: String,
        value: FieldValue,
    },
    ClearField {
        entity_id: EntityId,
        field_key: String,
    },
    ApplyCrdt {
        entity_id: EntityId,
        field_key: String,
        crdt_type: CrdtType,
        delta: Vec<u8>,
    },
    ClearAndAdd {
        entity_id: EntityId,
        field_key: String,
        values: Vec<FieldValue>,
    },
    CreateEdge {
        edge_id: EdgeId,
        edge_type: String,
        source_id: EntityId,
        target_id: EntityId,
        properties: Vec<(String, FieldValue)>,
    },
    DeleteEdge {
        edge_id: EdgeId,
    },
    SetEdgeProperty {
        edge_id: EdgeId,
        property_key: String,
        value: FieldValue,
    },
    ClearEdgeProperty {
        edge_id: EdgeId,
        property_key: String,
    },
    CreateOrderedEdge {
        edge_id: EdgeId,
        edge_type: String,
        source_id: EntityId,
        target_id: EntityId,
        after: Option<EdgeId>,
        before: Option<EdgeId>,
        properties: Vec<(String, FieldValue)>,
    },
    MoveOrderedEdge {
        edge_id: EdgeId,
        after: Option<EdgeId>,
        before: Option<EdgeId>,
    },
    LinkTables {
        source_table: TableId,
        target_table: TableId,
        field_mappings: Vec<(String, String)>,
    },
    UnlinkTables {
        source_table: TableId,
        target_table: TableId,
        data_handling: String,
    },
    AddToTable {
        entity_id: EntityId,
        table: String,
        defaults: Vec<(String, FieldValue)>,
    },
    RemoveFromTable {
        entity_id: EntityId,
        table: String,
        data_handling: String,
    },
    ConfirmFieldMapping {
        source_table: TableId,
        target_table: TableId,
        source_field: String,
        target_field: String,
    },
    MergeEntities {
        survivor: EntityId,
        absorbed: EntityId,
    },
    SplitEntity {
        source: EntityId,
        new_entity: EntityId,
        facets: Vec<String>,
    },
    CreateRule {
        rule_id: RuleId,
        name: String,
        when_clause: String,
        action_type: String,
        action_params: Vec<u8>,
        auto_accept: bool,
    },
    RestoreEntity {
        entity_id: EntityId,
    },
    RestoreEdge {
        edge_id: EdgeId,
    },
    ResolveConflict {
        conflict_id: ConflictId,
        entity_id: EntityId,
        field_key: String,
        chosen_value: Option<FieldValue>,
    },
}

impl OperationPayload {
    /// Extract the primary entity_id this operation targets (if any).
    pub fn entity_id(&self) -> Option<EntityId> {
        match self {
            Self::CreateEntity { entity_id, .. }
            | Self::DeleteEntity { entity_id, .. }
            | Self::AttachFacet { entity_id, .. }
            | Self::DetachFacet { entity_id, .. }
            | Self::RestoreFacet { entity_id, .. }
            | Self::SetField { entity_id, .. }
            | Self::ClearField { entity_id, .. }
            | Self::ApplyCrdt { entity_id, .. }
            | Self::ClearAndAdd { entity_id, .. }
            | Self::AddToTable { entity_id, .. }
            | Self::RemoveFromTable { entity_id, .. }
            | Self::RestoreEntity { entity_id, .. }
            | Self::ResolveConflict { entity_id, .. } => Some(*entity_id),
            Self::CreateEdge { source_id, .. } | Self::CreateOrderedEdge { source_id, .. } => {
                Some(*source_id)
            }
            Self::MergeEntities { survivor, .. } => Some(*survivor),
            Self::SplitEntity { source, .. } => Some(*source),
            Self::DeleteEdge { .. }
            | Self::SetEdgeProperty { .. }
            | Self::ClearEdgeProperty { .. }
            | Self::MoveOrderedEdge { .. }
            | Self::LinkTables { .. }
            | Self::UnlinkTables { .. }
            | Self::ConfirmFieldMapping { .. }
            | Self::CreateRule { .. }
            | Self::RestoreEdge { .. } => None,
        }
    }

    /// String name of the operation type for storage/indexing.
    pub fn op_type_name(&self) -> &'static str {
        match self {
            Self::CreateEntity { .. } => "CreateEntity",
            Self::DeleteEntity { .. } => "DeleteEntity",
            Self::AttachFacet { .. } => "AttachFacet",
            Self::DetachFacet { .. } => "DetachFacet",
            Self::RestoreFacet { .. } => "RestoreFacet",
            Self::SetField { .. } => "SetField",
            Self::ClearField { .. } => "ClearField",
            Self::ApplyCrdt { .. } => "ApplyCrdt",
            Self::ClearAndAdd { .. } => "ClearAndAdd",
            Self::CreateEdge { .. } => "CreateEdge",
            Self::DeleteEdge { .. } => "DeleteEdge",
            Self::SetEdgeProperty { .. } => "SetEdgeProperty",
            Self::ClearEdgeProperty { .. } => "ClearEdgeProperty",
            Self::CreateOrderedEdge { .. } => "CreateOrderedEdge",
            Self::MoveOrderedEdge { .. } => "MoveOrderedEdge",
            Self::LinkTables { .. } => "LinkTables",
            Self::UnlinkTables { .. } => "UnlinkTables",
            Self::AddToTable { .. } => "AddToTable",
            Self::RemoveFromTable { .. } => "RemoveFromTable",
            Self::ConfirmFieldMapping { .. } => "ConfirmFieldMapping",
            Self::MergeEntities { .. } => "MergeEntities",
            Self::SplitEntity { .. } => "SplitEntity",
            Self::CreateRule { .. } => "CreateRule",
            Self::RestoreEntity { .. } => "RestoreEntity",
            Self::RestoreEdge { .. } => "RestoreEdge",
            Self::ResolveConflict { .. } => "ResolveConflict",
        }
    }

    pub fn to_msgpack(&self) -> Result<Vec<u8>, CoreError> {
        rmp_serde::to_vec(self).map_err(|e| CoreError::Serialization(e.to_string()))
    }

    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, CoreError> {
        rmp_serde::from_slice(bytes).map_err(|e| CoreError::Serialization(e.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    pub op_id: OpId,
    pub actor_id: ActorId,
    pub hlc: Hlc,
    pub bundle_id: BundleId,
    pub module_versions: BTreeMap<String, String>,
    pub payload: OperationPayload,
    pub signature: Signature,
}

impl Operation {
    fn signing_bytes(
        op_id: &OpId,
        actor_id: &ActorId,
        hlc: &Hlc,
        module_versions: &BTreeMap<String, String>,
        payload_bytes: &[u8],
    ) -> Result<Vec<u8>, CoreError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(op_id.as_bytes());
        bytes.extend_from_slice(actor_id.as_bytes());
        bytes.extend_from_slice(&hlc.to_bytes());
        let mv_bytes = rmp_serde::to_vec(module_versions)
            .map_err(|e| CoreError::Serialization(e.to_string()))?;
        bytes.extend_from_slice(&mv_bytes);
        bytes.extend_from_slice(payload_bytes);
        Ok(bytes)
    }

    pub fn new_signed(
        identity: &ActorIdentity,
        hlc: Hlc,
        bundle_id: BundleId,
        module_versions: BTreeMap<String, String>,
        payload: OperationPayload,
    ) -> Result<Self, CoreError> {
        let op_id = OpId::new();
        let actor_id = identity.actor_id();
        let payload_bytes = payload.to_msgpack()?;
        let signing_bytes =
            Self::signing_bytes(&op_id, &actor_id, &hlc, &module_versions, &payload_bytes)?;
        let signature = identity.sign(&signing_bytes);

        Ok(Self {
            op_id,
            actor_id,
            hlc,
            bundle_id,
            module_versions,
            payload,
            signature,
        })
    }

    pub fn verify_signature(&self) -> Result<(), CoreError> {
        let payload_bytes = self.payload.to_msgpack()?;
        let signing_bytes = Self::signing_bytes(
            &self.op_id,
            &self.actor_id,
            &self.hlc,
            &self.module_versions,
            &payload_bytes,
        )?;
        verify_signature(&self.actor_id, &signing_bytes, &self.signature)
    }
}

impl Ord for Operation {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.hlc
            .cmp(&other.hlc)
            .then(self.op_id.cmp(&other.op_id))
    }
}

impl PartialOrd for Operation {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BundleType {
    UserEdit = 1,
    ScriptOutput = 2,
    Import = 3,
    System = 4,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub bundle_id: BundleId,
    pub actor_id: ActorId,
    pub hlc: Hlc,
    pub bundle_type: BundleType,
    pub op_count: u32,
    pub checksum: [u8; 32],
    pub creates: Vec<EntityId>,
    pub deletes: Vec<EntityId>,
    pub meta: Option<Vec<u8>>,
    pub signature: Signature,
    pub creator_vc: Option<VectorClock>,
}

impl Bundle {
    pub fn new_signed(
        bundle_id: BundleId,
        identity: &ActorIdentity,
        hlc: Hlc,
        bundle_type: BundleType,
        operations: &[Operation],
        creator_vc: Option<VectorClock>,
    ) -> Result<Self, CoreError> {
        let actor_id = identity.actor_id();
        let op_count = operations.len() as u32;

        let mut hasher = blake3::Hasher::new();
        for op in operations {
            let bytes = op.payload.to_msgpack()?;
            hasher.update(&bytes);
        }
        let checksum = *hasher.finalize().as_bytes();

        let mut creates = Vec::new();
        let mut deletes = Vec::new();
        for op in operations {
            match &op.payload {
                OperationPayload::CreateEntity { entity_id, .. } => creates.push(*entity_id),
                OperationPayload::DeleteEntity { entity_id, .. } => deletes.push(*entity_id),
                _ => {}
            }
        }

        let mut sign_bytes = Vec::new();
        sign_bytes.extend_from_slice(bundle_id.as_bytes());
        sign_bytes.extend_from_slice(actor_id.as_bytes());
        sign_bytes.extend_from_slice(&hlc.to_bytes());
        sign_bytes.push(bundle_type as u8);
        sign_bytes.extend_from_slice(&op_count.to_be_bytes());
        sign_bytes.extend_from_slice(&checksum);
        let vc_bytes = rmp_serde::to_vec(&creator_vc)
            .map_err(|e| CoreError::Serialization(e.to_string()))?;
        sign_bytes.extend_from_slice(&vc_bytes);
        let signature = identity.sign(&sign_bytes);

        Ok(Self {
            bundle_id,
            actor_id,
            hlc,
            bundle_type,
            op_count,
            checksum,
            creates,
            deletes,
            meta: None,
            signature,
            creator_vc,
        })
    }
}
