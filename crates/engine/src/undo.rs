use std::collections::VecDeque;

use openprod_core::{
    field_value::FieldValue,
    hlc::Hlc,
    ids::*,
    operations::OperationPayload,
};
use openprod_storage::{EdgeRecord, FacetRecord, SqliteStorage, Storage, StorageError};

pub struct UndoManager {
    undo_stack: VecDeque<UndoEntry>,
    redo_stack: VecDeque<UndoEntry>,
    max_depth: usize,
}

pub struct UndoEntry {
    pub bundle_id: BundleId,
    pub bundle_hlc: Hlc,
    pub payloads: Vec<OperationPayload>,
    pub snapshot: PreExecutionSnapshot,
}

pub struct PreExecutionSnapshot {
    pub field_states: Vec<FieldSnapshot>,
    pub entity_states: Vec<EntitySnapshot>,
    pub edge_states: Vec<EdgeSnapshot>,
    pub facet_states: Vec<FacetSnapshot>,
    pub edge_property_states: Vec<EdgePropertySnapshot>,
}

pub struct FieldSnapshot {
    pub entity_id: EntityId,
    pub field_key: String,
    pub previous_value: Option<FieldValue>,
    /// Captured for conflict detection during undo (see spec: operations.md Undo/Redo).
    /// Not used by compute_inverse.
    pub previous_metadata: Option<(ActorId, Hlc)>,
}

pub struct EntitySnapshot {
    pub entity_id: EntityId,
    /// None = didn't exist, Some(true) = existed and was deleted, Some(false) = existed and alive
    pub existed: Option<bool>,
    pub facets: Vec<FacetRecord>,
    pub fields: Vec<(String, FieldValue)>,
}

pub struct EdgeSnapshot {
    pub edge_id: EdgeId,
    pub previous_state: Option<EdgeRecord>,
}

pub struct FacetSnapshot {
    pub entity_id: EntityId,
    pub facet_type: String,
    pub was_attached: bool,
}

pub struct EdgePropertySnapshot {
    pub edge_id: EdgeId,
    pub property_key: String,
    pub previous_value: Option<FieldValue>,
    pub previous_metadata: Option<(ActorId, Hlc)>,
}

impl UndoManager {
    pub fn new(max_depth: usize) -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            max_depth,
        }
    }

    pub fn push_undo(
        &mut self,
        bundle_id: BundleId,
        hlc: Hlc,
        payloads: Vec<OperationPayload>,
        snapshot: PreExecutionSnapshot,
    ) {
        self.undo_stack.push_back(UndoEntry {
            bundle_id,
            bundle_hlc: hlc,
            payloads,
            snapshot,
        });
        // Enforce depth limit by dropping oldest entry
        if self.undo_stack.len() > self.max_depth {
            self.undo_stack.pop_front();
        }
    }

    pub fn pop_undo(&mut self) -> Option<UndoEntry> {
        self.undo_stack.pop_back()
    }

    pub fn push_redo(&mut self, entry: UndoEntry) {
        self.redo_stack.push_back(entry);
    }

    pub fn pop_redo(&mut self) -> Option<UndoEntry> {
        self.redo_stack.pop_back()
    }

    pub fn clear_redo(&mut self) {
        self.redo_stack.clear();
    }

    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Capture pre-execution snapshot by examining the payloads and querying current state.
    pub fn capture_snapshot(
        &self,
        storage: &SqliteStorage,
        payloads: &[OperationPayload],
    ) -> Result<PreExecutionSnapshot, StorageError> {
        let mut field_states = Vec::new();
        let mut entity_states = Vec::new();
        let mut edge_states = Vec::new();
        let mut facet_states = Vec::new();
        let mut edge_property_states = Vec::new();

        for payload in payloads {
            match payload {
                OperationPayload::CreateEntity {
                    entity_id,
                    initial_table,
                } => {
                    // Snapshot: entity may or may not exist before create
                    let existed = storage.get_entity(*entity_id)?.map(|e| e.deleted);
                    entity_states.push(EntitySnapshot {
                        entity_id: *entity_id,
                        existed,
                        facets: Vec::new(),
                        fields: Vec::new(),
                    });
                    // If there's an initial_table, snapshot facet state
                    if let Some(facet_type) = initial_table {
                        facet_states.push(FacetSnapshot {
                            entity_id: *entity_id,
                            facet_type: facet_type.clone(),
                            was_attached: false,
                        });
                    }
                }

                OperationPayload::DeleteEntity { entity_id, .. } => {
                    // Snapshot: full entity state before deletion
                    let existed = storage.get_entity(*entity_id)?.map(|e| e.deleted);
                    let facets = storage.get_facets(*entity_id)?;
                    let fields = storage.get_fields(*entity_id)?;

                    // Also snapshot all connected edges (both from and to)
                    let edges_from = storage.get_edges_from(*entity_id)?;
                    let edges_to = storage.get_edges_to(*entity_id)?;
                    for edge in edges_from.iter().chain(edges_to.iter()) {
                        if !edge.deleted {
                            edge_states.push(EdgeSnapshot {
                                edge_id: edge.edge_id,
                                previous_state: Some(edge.clone()),
                            });
                        }
                    }

                    entity_states.push(EntitySnapshot {
                        entity_id: *entity_id,
                        existed,
                        facets,
                        fields,
                    });
                }

                OperationPayload::SetField {
                    entity_id,
                    field_key,
                    ..
                } => {
                    let previous_value = storage.get_field(*entity_id, field_key)?;
                    let previous_metadata =
                        storage.get_field_metadata(*entity_id, field_key)?;
                    field_states.push(FieldSnapshot {
                        entity_id: *entity_id,
                        field_key: field_key.clone(),
                        previous_value,
                        previous_metadata,
                    });
                }

                OperationPayload::ClearField {
                    entity_id,
                    field_key,
                } => {
                    let previous_value = storage.get_field(*entity_id, field_key)?;
                    let previous_metadata =
                        storage.get_field_metadata(*entity_id, field_key)?;
                    field_states.push(FieldSnapshot {
                        entity_id: *entity_id,
                        field_key: field_key.clone(),
                        previous_value,
                        previous_metadata,
                    });
                }

                OperationPayload::CreateEdge { edge_id, properties, .. } => {
                    let previous_state = storage.get_edge(*edge_id)?;
                    edge_states.push(EdgeSnapshot {
                        edge_id: *edge_id,
                        previous_state,
                    });
                    // Snapshot edge properties for initial properties
                    for (key, _) in properties {
                        let previous_value = storage.get_edge_property(*edge_id, key)?;
                        let previous_metadata = storage.get_edge_property_metadata(*edge_id, key)?;
                        edge_property_states.push(EdgePropertySnapshot {
                            edge_id: *edge_id,
                            property_key: key.clone(),
                            previous_value,
                            previous_metadata,
                        });
                    }
                }

                OperationPayload::DeleteEdge { edge_id } => {
                    let previous_state = storage.get_edge(*edge_id)?;
                    edge_states.push(EdgeSnapshot {
                        edge_id: *edge_id,
                        previous_state,
                    });
                }

                OperationPayload::AttachFacet {
                    entity_id,
                    facet_type,
                } => {
                    let facets = storage.get_facets(*entity_id)?;
                    let was_attached = facets
                        .iter()
                        .any(|f| f.facet_type == *facet_type && !f.detached);
                    facet_states.push(FacetSnapshot {
                        entity_id: *entity_id,
                        facet_type: facet_type.clone(),
                        was_attached,
                    });
                }

                OperationPayload::DetachFacet {
                    entity_id,
                    facet_type,
                    ..
                } => {
                    let facets = storage.get_facets(*entity_id)?;
                    let was_attached = facets
                        .iter()
                        .any(|f| f.facet_type == *facet_type && !f.detached);
                    facet_states.push(FacetSnapshot {
                        entity_id: *entity_id,
                        facet_type: facet_type.clone(),
                        was_attached,
                    });
                }

                OperationPayload::RestoreEntity { entity_id } => {
                    // Snapshot entity state before restore (same need as CreateEntity)
                    let existed = storage.get_entity(*entity_id)?.map(|e| e.deleted);
                    entity_states.push(EntitySnapshot {
                        entity_id: *entity_id,
                        existed,
                        facets: Vec::new(),
                        fields: Vec::new(),
                    });
                }

                OperationPayload::RestoreEdge { edge_id } => {
                    let previous_state = storage.get_edge(*edge_id)?;
                    edge_states.push(EdgeSnapshot {
                        edge_id: *edge_id,
                        previous_state,
                    });
                }

                OperationPayload::SetEdgeProperty {
                    edge_id,
                    property_key,
                    ..
                } => {
                    let previous_value = storage.get_edge_property(*edge_id, property_key)?;
                    let previous_metadata = storage.get_edge_property_metadata(*edge_id, property_key)?;
                    edge_property_states.push(EdgePropertySnapshot {
                        edge_id: *edge_id,
                        property_key: property_key.clone(),
                        previous_value,
                        previous_metadata,
                    });
                }

                OperationPayload::ClearEdgeProperty {
                    edge_id,
                    property_key,
                } => {
                    let previous_value = storage.get_edge_property(*edge_id, property_key)?;
                    let previous_metadata = storage.get_edge_property_metadata(*edge_id, property_key)?;
                    edge_property_states.push(EdgePropertySnapshot {
                        edge_id: *edge_id,
                        property_key: property_key.clone(),
                        previous_value,
                        previous_metadata,
                    });
                }

                // Other operations: no snapshot needed for undo
                _ => {}
            }
        }

        Ok(PreExecutionSnapshot {
            field_states,
            entity_states,
            edge_states,
            facet_states,
            edge_property_states,
        })
    }

    /// Compute inverse operations from a snapshot and original payloads.
    pub fn compute_inverse(&self, entry: &UndoEntry) -> Vec<OperationPayload> {
        let mut inverse = Vec::new();

        for payload in &entry.payloads {
            match payload {
                OperationPayload::CreateEntity { entity_id, .. } => {
                    // Inverse of create = delete. cascade_edges left empty here;
                    // Engine::undo() computes fresh cascade from live storage state
                    // before executing the inverse bundle.
                    inverse.push(OperationPayload::DeleteEntity {
                        entity_id: *entity_id,
                        cascade_edges: Vec::new(),
                    });
                }

                OperationPayload::DeleteEntity { entity_id, .. } => {
                    // Inverse of delete = restore entity + restore cascade-deleted edges.
                    // DeleteEntity only soft-deletes the entity row and cascade edges;
                    // it does not touch the fields or facets tables, so those survive
                    // intact and need no restoration.
                    inverse.push(OperationPayload::RestoreEntity {
                        entity_id: *entity_id,
                    });

                    // Restore edges that were cascade-deleted
                    for edge_snap in &entry.snapshot.edge_states {
                        if let Some(edge) = &edge_snap.previous_state
                            && (edge.source_id == *entity_id || edge.target_id == *entity_id)
                        {
                            inverse.push(OperationPayload::RestoreEdge {
                                edge_id: edge_snap.edge_id,
                            });
                        }
                    }
                }

                OperationPayload::SetField {
                    entity_id,
                    field_key,
                    ..
                } => {
                    if let Some(field_snap) = entry.snapshot.field_states.iter().find(|s| {
                        s.entity_id == *entity_id && s.field_key == *field_key
                    }) {
                        match &field_snap.previous_value {
                            Some(prev_val) => {
                                inverse.push(OperationPayload::SetField {
                                    entity_id: *entity_id,
                                    field_key: field_key.clone(),
                                    value: prev_val.clone(),
                                });
                            }
                            None => {
                                // Field didn't exist before -- clear it
                                inverse.push(OperationPayload::ClearField {
                                    entity_id: *entity_id,
                                    field_key: field_key.clone(),
                                });
                            }
                        }
                    }
                }

                OperationPayload::ClearField {
                    entity_id,
                    field_key,
                } => {
                    if let Some(field_snap) = entry.snapshot.field_states.iter().find(|s| {
                        s.entity_id == *entity_id && s.field_key == *field_key
                    })
                        && let Some(prev_val) = &field_snap.previous_value
                    {
                        inverse.push(OperationPayload::SetField {
                            entity_id: *entity_id,
                            field_key: field_key.clone(),
                            value: prev_val.clone(),
                        });
                    }
                    // If field didn't exist before clear, no-op
                }

                OperationPayload::CreateEdge { edge_id, .. } => {
                    inverse.push(OperationPayload::DeleteEdge { edge_id: *edge_id });
                }

                OperationPayload::DeleteEdge { edge_id } => {
                    inverse.push(OperationPayload::RestoreEdge { edge_id: *edge_id });
                }

                OperationPayload::AttachFacet {
                    entity_id,
                    facet_type,
                } => {
                    // Inverse of attach = detach (preserve values so redo can restore them)
                    inverse.push(OperationPayload::DetachFacet {
                        entity_id: *entity_id,
                        facet_type: facet_type.clone(),
                        preserve_values: true,
                    });
                }

                OperationPayload::DetachFacet {
                    entity_id,
                    facet_type,
                    preserve_values,
                } => {
                    if *preserve_values {
                        // Was preserved, restore it
                        inverse.push(OperationPayload::RestoreFacet {
                            entity_id: *entity_id,
                            facet_type: facet_type.clone(),
                        });
                    } else {
                        // Wasn't preserved, just reattach
                        inverse.push(OperationPayload::AttachFacet {
                            entity_id: *entity_id,
                            facet_type: facet_type.clone(),
                        });
                    }
                }

                OperationPayload::RestoreEntity { entity_id } => {
                    // Inverse of restore = re-delete
                    inverse.push(OperationPayload::DeleteEntity {
                        entity_id: *entity_id,
                        cascade_edges: Vec::new(),
                    });
                }

                OperationPayload::RestoreEdge { edge_id } => {
                    // Inverse of restore = re-delete
                    inverse.push(OperationPayload::DeleteEdge { edge_id: *edge_id });
                }

                OperationPayload::SetEdgeProperty {
                    edge_id,
                    property_key,
                    ..
                } => {
                    if let Some(snap) = entry.snapshot.edge_property_states.iter().find(|s| {
                        s.edge_id == *edge_id && s.property_key == *property_key
                    }) {
                        match &snap.previous_value {
                            Some(prev_val) => {
                                inverse.push(OperationPayload::SetEdgeProperty {
                                    edge_id: *edge_id,
                                    property_key: property_key.clone(),
                                    value: prev_val.clone(),
                                });
                            }
                            None => {
                                inverse.push(OperationPayload::ClearEdgeProperty {
                                    edge_id: *edge_id,
                                    property_key: property_key.clone(),
                                });
                            }
                        }
                    }
                }

                OperationPayload::ClearEdgeProperty {
                    edge_id,
                    property_key,
                } => {
                    if let Some(snap) = entry.snapshot.edge_property_states.iter().find(|s| {
                        s.edge_id == *edge_id && s.property_key == *property_key
                    })
                        && let Some(prev_val) = &snap.previous_value
                    {
                        inverse.push(OperationPayload::SetEdgeProperty {
                            edge_id: *edge_id,
                            property_key: property_key.clone(),
                            value: prev_val.clone(),
                        });
                    }
                    // If property didn't exist before clear, no-op
                }

                // Other operations: no inverse needed (shouldn't be undoable)
                _ => {}
            }
        }

        inverse
    }
}
