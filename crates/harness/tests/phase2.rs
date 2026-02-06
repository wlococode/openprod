use openprod_core::{
    field_value::FieldValue,
    identity::ActorIdentity,
    ids::*,
    operations::*,
};
use openprod_engine::{Engine, EngineError, UndoResult};
use openprod_harness::TestPeer;
use openprod_storage::SqliteStorage;

// ============================================================================
// Engine Parity Tests (4 tests)
// ============================================================================

#[test]
fn engine_create_entity_with_fields() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record(
        "Project",
        vec![
            ("name", FieldValue::Text("Alpha".into())),
            ("priority", FieldValue::Integer(1)),
        ],
    )?;

    // Verify entity exists and is not deleted
    let entity = peer.engine.get_entity(entity_id)?;
    assert!(entity.is_some());
    let entity = entity.unwrap();
    assert_eq!(entity.entity_id, entity_id);
    assert!(!entity.deleted);

    // Verify fields match
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("Alpha".into())));

    let priority = peer.engine.get_field(entity_id, "priority")?;
    assert_eq!(priority, Some(FieldValue::Integer(1)));

    // Verify facet "Project" attached and not detached
    let facets = peer.engine.get_facets(entity_id)?;
    assert_eq!(facets.len(), 1);
    assert_eq!(facets[0].facet_type, "Project");
    assert!(!facets[0].detached);

    Ok(())
}

#[test]
fn engine_update_and_clear_field() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record(
        "Document",
        vec![("status", FieldValue::Text("draft".into()))],
    )?;

    // Verify initial value
    let status = peer.engine.get_field(entity_id, "status")?;
    assert_eq!(status, Some(FieldValue::Text("draft".into())));

    // Update to "published"
    peer.set_field(entity_id, "status", FieldValue::Text("published".into()))?;
    let status = peer.engine.get_field(entity_id, "status")?;
    assert_eq!(status, Some(FieldValue::Text("published".into())));

    // Clear the field
    peer.clear_field(entity_id, "status")?;
    let status = peer.engine.get_field(entity_id, "status")?;
    assert_eq!(status, None);

    Ok(())
}

#[test]
fn engine_delete_entity_cascades_edges() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    let entity_a = peer.create_record("Node", vec![])?;
    let entity_b = peer.create_record("Node", vec![])?;
    let entity_c = peer.create_record("Node", vec![])?;

    // Create edges A→B and C→A
    let edge_ab = peer.create_edge("link", entity_a, entity_b)?;
    let edge_ca = peer.create_edge("link", entity_c, entity_a)?;

    // Delete entity A — should cascade both edges
    peer.delete_entity(entity_a)?;

    // Verify entity A is deleted
    let entity = peer.engine.get_entity(entity_a)?.unwrap();
    assert!(entity.deleted);

    // Verify edge A→B is soft-deleted
    let edge = peer.engine.get_edge(edge_ab)?.unwrap();
    assert!(edge.deleted);

    // Verify edge C→A is soft-deleted
    let edge = peer.engine.get_edge(edge_ca)?.unwrap();
    assert!(edge.deleted);

    Ok(())
}

#[test]
fn engine_query_pass_through() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record(
        "Widget",
        vec![
            ("name", FieldValue::Text("Gadget".into())),
            ("count", FieldValue::Integer(42)),
        ],
    )?;

    // get_entity returns Some
    let entity = peer.engine.get_entity(entity_id)?;
    assert!(entity.is_some());

    // get_fields returns correct fields
    let fields = peer.engine.get_fields(entity_id)?;
    assert_eq!(fields.len(), 2);

    // get_facets returns correct facets
    let facets = peer.engine.get_facets(entity_id)?;
    assert_eq!(facets.len(), 1);
    assert_eq!(facets[0].facet_type, "Widget");

    // get_entities_by_facet returns the entity
    let by_facet = peer.engine.get_entities_by_facet("Widget")?;
    assert!(by_facet.contains(&entity_id));

    // op_count > 0
    let count = peer.engine.op_count()?;
    assert!(count > 0, "op_count should be > 0, got {count}");

    // get_vector_clock has the actor
    let vc = peer.engine.get_vector_clock()?;
    assert!(
        vc.get(&peer.actor_id()).is_some(),
        "vector clock should contain this peer's actor"
    );

    // get_ops_canonical is non-empty
    let ops = peer.engine.get_ops_canonical()?;
    assert!(!ops.is_empty(), "canonical ops should be non-empty");

    Ok(())
}

// ============================================================================
// Basic Undo Tests (6 tests)
// ============================================================================

#[test]
fn undo_set_field() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record(
        "Task",
        vec![("name", FieldValue::Text("Original".into()))],
    )?;

    let count_before = peer.engine.op_count()?;

    // Update the field
    peer.set_field(entity_id, "name", FieldValue::Text("Updated".into()))?;
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("Updated".into())));

    // Undo the update
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    // Verify name reverted to "Original"
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("Original".into())));

    // Verify undo added inverse operations to the oplog (op_count increased)
    let count_after = peer.engine.op_count()?;
    assert!(
        count_after > count_before + 1,
        "undo should have added inverse ops; before={count_before}, after={count_after}"
    );

    Ok(())
}

#[test]
fn undo_set_field_previously_null() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![])?;

    // Field doesn't exist yet
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, None);

    // Set the field
    peer.set_field(entity_id, "name", FieldValue::Text("hello".into()))?;
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("hello".into())));

    // Undo — should clear the field since it didn't exist before
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, None, "undo should clear a field that was previously null");

    Ok(())
}

#[test]
fn undo_clear_field() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record(
        "Task",
        vec![("name", FieldValue::Text("keep".into()))],
    )?;

    // Verify field exists
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("keep".into())));

    // Clear the field
    peer.clear_field(entity_id, "name")?;
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, None);

    // Undo the clear — should restore the field
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(
        name,
        Some(FieldValue::Text("keep".into())),
        "undo of clear should restore the previous value"
    );

    Ok(())
}

#[test]
fn undo_create_entity() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Use engine directly to get the entity_id back from create
    let (entity_id, _bundle_id) = peer.engine.create_entity_with_fields(
        "Task",
        vec![("name", FieldValue::Text("Todo".into()))],
    )?;

    // Verify entity is alive
    let entity = peer.engine.get_entity(entity_id)?.unwrap();
    assert!(!entity.deleted);

    // Undo the create — inverse is DeleteEntity
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    // Verify entity is now soft-deleted
    let entity = peer.engine.get_entity(entity_id)?.unwrap();
    assert!(entity.deleted, "undo of create should soft-delete the entity");

    Ok(())
}

#[test]
fn undo_delete_entity() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Create entity A with facet, fields, and an edge
    let entity_a = peer.create_record(
        "Task",
        vec![
            ("name", FieldValue::Text("Important".into())),
            ("priority", FieldValue::Integer(1)),
        ],
    )?;
    let entity_b = peer.create_record("Task", vec![])?;
    let edge_ab = peer.create_edge("depends_on", entity_a, entity_b)?;

    // Verify initial state
    let entity = peer.engine.get_entity(entity_a)?.unwrap();
    assert!(!entity.deleted);
    let edge = peer.engine.get_edge(edge_ab)?.unwrap();
    assert!(!edge.deleted);

    // Delete entity A (cascades edge A→B)
    peer.delete_entity(entity_a)?;

    // Verify entity A is deleted and edge is cascade-deleted
    let entity = peer.engine.get_entity(entity_a)?.unwrap();
    assert!(entity.deleted);
    let edge = peer.engine.get_edge(edge_ab)?.unwrap();
    assert!(edge.deleted);

    // Undo the delete
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    // Verify entity A is restored (deleted=false)
    let entity = peer.engine.get_entity(entity_a)?.unwrap();
    assert!(!entity.deleted, "undo of delete should restore entity");

    // Verify fields are restored
    let name = peer.engine.get_field(entity_a, "name")?;
    assert_eq!(
        name,
        Some(FieldValue::Text("Important".into())),
        "undo of delete should restore fields"
    );
    let priority = peer.engine.get_field(entity_a, "priority")?;
    assert_eq!(
        priority,
        Some(FieldValue::Integer(1)),
        "undo of delete should restore all fields"
    );

    // Verify edge A→B is restored (deleted=false)
    let edge = peer.engine.get_edge(edge_ab)?.unwrap();
    assert!(!edge.deleted, "undo of delete should restore cascade-deleted edges");

    Ok(())
}

#[test]
fn undo_create_edge() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    let entity_a = peer.create_record("Node", vec![])?;
    let entity_b = peer.create_record("Node", vec![])?;

    let edge_id = peer.create_edge("relates_to", entity_a, entity_b)?;

    // Verify edge exists and is alive
    let edge = peer.engine.get_edge(edge_id)?.unwrap();
    assert!(!edge.deleted);

    // Undo the create edge — inverse is DeleteEdge
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    // Verify edge is soft-deleted
    let edge = peer.engine.get_edge(edge_id)?.unwrap();
    assert!(edge.deleted, "undo of create edge should soft-delete the edge");

    Ok(())
}

// ============================================================================
// Undo Stack Tests (4 tests)
// ============================================================================

#[test]
fn undo_stack_depth_limit() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // 1 create + 110 set_fields = 111 undoable ops; stack holds 100
    let (entity_id, _) = peer.engine.create_entity_with_fields(
        "Counter",
        vec![("val", FieldValue::Integer(0))],
    )?;

    for i in 1..=110 {
        peer.engine.set_field(entity_id, "val", FieldValue::Integer(i))?;
    }

    // Undo 100 times — all should succeed
    for i in 0..100 {
        let result = peer.engine.undo()?;
        assert!(
            matches!(result, UndoResult::Applied(_)),
            "undo #{} should be Applied, got {:?}",
            i + 1,
            result
        );
    }

    // 101st undo should be Empty (oldest 11 entries were pruned)
    let result = peer.engine.undo()?;
    assert!(
        matches!(result, UndoResult::Empty),
        "undo #101 should be Empty (stack exhausted), got {:?}",
        result
    );

    Ok(())
}

#[test]
fn multiple_consecutive_undos() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Bundle 1: create entity with name="A"
    let (entity_id, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![("name", FieldValue::Text("A".into()))],
    )?;

    // Bundle 2: set name="B"
    peer.engine.set_field(entity_id, "name", FieldValue::Text("B".into()))?;

    // Bundle 3: set name="C"
    peer.engine.set_field(entity_id, "name", FieldValue::Text("C".into()))?;

    // Undo #1 → reverts set name="C", name should be "B"
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("B".into())));

    // Undo #2 → reverts set name="B", name should be "A"
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("A".into())));

    // Undo #3 → reverts create entity bundle, entity should be soft-deleted
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let entity = peer.engine.get_entity(entity_id)?.unwrap();
    assert!(entity.deleted, "third undo should soft-delete the entity");

    Ok(())
}

#[test]
fn undo_empty_stack() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Fresh peer, no operations — undo should return Empty
    let result = peer.engine.undo()?;
    assert!(
        matches!(result, UndoResult::Empty),
        "undo on empty stack should return Empty, got {:?}",
        result
    );

    Ok(())
}

#[test]
fn undo_does_not_push_to_undo_stack() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Create entity (undo slot 1)
    let (entity_id, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![("name", FieldValue::Text("X".into()))],
    )?;

    // Set field (undo slot 2)
    peer.engine.set_field(entity_id, "name", FieldValue::Text("A".into()))?;

    // Undo the set_field — name reverts to "X"
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("X".into())));

    // Undo the create entity — entity should be soft-deleted
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let entity = peer.engine.get_entity(entity_id)?.unwrap();
    assert!(entity.deleted, "second undo should soft-delete the entity");

    // One more undo — should be Empty (undo itself was NOT pushed to undo stack)
    let result = peer.engine.undo()?;
    assert!(
        matches!(result, UndoResult::Empty),
        "undo operations should not push to undo stack; expected Empty, got {:?}",
        result
    );

    Ok(())
}

// ============================================================================
// Redo Tests (4 tests)
// ============================================================================

#[test]
fn redo_after_undo() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    let (entity_id, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![("name", FieldValue::Text("A".into()))],
    )?;

    // Set name to "B"
    peer.engine.set_field(entity_id, "name", FieldValue::Text("B".into()))?;

    // Undo → name should revert to "A"
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("A".into())));

    // Redo → name should be "B" again
    let result = peer.engine.redo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("B".into())));

    Ok(())
}

#[test]
fn redo_empty_stack() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Fresh peer, no operations — redo should return Empty
    let result = peer.engine.redo()?;
    assert!(
        matches!(result, UndoResult::Empty),
        "redo on empty stack should return Empty, got {:?}",
        result
    );

    Ok(())
}

#[test]
fn new_command_clears_redo() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    let (_entity_id, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![("name", FieldValue::Text("A".into()))],
    )?;

    // Undo the create (entity soft-deleted)
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    // New command: set a field on a different entity (clears redo stack)
    let (entity_id2, _) = peer.engine.create_entity(None)?;
    peer.engine.set_field(entity_id2, "name", FieldValue::Text("B".into()))?;

    // Redo should be Empty — redo stack was cleared by the new command
    let result = peer.engine.redo()?;
    assert!(
        matches!(result, UndoResult::Empty),
        "redo should be Empty after new command clears redo stack, got {:?}",
        result
    );

    Ok(())
}

#[test]
fn multiple_undo_redo_cycle() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    let (entity_id, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![("name", FieldValue::Text("A".into()))],
    )?;

    peer.engine.set_field(entity_id, "name", FieldValue::Text("B".into()))?;
    peer.engine.set_field(entity_id, "name", FieldValue::Text("C".into()))?;

    // Undo → name="B"
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("B".into())), "after first undo");

    // Undo → name="A"
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("A".into())), "after second undo");

    // Redo → name="B"
    let result = peer.engine.redo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("B".into())), "after first redo");

    // Redo → name="C"
    let result = peer.engine.redo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("C".into())), "after second redo");

    Ok(())
}

// ============================================================================
// Conflict Detection Tests (4 tests)
// ============================================================================

/// Helper: create a second engine, replicate the entity into it, set a field,
/// then extract the SetField bundle + ops for injection into the primary peer.
fn inject_foreign_set_field(
    peer: &mut TestPeer,
    entity_id: EntityId,
    field_key: &str,
    value: FieldValue,
) -> Result<ActorId, Box<dyn std::error::Error>> {
    let identity_b = ActorIdentity::generate();
    let actor_b = identity_b.actor_id();
    let storage_b = SqliteStorage::open_in_memory()?;
    let mut engine_b = Engine::new(identity_b, storage_b);

    // Create the entity in engine_b so it can set fields on it
    engine_b.execute(
        BundleType::UserEdit,
        vec![OperationPayload::CreateEntity {
            entity_id,
            initial_table: Some("Task".to_string()),
        }],
    )?;

    // Set the conflicting field via engine_b
    engine_b.set_field(entity_id, field_key, value)?;

    // Extract the SetField operation from engine_b's oplog
    let all_ops_b = engine_b.get_ops_canonical()?;
    let set_field_op = all_ops_b
        .iter()
        .find(|op| {
            matches!(
                &op.payload,
                OperationPayload::SetField {
                    entity_id: eid,
                    field_key: fk,
                    ..
                } if *eid == entity_id && fk == field_key
            )
        })
        .expect("engine_b should have a SetField op");
    let target_bundle_id = set_field_op.bundle_id;
    let bundle_ops = engine_b.get_ops_by_bundle(target_bundle_id)?;

    // Create a signed bundle for these ops
    let bundle = Bundle::new_signed(
        target_bundle_id,
        engine_b.identity(),
        set_field_op.hlc,
        BundleType::UserEdit,
        &bundle_ops,
        None,
    )?;

    // Inject only the SetField bundle into the primary peer
    peer.engine.ingest_bundle(&bundle, &bundle_ops)?;

    Ok(actor_b)
}

#[test]
fn undo_conflict_skip_and_advance() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Peer A creates entity with name="Original"
    let (entity_id, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![("name", FieldValue::Text("Original".into()))],
    )?;

    // Peer A sets name="Updated" (this is the undoable action)
    peer.engine
        .set_field(entity_id, "name", FieldValue::Text("Updated".into()))?;

    // Inject Peer B's SetField on the same entity's "name" field
    let actor_b = inject_foreign_set_field(
        &mut peer,
        entity_id,
        "name",
        FieldValue::Text("conflict".into()),
    )?;

    // Peer A calls undo — should return Skipped because Peer B modified "name"
    let result = peer.engine.undo()?;
    match result {
        UndoResult::Skipped { ref conflicts } => {
            assert!(
                !conflicts.is_empty(),
                "conflicts should be non-empty on skip"
            );
            let conflict = &conflicts[0];
            assert_eq!(conflict.entity_id, entity_id);
            assert_eq!(conflict.field_key, "name");
            assert_eq!(conflict.modified_by, actor_b);
        }
        other => panic!("expected Skipped, got {:?}", other),
    }

    Ok(())
}

#[test]
fn undo_no_conflict_same_actor() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    let (entity_id, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![("name", FieldValue::Text("A".into()))],
    )?;

    // Same actor sets field to "B"
    peer.engine
        .set_field(entity_id, "name", FieldValue::Text("B".into()))?;

    // Undo — should return Applied (same actor modified both, no conflict)
    let result = peer.engine.undo()?;
    assert!(
        matches!(result, UndoResult::Applied(_)),
        "same-actor modifications should not conflict; got {:?}",
        result
    );

    // Verify field reverts to "A"
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("A".into())));

    Ok(())
}

#[test]
fn undo_conflict_different_field_no_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Peer A creates entity with name="test", priority=1
    let (entity_id, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![
            ("name", FieldValue::Text("test".into())),
            ("priority", FieldValue::Integer(1)),
        ],
    )?;

    // Peer A sets name="updated"
    peer.engine
        .set_field(entity_id, "name", FieldValue::Text("updated".into()))?;

    // Inject Peer B's SetField on DIFFERENT field "priority" -> 99
    let _actor_b = inject_foreign_set_field(
        &mut peer,
        entity_id,
        "priority",
        FieldValue::Integer(99),
    )?;

    // Peer A undoes the set name="updated" — should succeed (different field)
    let result = peer.engine.undo()?;
    assert!(
        matches!(result, UndoResult::Applied(_)),
        "different field by another actor should not conflict; got {:?}",
        result
    );

    // Verify name reverts to "test"
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("test".into())));

    Ok(())
}

#[test]
fn undo_conflict_entity_with_other_modifications() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Peer A creates entity with facet "Task"
    let (entity_id, _) = peer.engine.create_entity_with_fields("Task", vec![])?;

    // Inject Peer B writing field "notes"="hello" to the SAME entity
    let actor_b = inject_foreign_set_field(
        &mut peer,
        entity_id,
        "notes",
        FieldValue::Text("hello".into()),
    )?;

    // Peer A calls undo (trying to undo the create) — should return Skipped
    // because another actor wrote to the entity
    let result = peer.engine.undo()?;
    match result {
        UndoResult::Skipped { ref conflicts } => {
            assert!(
                !conflicts.is_empty(),
                "conflicts should be non-empty when another actor wrote to entity"
            );
            let conflict = conflicts
                .iter()
                .find(|c| c.field_key == "notes")
                .expect("should have conflict on 'notes' field");
            assert_eq!(conflict.entity_id, entity_id);
            assert_eq!(conflict.modified_by, actor_b);
        }
        other => panic!(
            "expected Skipped (another actor wrote to entity being un-created), got {:?}",
            other
        ),
    }

    Ok(())
}

// ============================================================================
// State Rebuild Tests (2 tests)
// ============================================================================

#[test]
fn rebuild_state_matches_incremental() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Create entities with fields and edges
    let (entity_a, _) = peer.engine.create_entity_with_fields(
        "Project",
        vec![
            ("name", FieldValue::Text("Alpha".into())),
            ("priority", FieldValue::Integer(1)),
        ],
    )?;
    let (entity_b, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![("name", FieldValue::Text("Beta".into()))],
    )?;
    let (edge_id, _) = peer.engine.create_edge("depends_on", entity_a, entity_b)?;

    // Update a field
    peer.engine
        .set_field(entity_a, "priority", FieldValue::Integer(5))?;

    // Capture current state
    let entity_a_before = peer.engine.get_entity(entity_a)?;
    let entity_b_before = peer.engine.get_entity(entity_b)?;
    let fields_a_before = peer.engine.get_fields(entity_a)?;
    let fields_b_before = peer.engine.get_fields(entity_b)?;
    let facets_a_before = peer.engine.get_facets(entity_a)?;
    let facets_b_before = peer.engine.get_facets(entity_b)?;
    let edge_before = peer.engine.get_edge(edge_id)?;

    // Rebuild state from oplog
    let replayed = peer.engine.rebuild_state()?;
    assert!(replayed > 0, "rebuild should replay some operations");

    // Re-query and verify state matches
    let entity_a_after = peer.engine.get_entity(entity_a)?;
    let entity_b_after = peer.engine.get_entity(entity_b)?;
    let fields_a_after = peer.engine.get_fields(entity_a)?;
    let fields_b_after = peer.engine.get_fields(entity_b)?;
    let facets_a_after = peer.engine.get_facets(entity_a)?;
    let facets_b_after = peer.engine.get_facets(entity_b)?;
    let edge_after = peer.engine.get_edge(edge_id)?;

    // Entity records
    assert_eq!(
        entity_a_before.as_ref().map(|e| e.deleted),
        entity_a_after.as_ref().map(|e| e.deleted),
        "entity_a deleted flag should match after rebuild"
    );
    assert_eq!(
        entity_b_before.as_ref().map(|e| e.deleted),
        entity_b_after.as_ref().map(|e| e.deleted),
        "entity_b deleted flag should match after rebuild"
    );

    // Fields
    assert_eq!(
        fields_a_before, fields_a_after,
        "entity_a fields should match after rebuild"
    );
    assert_eq!(
        fields_b_before, fields_b_after,
        "entity_b fields should match after rebuild"
    );

    // Facets
    assert_eq!(facets_a_before.len(), facets_a_after.len());
    for (before, after) in facets_a_before.iter().zip(facets_a_after.iter()) {
        assert_eq!(before.facet_type, after.facet_type);
        assert_eq!(before.detached, after.detached);
    }
    assert_eq!(facets_b_before.len(), facets_b_after.len());
    for (before, after) in facets_b_before.iter().zip(facets_b_after.iter()) {
        assert_eq!(before.facet_type, after.facet_type);
        assert_eq!(before.detached, after.detached);
    }

    // Edge
    assert_eq!(
        edge_before.as_ref().map(|e| e.deleted),
        edge_after.as_ref().map(|e| e.deleted),
        "edge deleted flag should match after rebuild"
    );
    assert_eq!(
        edge_before.as_ref().map(|e| e.edge_type.clone()),
        edge_after.as_ref().map(|e| e.edge_type.clone()),
        "edge type should match after rebuild"
    );

    Ok(())
}

#[test]
fn rebuild_state_empty_oplog() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Fresh peer — no operations
    let replayed = peer.engine.rebuild_state()?;
    assert_eq!(replayed, 0, "rebuild on empty oplog should replay 0 ops");

    // Verify empty state
    let count = peer.engine.op_count()?;
    assert_eq!(count, 0, "op_count should remain 0");

    Ok(())
}

// ============================================================================
// Entity/Edge Redo Tests (2 tests)
// ============================================================================

#[test]
fn redo_create_entity_after_undo() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Create entity with facet "Task" and name="TodoItem"
    let (entity_id, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![("name", FieldValue::Text("TodoItem".into()))],
    )?;

    // Verify entity is alive
    let entity = peer.engine.get_entity(entity_id)?.unwrap();
    assert!(!entity.deleted);

    // Undo — entity should be soft-deleted
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let entity = peer.engine.get_entity(entity_id)?.unwrap();
    assert!(entity.deleted, "undo should soft-delete the entity");

    // Redo — should use RestoreEntity since entity is soft-deleted
    let result = peer.engine.redo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    // Verify entity is alive again (deleted=false)
    let entity = peer.engine.get_entity(entity_id)?.unwrap();
    assert!(
        !entity.deleted,
        "redo of create should restore the entity (deleted=false)"
    );

    Ok(())
}

#[test]
fn redo_delete_entity_after_undo() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Create entity with fields
    let (entity_id, _) = peer.engine.create_entity_with_fields(
        "Task",
        vec![
            ("name", FieldValue::Text("Important".into())),
            ("status", FieldValue::Text("active".into())),
        ],
    )?;

    // Delete entity
    peer.engine.delete_entity(entity_id)?;
    let entity = peer.engine.get_entity(entity_id)?.unwrap();
    assert!(entity.deleted, "entity should be deleted");

    // Undo delete — entity restored
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let entity = peer.engine.get_entity(entity_id)?.unwrap();
    assert!(!entity.deleted, "undo should restore the entity");

    // Redo delete — entity soft-deleted again
    let result = peer.engine.redo()?;
    assert!(matches!(result, UndoResult::Applied(_)));
    let entity = peer.engine.get_entity(entity_id)?.unwrap();
    assert!(
        entity.deleted,
        "redo of delete should soft-delete the entity again"
    );

    Ok(())
}

// ============================================================================
// Edge Case Tests (1 test)
// ============================================================================

#[test]
fn execute_batch_with_undo() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Create entity via engine
    let (entity_id, _) = peer.engine.create_entity_with_fields("Task", vec![])?;

    // Use execute() with multiple SetField payloads in one bundle
    peer.engine.execute(
        BundleType::UserEdit,
        vec![
            OperationPayload::SetField {
                entity_id,
                field_key: "name".to_string(),
                value: FieldValue::Text("BatchName".into()),
            },
            OperationPayload::SetField {
                entity_id,
                field_key: "priority".to_string(),
                value: FieldValue::Integer(42),
            },
            OperationPayload::SetField {
                entity_id,
                field_key: "status".to_string(),
                value: FieldValue::Text("active".into()),
            },
        ],
    )?;

    // Verify all 3 fields are set
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(name, Some(FieldValue::Text("BatchName".into())));
    let priority = peer.engine.get_field(entity_id, "priority")?;
    assert_eq!(priority, Some(FieldValue::Integer(42)));
    let status = peer.engine.get_field(entity_id, "status")?;
    assert_eq!(status, Some(FieldValue::Text("active".into())));

    // Undo — should reverse all 3 fields at once (single bundle)
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    // Verify all 3 fields are None (since they were set for the first time)
    let name = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(
        name, None,
        "undo of batch should clear 'name' (was previously null)"
    );
    let priority = peer.engine.get_field(entity_id, "priority")?;
    assert_eq!(
        priority, None,
        "undo of batch should clear 'priority' (was previously null)"
    );
    let status = peer.engine.get_field(entity_id, "status")?;
    assert_eq!(
        status, None,
        "undo of batch should clear 'status' (was previously null)"
    );

    Ok(()
)
}

// ============================================================================
// Edge Property Tests (6 tests)
// ============================================================================

#[test]
fn engine_create_edge_with_properties() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_a = peer.create_record("Node", vec![])?;
    let entity_b = peer.create_record("Node", vec![])?;

    let edge_id = peer.create_edge_with_properties(
        "relationship",
        entity_a,
        entity_b,
        vec![
            ("role", FieldValue::Text("manager".into())),
            ("weight", FieldValue::Integer(10)),
        ],
    )?;

    // Verify edge exists
    let edge = peer.engine.get_edge(edge_id)?.unwrap();
    assert!(!edge.deleted);
    assert_eq!(edge.edge_type, "relationship");

    // Verify properties individually
    let role = peer.engine.get_edge_property(edge_id, "role")?;
    assert_eq!(role, Some(FieldValue::Text("manager".into())));

    let weight = peer.engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(weight, Some(FieldValue::Integer(10)));

    // Verify get_edge_properties returns all
    let props = peer.engine.get_edge_properties(edge_id)?;
    assert_eq!(props.len(), 2);

    Ok(())
}

#[test]
fn engine_set_edge_property() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_a = peer.create_record("Node", vec![])?;
    let entity_b = peer.create_record("Node", vec![])?;

    let edge_id = peer.create_edge("link", entity_a, entity_b)?;

    // Edge starts with no properties
    let props = peer.engine.get_edge_properties(edge_id)?;
    assert!(props.is_empty());

    // Set a property
    peer.set_edge_property(edge_id, "label", FieldValue::Text("important".into()))?;

    let label = peer.engine.get_edge_property(edge_id, "label")?;
    assert_eq!(label, Some(FieldValue::Text("important".into())));

    // Update the property
    peer.set_edge_property(edge_id, "label", FieldValue::Text("critical".into()))?;

    let label = peer.engine.get_edge_property(edge_id, "label")?;
    assert_eq!(label, Some(FieldValue::Text("critical".into())));

    Ok(())
}

#[test]
fn engine_clear_edge_property() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_a = peer.create_record("Node", vec![])?;
    let entity_b = peer.create_record("Node", vec![])?;

    let edge_id = peer.create_edge_with_properties(
        "link",
        entity_a,
        entity_b,
        vec![("weight", FieldValue::Integer(5))],
    )?;

    // Verify property exists
    let weight = peer.engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(weight, Some(FieldValue::Integer(5)));

    // Clear the property
    peer.clear_edge_property(edge_id, "weight")?;

    let weight = peer.engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(weight, None, "cleared property should be None");

    // Verify get_edge_properties is empty
    let props = peer.engine.get_edge_properties(edge_id)?;
    assert!(props.is_empty(), "edge properties should be empty after clear");

    Ok(())
}

#[test]
fn undo_set_edge_property() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_a = peer.create_record("Node", vec![])?;
    let entity_b = peer.create_record("Node", vec![])?;

    let edge_id = peer.create_edge_with_properties(
        "link",
        entity_a,
        entity_b,
        vec![("weight", FieldValue::Integer(5))],
    )?;

    // Set property to new value
    peer.set_edge_property(edge_id, "weight", FieldValue::Integer(10))?;
    let weight = peer.engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(weight, Some(FieldValue::Integer(10)));

    // Undo — should revert to 5
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    let weight = peer.engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(weight, Some(FieldValue::Integer(5)), "undo should restore previous edge property value");

    Ok(())
}

#[test]
fn undo_create_edge_with_properties() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_a = peer.create_record("Node", vec![])?;
    let entity_b = peer.create_record("Node", vec![])?;

    let edge_id = peer.create_edge_with_properties(
        "link",
        entity_a,
        entity_b,
        vec![
            ("role", FieldValue::Text("lead".into())),
            ("priority", FieldValue::Integer(1)),
        ],
    )?;

    // Verify edge and properties exist
    let edge = peer.engine.get_edge(edge_id)?.unwrap();
    assert!(!edge.deleted);
    let props = peer.engine.get_edge_properties(edge_id)?;
    assert_eq!(props.len(), 2);

    // Undo create edge — edge should be soft-deleted
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    let edge = peer.engine.get_edge(edge_id)?.unwrap();
    assert!(edge.deleted, "undo of create edge should soft-delete the edge");

    Ok(())
}

#[test]
fn rebuild_state_edge_properties() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_a = peer.create_record("Node", vec![])?;
    let entity_b = peer.create_record("Node", vec![])?;

    let edge_id = peer.create_edge_with_properties(
        "link",
        entity_a,
        entity_b,
        vec![("weight", FieldValue::Integer(5))],
    )?;

    // Update the property
    peer.set_edge_property(edge_id, "weight", FieldValue::Integer(10))?;

    // Add another property
    peer.set_edge_property(edge_id, "label", FieldValue::Text("important".into()))?;

    // Capture current state
    let props_before = peer.engine.get_edge_properties(edge_id)?;
    let weight_before = peer.engine.get_edge_property(edge_id, "weight")?;
    let label_before = peer.engine.get_edge_property(edge_id, "label")?;

    // Rebuild state from oplog
    let replayed = peer.engine.rebuild_state()?;
    assert!(replayed > 0);

    // Verify properties match after rebuild
    let props_after = peer.engine.get_edge_properties(edge_id)?;
    assert_eq!(props_before.len(), props_after.len(), "property count should match after rebuild");

    let weight_after = peer.engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(weight_before, weight_after, "weight should match after rebuild");

    let label_after = peer.engine.get_edge_property(edge_id, "label")?;
    assert_eq!(label_before, label_after, "label should match after rebuild");

    Ok(())
}

// ============================================================================
// Error Path Tests (3 tests)
// ============================================================================

#[test]
fn error_set_field_nonexistent_entity() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // EntityId that was never created
    let fake_id = EntityId::new();
    let result = peer.engine.set_field(fake_id, "name", FieldValue::Text("hello".into()));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, EngineError::EntityNotFound(_)),
        "expected EntityNotFound, got: {err}"
    );

    Ok(())
}

#[test]
fn error_set_field_deleted_entity() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![])?;
    peer.delete_entity(entity_id)?;

    let result = peer.engine.set_field(entity_id, "name", FieldValue::Text("hello".into()));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, EngineError::EntityAlreadyDeleted(_)),
        "expected EntityAlreadyDeleted, got: {err}"
    );

    Ok(())
}

#[test]
fn error_resolve_nonexistent_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    let fake_conflict_id = ConflictId::new();
    let result = peer.engine.resolve_conflict(fake_conflict_id, Some(FieldValue::Text("value".into())));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, EngineError::ConflictNotFound(_)),
        "expected ConflictNotFound, got: {err}"
    );

    Ok(())
}

// ============================================================================
// Edge Property Undo Tests (1 test)
// ============================================================================

#[test]
fn undo_clear_edge_property() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_a = peer.create_record("Node", vec![])?;
    let entity_b = peer.create_record("Node", vec![])?;

    let edge_id = peer.create_edge_with_properties(
        "link",
        entity_a,
        entity_b,
        vec![("weight", FieldValue::Integer(5))],
    )?;

    // Set property to new value
    peer.set_edge_property(edge_id, "weight", FieldValue::Integer(10))?;
    let weight = peer.engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(weight, Some(FieldValue::Integer(10)));

    // Clear the property
    peer.clear_edge_property(edge_id, "weight")?;
    let weight = peer.engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(weight, None, "property should be cleared");

    // Undo the clear — should restore property to 10
    let result = peer.engine.undo()?;
    assert!(matches!(result, UndoResult::Applied(_)));

    let weight = peer.engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(
        weight,
        Some(FieldValue::Integer(10)),
        "undo of clear_edge_property should restore the previous value"
    );

    Ok(())
}
