use openprod_core::{
    field_value::FieldValue,
    hlc::HlcClock,
    identity::ActorIdentity,
    ids::*,
    operations::*,
};
use openprod_storage::{SqliteStorage, Storage, StorageError};
use std::collections::BTreeMap;

pub struct TestPeer {
    pub identity: ActorIdentity,
    pub clock: HlcClock,
    pub storage: SqliteStorage,
}

impl TestPeer {
    pub fn new() -> Result<Self, StorageError> {
        Ok(Self {
            identity: ActorIdentity::generate(),
            clock: HlcClock::new(),
            storage: SqliteStorage::open_in_memory()?,
        })
    }

    pub fn actor_id(&self) -> ActorId {
        self.identity.actor_id()
    }

    /// Execute a list of operation payloads as a single bundle.
    pub fn execute_bundle(
        &mut self,
        bundle_type: BundleType,
        payloads: Vec<OperationPayload>,
    ) -> Result<BundleId, Box<dyn std::error::Error>> {
        let bundle_id = BundleId::new();
        let hlc = self.clock.tick()?;
        let module_versions = BTreeMap::new();

        let mut operations = Vec::new();
        for payload in payloads {
            let op = Operation::new_signed(
                &self.identity,
                hlc,
                bundle_id,
                module_versions.clone(),
                payload,
            )?;
            operations.push(op);
        }

        let bundle = Bundle::new_signed(
            bundle_id,
            &self.identity,
            hlc,
            bundle_type,
            &operations,
        )?;

        self.storage.append_bundle(&bundle, &operations)?;
        Ok(bundle_id)
    }

    /// Create an entity with a facet and optional fields.
    pub fn create_record(
        &mut self,
        facet_type: &str,
        fields: Vec<(&str, FieldValue)>,
    ) -> Result<EntityId, Box<dyn std::error::Error>> {
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
        self.execute_bundle(BundleType::UserEdit, payloads)?;
        Ok(entity_id)
    }

    /// Set a field on an entity.
    pub fn set_field(
        &mut self,
        entity_id: EntityId,
        field_key: &str,
        value: FieldValue,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.execute_bundle(
            BundleType::UserEdit,
            vec![OperationPayload::SetField {
                entity_id,
                field_key: field_key.to_string(),
                value,
            }],
        )?;
        Ok(())
    }

    /// Clear a field on an entity.
    pub fn clear_field(
        &mut self,
        entity_id: EntityId,
        field_key: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.execute_bundle(
            BundleType::UserEdit,
            vec![OperationPayload::ClearField {
                entity_id,
                field_key: field_key.to_string(),
            }],
        )?;
        Ok(())
    }

    /// Delete an entity. Queries current edges to compute cascade_edges.
    pub fn delete_entity(
        &mut self,
        entity_id: EntityId,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let edges_from = self.storage.get_edges_from(entity_id)?;
        let edges_to = self.storage.get_edges_to(entity_id)?;
        let cascade_edges: Vec<EdgeId> = edges_from
            .iter()
            .chain(edges_to.iter())
            .filter(|e| !e.deleted)
            .map(|e| e.edge_id)
            .collect();

        self.execute_bundle(
            BundleType::UserEdit,
            vec![OperationPayload::DeleteEntity {
                entity_id,
                cascade_edges,
            }],
        )?;
        Ok(())
    }

    /// Create an edge between two entities.
    pub fn create_edge(
        &mut self,
        edge_type: &str,
        source_id: EntityId,
        target_id: EntityId,
    ) -> Result<EdgeId, Box<dyn std::error::Error>> {
        let edge_id = EdgeId::new();
        self.execute_bundle(
            BundleType::UserEdit,
            vec![OperationPayload::CreateEdge {
                edge_id,
                edge_type: edge_type.to_string(),
                source_id,
                target_id,
                properties: Vec::new(),
            }],
        )?;
        Ok(edge_id)
    }

    /// Delete an edge.
    pub fn delete_edge(&mut self, edge_id: EdgeId) -> Result<(), Box<dyn std::error::Error>> {
        self.execute_bundle(
            BundleType::UserEdit,
            vec![OperationPayload::DeleteEdge { edge_id }],
        )?;
        Ok(())
    }

    /// Detach a facet from an entity.
    pub fn detach_facet(
        &mut self,
        entity_id: EntityId,
        facet_type: &str,
        preserve: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.execute_bundle(
            BundleType::UserEdit,
            vec![OperationPayload::DetachFacet {
                entity_id,
                facet_type: facet_type.to_string(),
                preserve_values: preserve,
            }],
        )?;
        Ok(())
    }
}
