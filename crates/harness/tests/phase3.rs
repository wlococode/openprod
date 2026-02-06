use std::collections::BTreeMap;

use openprod_core::{
    field_value::FieldValue,
    hlc::Hlc,
    identity::ActorIdentity,
    ids::*,
    operations::*,
    vector_clock::VectorClock,
};
use openprod_harness::{TestNetwork, TestPeer};
use openprod_storage::{ConflictRecord, ConflictStatus, ConflictValue, SqliteStorage, Storage};

/// Helper: create a shared entity on peer_a, replicate its creation bundle to peer_b.
/// Returns the entity_id.
fn setup_shared_entity(
    peer_a: &mut TestPeer,
    peer_b: &mut TestPeer,
    field_key: &str,
    initial_value: FieldValue,
) -> Result<EntityId, Box<dyn std::error::Error>> {
    let entity_id = peer_a.create_record("Task", vec![(field_key, initial_value)])?;

    // Extract bundles and ops from peer_a and ingest into peer_b
    let ops = peer_a.engine.get_ops_canonical()?;
    let bundle_id = ops[0].bundle_id;
    let bundle_ops = peer_a.engine.get_ops_by_bundle(bundle_id)?;
    let vc = peer_a.engine.storage().get_bundle_vector_clock(bundle_id)?;
    let bundle = Bundle::new_signed(
        bundle_id,
        peer_a.engine.identity(),
        ops[0].hlc,
        BundleType::UserEdit,
        &bundle_ops,
        vc,
    )?;
    peer_b.engine.ingest_bundle(&bundle, &bundle_ops)?;

    Ok(entity_id)
}

/// Helper: extract the latest bundle from a peer and ingest it into another peer.
fn sync_latest_bundle(
    from: &TestPeer,
    to: &mut TestPeer,
) -> Result<Vec<ConflictRecord>, Box<dyn std::error::Error>> {
    let ops = from.engine.get_ops_canonical()?;
    let last_op = ops.last().unwrap();
    let bundle_id = last_op.bundle_id;
    let bundle_ops = from.engine.get_ops_by_bundle(bundle_id)?;
    let vc = from.engine.storage().get_bundle_vector_clock(bundle_id)?;
    let bundle = Bundle::new_signed(
        bundle_id,
        from.engine.identity(),
        last_op.hlc,
        BundleType::UserEdit,
        &bundle_ops,
        vc,
    )?;
    let conflicts = to.engine.ingest_bundle(&bundle, &bundle_ops)?;
    Ok(conflicts)
}

// ============================================================================
// Batch 1: Core Types + Schema + LWW Fix Tests
// ============================================================================

#[test]
fn vector_clock_msgpack_roundtrip_empty() -> Result<(), Box<dyn std::error::Error>> {
    let vc = VectorClock::new();
    let bytes = vc.to_msgpack()?;
    let recovered = VectorClock::from_msgpack(&bytes)?;
    assert_eq!(vc, recovered);
    Ok(())
}

#[test]
fn vector_clock_msgpack_roundtrip_single() -> Result<(), Box<dyn std::error::Error>> {
    let mut vc = VectorClock::new();
    vc.update(ActorId::from_bytes([1; 32]), Hlc::new(1000, 5));
    let bytes = vc.to_msgpack()?;
    let recovered = VectorClock::from_msgpack(&bytes)?;
    assert_eq!(vc, recovered);
    Ok(())
}

#[test]
fn vector_clock_msgpack_roundtrip_multi() -> Result<(), Box<dyn std::error::Error>> {
    let mut vc = VectorClock::new();
    vc.update(ActorId::from_bytes([1; 32]), Hlc::new(1000, 5));
    vc.update(ActorId::from_bytes([2; 32]), Hlc::new(2000, 0));
    vc.update(ActorId::from_bytes([3; 32]), Hlc::new(500, 99));
    let bytes = vc.to_msgpack()?;
    let recovered = VectorClock::from_msgpack(&bytes)?;
    assert_eq!(vc, recovered);
    Ok(())
}

#[test]
fn conflict_id_and_overlay_id_creation() {
    let cid = ConflictId::new();
    let oid = OverlayId::new();
    // Verify they produce valid UUIDs (no panic) and are distinct types
    assert_ne!(cid.as_bytes(), oid.as_bytes());
}

#[test]
fn resolve_conflict_payload_entity_id_and_op_type() {
    let entity_id = EntityId::new();
    let conflict_id = ConflictId::new();
    let payload = OperationPayload::ResolveConflict {
        conflict_id,
        entity_id,
        field_key: "name".to_string(),
        chosen_value: Some(FieldValue::Text("resolved".into())),
    };
    assert_eq!(payload.entity_id(), Some(entity_id));
    assert_eq!(payload.op_type_name(), "ResolveConflict");
}

#[test]
fn schema_creates_new_tables() -> Result<(), Box<dyn std::error::Error>> {
    let _storage = SqliteStorage::open_in_memory()?;
    // If schema init didn't create the tables, the open would fail.
    // Other tests (insert_and_read_conflict_record etc.) verify queries work.
    Ok(())
}

#[test]
fn insert_and_read_conflict_record() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("test".into()))])?;

    let conflict_id = ConflictId::new();
    let detected_at = Hlc::new(5000, 0);
    // Use a real bundle_id from the entity creation (FK constraint)
    let ops = peer.engine.get_ops_canonical()?;
    let bundle_id = ops[0].bundle_id;

    let record = ConflictRecord {
        conflict_id,
        entity_id,
        field_key: "name".to_string(),
        status: ConflictStatus::Open,
        values: vec![
            ConflictValue {
                value: Some(FieldValue::Text("alice".into()).to_msgpack()?),
                actor_id: ActorId::from_bytes([1; 32]),
                hlc: Hlc::new(1000, 0),
                op_id: OpId::new(),
            },
            ConflictValue {
                value: Some(FieldValue::Text("bob".into()).to_msgpack()?),
                actor_id: ActorId::from_bytes([2; 32]),
                hlc: Hlc::new(1001, 0),
                op_id: OpId::new(),
            },
        ],
        detected_at,
        detected_in_bundle: bundle_id,
        resolved_at: None,
        resolved_by: None,
        resolved_op_id: None,
        resolved_value: None,
        reopened_at: None,
        reopened_by_op: None,
    };

    peer.engine.storage_mut().insert_conflict(&record)?;

    let loaded = peer.engine.storage().get_conflict(conflict_id)?;
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.conflict_id, conflict_id);
    assert_eq!(loaded.entity_id, entity_id);
    assert_eq!(loaded.field_key, "name");
    assert_eq!(loaded.status, ConflictStatus::Open);

    // Check open conflicts query
    let open = peer.engine.storage().get_open_conflicts_for_entity(entity_id)?;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].conflict_id, conflict_id);

    Ok(())
}

#[test]
fn bundle_stores_and_retrieves_creator_vector_clock() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("test".into()))])?;

    // The bundle should have a creator_vc stored
    let ops = peer.engine.get_ops_canonical()?;
    assert!(!ops.is_empty());
    let bundle_id = ops[0].bundle_id;
    let vc = peer.engine.storage().get_bundle_vector_clock(bundle_id)?;
    // First bundle's creator_vc should be empty VC (no prior state)
    assert!(vc.is_some());

    // Make a second edit — its creator_vc should contain our actor
    peer.set_field(entity_id, "status", FieldValue::Text("active".into()))?;
    let ops2 = peer.engine.get_ops_canonical()?;
    let second_bundle_id = ops2.last().unwrap().bundle_id;
    let vc2 = peer.engine.storage().get_bundle_vector_clock(second_bundle_id)?;
    assert!(vc2.is_some());
    let vc2 = vc2.unwrap();
    // Should contain our actor with an HLC
    assert!(vc2.get(&peer.actor_id()).is_some());

    Ok(())
}

#[test]
fn lww_later_set_field_wins_over_earlier() -> Result<(), Box<dyn std::error::Error>> {
    // Test: when ops arrive out of order, LWW picks the latest
    let identity = ActorIdentity::generate();
    let mut storage = SqliteStorage::open_in_memory()?;

    let entity_id = EntityId::new();
    let bundle_id_1 = BundleId::new();
    let hlc_early = Hlc::new(1000, 0);
    let hlc_late = Hlc::new(2000, 0);

    // Create entity first
    let create_op = Operation::new_signed(
        &identity, hlc_early, bundle_id_1,
        std::collections::BTreeMap::new(),
        OperationPayload::CreateEntity { entity_id, initial_table: None },
    )?;
    let bundle1 = Bundle::new_signed(bundle_id_1, &identity, hlc_early, BundleType::UserEdit, std::slice::from_ref(&create_op), None)?;
    storage.append_bundle(&bundle1, &[create_op])?;

    // Ingest late SetField first
    let bundle_id_2 = BundleId::new();
    let set_late = Operation::new_signed(
        &identity, hlc_late, bundle_id_2,
        std::collections::BTreeMap::new(),
        OperationPayload::SetField { entity_id, field_key: "name".into(), value: FieldValue::Text("late".into()) },
    )?;
    let bundle2 = Bundle::new_signed(bundle_id_2, &identity, hlc_late, BundleType::UserEdit, std::slice::from_ref(&set_late), None)?;
    storage.append_bundle(&bundle2, &[set_late])?;

    // Now ingest early SetField (out of order)
    let bundle_id_3 = BundleId::new();
    let set_early = Operation::new_signed(
        &identity, hlc_early, bundle_id_3,
        std::collections::BTreeMap::new(),
        OperationPayload::SetField { entity_id, field_key: "name".into(), value: FieldValue::Text("early".into()) },
    )?;
    let bundle3 = Bundle::new_signed(bundle_id_3, &identity, hlc_early, BundleType::UserEdit, std::slice::from_ref(&set_early), None)?;
    storage.append_bundle(&bundle3, &[set_early])?;

    // LWW should keep "late"
    let val = storage.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("late".into())));

    Ok(())
}

#[test]
fn lww_clear_field_older_does_not_delete_newer_set() -> Result<(), Box<dyn std::error::Error>> {
    let identity = ActorIdentity::generate();
    let mut storage = SqliteStorage::open_in_memory()?;

    let entity_id = EntityId::new();
    let hlc_early = Hlc::new(1000, 0);
    let hlc_late = Hlc::new(2000, 0);

    // Create entity
    let bundle_id_1 = BundleId::new();
    let create_op = Operation::new_signed(
        &identity, hlc_early, bundle_id_1,
        std::collections::BTreeMap::new(),
        OperationPayload::CreateEntity { entity_id, initial_table: None },
    )?;
    let bundle1 = Bundle::new_signed(bundle_id_1, &identity, hlc_early, BundleType::UserEdit, std::slice::from_ref(&create_op), None)?;
    storage.append_bundle(&bundle1, &[create_op])?;

    // Ingest SetField with late HLC
    let bundle_id_2 = BundleId::new();
    let set_op = Operation::new_signed(
        &identity, hlc_late, bundle_id_2,
        std::collections::BTreeMap::new(),
        OperationPayload::SetField { entity_id, field_key: "name".into(), value: FieldValue::Text("keep_me".into()) },
    )?;
    let bundle2 = Bundle::new_signed(bundle_id_2, &identity, hlc_late, BundleType::UserEdit, std::slice::from_ref(&set_op), None)?;
    storage.append_bundle(&bundle2, &[set_op])?;

    // Now ingest ClearField with earlier HLC — should NOT delete the field
    let bundle_id_3 = BundleId::new();
    let clear_op = Operation::new_signed(
        &identity, hlc_early, bundle_id_3,
        std::collections::BTreeMap::new(),
        OperationPayload::ClearField { entity_id, field_key: "name".into() },
    )?;
    let bundle3 = Bundle::new_signed(bundle_id_3, &identity, hlc_early, BundleType::UserEdit, std::slice::from_ref(&clear_op), None)?;
    storage.append_bundle(&bundle3, &[clear_op])?;

    // Field should still exist with the later value
    let val = storage.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("keep_me".into())));

    Ok(())
}

#[test]
fn lww_set_field_older_does_not_overwrite_newer_clear() -> Result<(), Box<dyn std::error::Error>> {
    let identity = ActorIdentity::generate();
    let mut storage = SqliteStorage::open_in_memory()?;

    let entity_id = EntityId::new();
    let hlc_early = Hlc::new(1000, 0);
    let hlc_late = Hlc::new(2000, 0);

    // Create entity
    let bundle_id_1 = BundleId::new();
    let create_op = Operation::new_signed(
        &identity, hlc_early, bundle_id_1,
        std::collections::BTreeMap::new(),
        OperationPayload::CreateEntity { entity_id, initial_table: None },
    )?;
    let bundle1 = Bundle::new_signed(bundle_id_1, &identity, hlc_early, BundleType::UserEdit, std::slice::from_ref(&create_op), None)?;
    storage.append_bundle(&bundle1, &[create_op])?;

    // Ingest ClearField with late HLC (tombstone)
    let bundle_id_2 = BundleId::new();
    let clear_op = Operation::new_signed(
        &identity, hlc_late, bundle_id_2,
        std::collections::BTreeMap::new(),
        OperationPayload::ClearField { entity_id, field_key: "name".into() },
    )?;
    let bundle2 = Bundle::new_signed(bundle_id_2, &identity, hlc_late, BundleType::UserEdit, std::slice::from_ref(&clear_op), None)?;
    storage.append_bundle(&bundle2, &[clear_op])?;

    // Now ingest SetField with earlier HLC — should NOT re-create the field
    let bundle_id_3 = BundleId::new();
    let set_op = Operation::new_signed(
        &identity, hlc_early, bundle_id_3,
        std::collections::BTreeMap::new(),
        OperationPayload::SetField { entity_id, field_key: "name".into(), value: FieldValue::Text("stale".into()) },
    )?;
    let bundle3 = Bundle::new_signed(bundle_id_3, &identity, hlc_early, BundleType::UserEdit, std::slice::from_ref(&set_op), None)?;
    storage.append_bundle(&bundle3, &[set_op])?;

    // Field should be None (cleared by tombstone)
    let val = storage.get_field(entity_id, "name")?;
    assert_eq!(val, None);

    Ok(())
}

#[test]
fn tombstone_get_field_returns_none_but_metadata_exists() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("test".into()))])?;

    // Clear the field — creates a tombstone
    peer.clear_field(entity_id, "name")?;

    // get_field should return None
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, None);

    // get_fields should not include the tombstone
    let fields = peer.engine.get_fields(entity_id)?;
    assert!(fields.is_empty() || fields.iter().all(|(k, _)| k != "name"));

    // get_field_metadata should still return metadata for the tombstone
    let meta = peer.engine.get_field_metadata(entity_id, "name")?;
    assert!(meta.is_some(), "tombstone should have metadata for conflict detection");

    Ok(())
}

// ============================================================================
// Batch 2: Conflict Detection Engine Tests
// ============================================================================

#[test]
fn concurrent_edits_produce_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    // Create shared entity on Alice, replicate to Bob
    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Alice edits offline
    alice.set_field(entity_id, "name", FieldValue::Text("alice_value".into()))?;

    // Bob edits offline (doesn't know about Alice's edit)
    bob.set_field(entity_id, "name", FieldValue::Text("bob_value".into()))?;

    // Sync Alice's edit to Bob → should detect conflict
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].entity_id, entity_id);
    assert_eq!(conflicts[0].field_key, "name");
    assert_eq!(conflicts[0].status, ConflictStatus::Open);

    Ok(())
}

#[test]
fn sequential_edits_no_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Alice edits
    alice.set_field(entity_id, "name", FieldValue::Text("alice_value".into()))?;

    // Bob syncs Alice's edit BEFORE making his own
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert!(conflicts.is_empty());

    // Now Bob edits — he knows about Alice's edit, so no conflict
    bob.set_field(entity_id, "name", FieldValue::Text("bob_value".into()))?;

    // Sync Bob's edit to Alice
    let conflicts = sync_latest_bundle(&bob, &mut alice)?;
    assert!(conflicts.is_empty());

    Ok(())
}

#[test]
fn different_fields_no_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Alice edits name
    alice.set_field(entity_id, "name", FieldValue::Text("alice_name".into()))?;

    // Bob edits status (different field)
    bob.set_field(entity_id, "status", FieldValue::Text("active".into()))?;

    // Sync Alice's name edit to Bob — no conflict (different fields)
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert!(conflicts.is_empty());

    Ok(())
}

#[test]
fn three_way_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;
    let mut charlie = TestPeer::new()?;

    // Create shared entity, replicate to all peers
    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;
    // Also replicate to Charlie
    let ops = alice.engine.get_ops_canonical()?;
    let bundle_id = ops[0].bundle_id;
    let bundle_ops = alice.engine.get_ops_by_bundle(bundle_id)?;
    let vc = alice.engine.storage().get_bundle_vector_clock(bundle_id)?;
    let bundle = Bundle::new_signed(bundle_id, alice.engine.identity(), ops[0].hlc, BundleType::UserEdit, &bundle_ops, vc)?;
    charlie.engine.ingest_bundle(&bundle, &bundle_ops)?;

    // All three edit offline
    alice.set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob".into()))?;
    charlie.set_field(entity_id, "name", FieldValue::Text("charlie".into()))?;

    // Sync Alice to Bob → first conflict
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert_eq!(conflicts.len(), 1);

    // Sync Charlie to Bob → should extend existing conflict to 3-way
    let conflicts2 = sync_latest_bundle(&charlie, &mut bob)?;
    assert_eq!(conflicts2.len(), 1, "should return the extended conflict");
    let open = bob.engine.get_open_conflicts_for_entity(entity_id)?;
    assert_eq!(open.len(), 1, "should still be one conflict record for the field");
    assert_eq!(open[0].values.len(), 3, "three-way conflict should have 3 branch tips");

    Ok(())
}

#[test]
fn resolve_conflict_updates_field_and_status() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    alice.set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob".into()))?;

    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert_eq!(conflicts.len(), 1);
    let conflict_id = conflicts[0].conflict_id;

    // Resolve conflict on Bob's engine
    let chosen = FieldValue::Text("resolved_value".into());
    let _bundle_id = bob.engine.resolve_conflict(conflict_id, Some(chosen.clone()))?;

    // Verify field is updated
    let val = bob.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(chosen));

    // Verify conflict status is resolved
    let conflict = bob.engine.get_conflict(conflict_id)?;
    assert!(conflict.is_some());
    assert_eq!(conflict.unwrap().status, ConflictStatus::Resolved);

    // Verify no open conflicts remain
    let open = bob.engine.get_open_conflicts_for_entity(entity_id)?;
    assert!(open.is_empty());

    Ok(())
}

#[test]
fn resolution_is_auditable() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    alice.set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob".into()))?;

    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    let conflict_id = conflicts[0].conflict_id;

    bob.engine.resolve_conflict(conflict_id, Some(FieldValue::Text("chosen".into())))?;

    // The oplog should contain a ResolveConflict operation
    let ops = bob.engine.get_ops_canonical()?;
    let resolve_ops: Vec<_> = ops.iter().filter(|o| {
        matches!(o.payload, OperationPayload::ResolveConflict { .. })
    }).collect();
    assert_eq!(resolve_ops.len(), 1);
    match &resolve_ops[0].payload {
        OperationPayload::ResolveConflict { conflict_id: cid, entity_id: eid, field_key, chosen_value } => {
            assert_eq!(*cid, conflict_id);
            assert_eq!(*eid, entity_id);
            assert_eq!(field_key, "name");
            assert_eq!(*chosen_value, Some(FieldValue::Text("chosen".into())));
        }
        _ => panic!("expected ResolveConflict"),
    }

    Ok(())
}

#[test]
fn late_arriving_edit_reopens_resolved_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;
    let mut darcy = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Also replicate to Darcy
    let ops = alice.engine.get_ops_canonical()?;
    let bundle_id = ops[0].bundle_id;
    let bundle_ops = alice.engine.get_ops_by_bundle(bundle_id)?;
    let vc = alice.engine.storage().get_bundle_vector_clock(bundle_id)?;
    let bundle = Bundle::new_signed(bundle_id, alice.engine.identity(), ops[0].hlc, BundleType::UserEdit, &bundle_ops, vc)?;
    darcy.engine.ingest_bundle(&bundle, &bundle_ops)?;

    // Alice and Bob edit concurrently
    alice.set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob".into()))?;

    // Sync Alice to Bob → conflict
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert_eq!(conflicts.len(), 1);
    let conflict_id = conflicts[0].conflict_id;

    // Bob resolves
    bob.engine.resolve_conflict(conflict_id, Some(FieldValue::Text("resolved".into())))?;
    let c = bob.engine.get_conflict(conflict_id)?.unwrap();
    assert_eq!(c.status, ConflictStatus::Resolved);

    // Darcy edits offline (doesn't know about resolution)
    darcy.set_field(entity_id, "name", FieldValue::Text("darcy".into()))?;

    // Sync Darcy to Bob → should reopen the conflict with fresh branch tips
    let conflicts2 = sync_latest_bundle(&darcy, &mut bob)?;
    assert_eq!(conflicts2.len(), 1);
    let reopened = bob.engine.get_conflict(conflict_id)?.unwrap();
    assert_eq!(reopened.status, ConflictStatus::Open);
    assert!(reopened.reopened_at.is_some());
    // Reopened conflict should have 2 values: the resolution tip + Darcy's late edit
    assert_eq!(reopened.values.len(), 2, "reopened conflict should have resolution + late edit tips");
    // One value should be the resolution value, the other should be Darcy's
    let has_resolved = reopened.values.iter().any(|v| {
        v.value.as_ref().and_then(|b| FieldValue::from_msgpack(b).ok()) == Some(FieldValue::Text("resolved".into()))
    });
    let has_darcy = reopened.values.iter().any(|v| {
        v.value.as_ref().and_then(|b| FieldValue::from_msgpack(b).ok()) == Some(FieldValue::Text("darcy".into()))
    });
    assert!(has_resolved, "reopened conflict should contain resolution value as a branch tip");
    assert!(has_darcy, "reopened conflict should contain late-arriving edit as a branch tip");

    Ok(())
}

#[test]
fn lww_display_during_open_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    alice.set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob".into()))?;

    let _conflicts = sync_latest_bundle(&alice, &mut bob)?;

    // During open conflict, the field should show the LWW winner
    let val = bob.engine.get_field(entity_id, "name")?;
    assert!(val.is_some(), "field should have a display value during conflict");

    Ok(())
}

#[test]
fn same_actor_no_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Alice edits twice
    alice.set_field(entity_id, "name", FieldValue::Text("first".into()))?;
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert!(conflicts.is_empty());

    alice.set_field(entity_id, "name", FieldValue::Text("second".into()))?;
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert!(conflicts.is_empty());

    Ok(())
}

#[test]
fn deterministic_lww_tiebreak() -> Result<(), Box<dyn std::error::Error>> {
    // When two ops have the same HLC, LWW should deterministically pick
    // the one with the larger op_id (byte comparison)
    let identity = ActorIdentity::generate();
    let mut storage = SqliteStorage::open_in_memory()?;

    let entity_id = EntityId::new();
    let hlc = Hlc::new(1000, 0);
    let same_hlc = Hlc::new(2000, 0);

    // Create entity
    let bid1 = BundleId::new();
    let create_op = Operation::new_signed(&identity, hlc, bid1, BTreeMap::new(),
        OperationPayload::CreateEntity { entity_id, initial_table: None })?;
    let b1 = Bundle::new_signed(bid1, &identity, hlc, BundleType::UserEdit, std::slice::from_ref(&create_op), None)?;
    storage.append_bundle(&b1, std::slice::from_ref(&create_op))?;

    // Two SetFields with identical HLC
    let bid2 = BundleId::new();
    let set_a = Operation::new_signed(&identity, same_hlc, bid2, BTreeMap::new(),
        OperationPayload::SetField { entity_id, field_key: "x".into(), value: FieldValue::Text("A".into()) })?;
    let b2 = Bundle::new_signed(bid2, &identity, same_hlc, BundleType::UserEdit, std::slice::from_ref(&set_a), None)?;
    storage.append_bundle(&b2, std::slice::from_ref(&set_a))?;

    let bid3 = BundleId::new();
    let set_b = Operation::new_signed(&identity, same_hlc, bid3, BTreeMap::new(),
        OperationPayload::SetField { entity_id, field_key: "x".into(), value: FieldValue::Text("B".into()) })?;
    let b3 = Bundle::new_signed(bid3, &identity, same_hlc, BundleType::UserEdit, std::slice::from_ref(&set_b), None)?;
    storage.append_bundle(&b3, std::slice::from_ref(&set_b))?;

    // The winner should be determined by op_id comparison
    let val = storage.get_field(entity_id, "x")?;
    assert!(val.is_some());
    // We don't assert which value wins (it's op_id dependent),
    // but it should be deterministic — same result after rebuild
    let val_before = val.unwrap();
    storage.rebuild_from_oplog()?;
    let val_after = storage.get_field(entity_id, "x")?;
    assert_eq!(Some(val_before), val_after);

    Ok(())
}

#[test]
fn resolve_already_resolved_conflict_returns_error() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    alice.set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob".into()))?;

    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    let conflict_id = conflicts[0].conflict_id;

    // Resolve once
    bob.engine.resolve_conflict(conflict_id, Some(FieldValue::Text("resolved".into())))?;

    // Try to resolve again → should error
    let result = bob.engine.resolve_conflict(conflict_id, Some(FieldValue::Text("again".into())));
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("already resolved"), "error should mention 'already resolved': {err_msg}");

    Ok(())
}

#[test]
fn crdt_field_no_conflict() -> Result<(), Box<dyn std::error::Error>> {
    // ApplyCrdt ops should NOT produce conflicts (they're naturally excluded
    // since only SetField/ClearField trigger snapshot_field_metadata)
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Create ApplyCrdt bundles manually on Alice and Bob
    let alice_crdt_payloads = vec![OperationPayload::ApplyCrdt {
        entity_id,
        field_key: "doc".into(),
        crdt_type: CrdtType::Text,
        delta: vec![1, 2, 3],
    }];
    alice.engine.execute(BundleType::UserEdit, alice_crdt_payloads)?;

    let bob_crdt_payloads = vec![OperationPayload::ApplyCrdt {
        entity_id,
        field_key: "doc".into(),
        crdt_type: CrdtType::Text,
        delta: vec![4, 5, 6],
    }];
    bob.engine.execute(BundleType::UserEdit, bob_crdt_payloads)?;

    // Sync Alice's CRDT op to Bob → no conflict
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert!(conflicts.is_empty(), "CRDT ops should not produce conflicts");

    Ok(())
}

// ============================================================================
// Batch 3: Overlay Core Tests
// ============================================================================

#[test]
fn overlay_create_makes_it_active() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    assert!(peer.engine.active_overlay().is_none());
    let overlay_id = peer.engine.create_overlay("draft")?;
    assert_eq!(peer.engine.active_overlay(), Some(overlay_id));

    Ok(())
}

#[test]
fn overlay_routes_writes_away_from_canonical() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;
    let op_count_before = peer.engine.op_count()?;

    // Create overlay and edit within it
    let _overlay_id = peer.engine.create_overlay("draft")?;
    peer.set_field(entity_id, "name", FieldValue::Text("overlay_value".into()))?;

    // Canonical op count should NOT have increased (overlay write doesn't go to oplog)
    let op_count_after = peer.engine.op_count()?;
    assert_eq!(op_count_before, op_count_after, "overlay write should not add to canonical oplog");

    // Canonical field should still be "original"
    let canonical_val = peer.engine.storage().get_field(entity_id, "name")?;
    assert_eq!(canonical_val, Some(FieldValue::Text("original".into())));

    Ok(())
}

#[test]
fn overlay_query_shows_overlay_value() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;

    let _overlay_id = peer.engine.create_overlay("draft")?;
    peer.set_field(entity_id, "name", FieldValue::Text("overlay_value".into()))?;

    // Engine query should show overlay value
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("overlay_value".into())));

    // get_fields should also reflect overlay
    let fields = peer.engine.get_fields(entity_id)?;
    let name_field = fields.iter().find(|(k, _)| k == "name");
    assert_eq!(name_field.map(|(_, v)| v.clone()), Some(FieldValue::Text("overlay_value".into())));

    Ok(())
}

#[test]
fn overlay_falls_through_to_canonical_for_unmodified_fields() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![
        ("name", FieldValue::Text("original".into())),
        ("status", FieldValue::Text("open".into())),
    ])?;

    let _overlay_id = peer.engine.create_overlay("draft")?;
    // Only modify "name" in overlay, leave "status" untouched
    peer.set_field(entity_id, "name", FieldValue::Text("overlay_name".into()))?;

    // "status" should fall through to canonical
    let status = peer.engine.get_field(entity_id, "status")?;
    assert_eq!(status, Some(FieldValue::Text("open".into())));

    // get_fields should show overlay "name" + canonical "status"
    let fields = peer.engine.get_fields(entity_id)?;
    assert_eq!(fields.len(), 2);
    let name = fields.iter().find(|(k, _)| k == "name").map(|(_, v)| v.clone());
    let status = fields.iter().find(|(k, _)| k == "status").map(|(_, v)| v.clone());
    assert_eq!(name, Some(FieldValue::Text("overlay_name".into())));
    assert_eq!(status, Some(FieldValue::Text("open".into())));

    Ok(())
}

#[test]
fn overlay_stash_deactivates() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;

    let overlay_id = peer.engine.create_overlay("draft")?;
    peer.set_field(entity_id, "name", FieldValue::Text("overlay_value".into()))?;

    // Stash
    peer.engine.stash_overlay(overlay_id)?;
    assert!(peer.engine.active_overlay().is_none());

    // After stash, queries should return canonical values
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("original".into())));

    // Should appear in stashed list
    let stashed = peer.engine.stashed_overlays()?;
    assert_eq!(stashed.len(), 1);
    assert_eq!(stashed[0].0, overlay_id);
    assert_eq!(stashed[0].1, "draft");

    Ok(())
}

#[test]
fn overlay_activate_auto_stashes_current() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    let overlay_a = peer.engine.create_overlay("A")?;
    assert_eq!(peer.engine.active_overlay(), Some(overlay_a));

    // Create B — should auto-stash A
    let overlay_b = peer.engine.create_overlay("B")?;
    assert_eq!(peer.engine.active_overlay(), Some(overlay_b));

    let stashed = peer.engine.stashed_overlays()?;
    assert_eq!(stashed.len(), 1);
    assert_eq!(stashed[0].0, overlay_a);

    // Activate A — should auto-stash B
    peer.engine.activate_overlay(overlay_a)?;
    assert_eq!(peer.engine.active_overlay(), Some(overlay_a));

    let stashed = peer.engine.stashed_overlays()?;
    assert_eq!(stashed.len(), 1);
    assert_eq!(stashed[0].0, overlay_b);

    Ok(())
}

#[test]
fn overlay_discard_removes_overlay_and_ops() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;

    let overlay_id = peer.engine.create_overlay("draft")?;
    peer.set_field(entity_id, "name", FieldValue::Text("overlay_value".into()))?;

    // Discard
    peer.engine.discard_overlay(overlay_id)?;
    assert!(peer.engine.active_overlay().is_none());

    // Queries should return canonical values
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("original".into())));

    // Overlay should not be in stashed list
    let stashed = peer.engine.stashed_overlays()?;
    assert!(stashed.is_empty());

    Ok(())
}

#[test]
fn overlay_undo_removes_op_falls_through_to_canonical() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;

    let _overlay_id = peer.engine.create_overlay("draft")?;
    peer.set_field(entity_id, "name", FieldValue::Text("overlay_value".into()))?;

    // Verify overlay value is active
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("overlay_value".into())));

    // Undo overlay op
    let undone = peer.engine.overlay_undo()?;
    assert!(undone);

    // Should fall through to canonical
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("original".into())));

    // Redo should bring it back
    let redone = peer.engine.overlay_redo()?;
    assert!(redone);
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("overlay_value".into())));

    Ok(())
}

#[test]
fn no_overlay_normal_canonical_flow() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    assert!(peer.engine.active_overlay().is_none());

    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;
    let op_count = peer.engine.op_count()?;
    assert!(op_count > 0, "canonical writes should produce ops");

    peer.set_field(entity_id, "name", FieldValue::Text("updated".into()))?;
    let op_count2 = peer.engine.op_count()?;
    assert!(op_count2 > op_count, "canonical edits should increase op count");

    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("updated".into())));

    Ok(())
}

// ============================================================================
// Batch 4: Overlay Commit + Canonical Drift Tests
// ============================================================================

#[test]
fn commit_overlay_produces_canonical_bundle() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;
    let op_count_before = peer.engine.op_count()?;

    // Create overlay and edit
    let overlay_id = peer.engine.create_overlay("draft")?;
    peer.set_field(entity_id, "name", FieldValue::Text("committed_value".into()))?;

    // Op count should NOT have changed yet (overlay write)
    assert_eq!(peer.engine.op_count()?, op_count_before);

    // Commit overlay
    let bundle_id = peer.engine.commit_overlay(overlay_id)?;

    // Op count should now have increased
    let op_count_after = peer.engine.op_count()?;
    assert!(op_count_after > op_count_before, "commit should produce canonical ops");

    // The committed ops should appear in the oplog
    let bundle_ops = peer.engine.get_ops_by_bundle(bundle_id)?;
    assert!(!bundle_ops.is_empty());

    // Verify the SetField op is present with correct value
    let set_field_op = bundle_ops.iter().find(|op| {
        matches!(&op.payload, OperationPayload::SetField { field_key, .. } if field_key == "name")
    });
    assert!(set_field_op.is_some(), "committed bundle should contain SetField op");

    // Canonical value should reflect the commit
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("committed_value".into())));

    Ok(())
}

#[test]
fn commit_overlay_is_atomic() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![
        ("name", FieldValue::Text("original".into())),
        ("status", FieldValue::Text("open".into())),
    ])?;

    // Create overlay with multiple edits
    let overlay_id = peer.engine.create_overlay("draft")?;
    peer.set_field(entity_id, "name", FieldValue::Text("new_name".into()))?;
    peer.set_field(entity_id, "status", FieldValue::Text("closed".into()))?;

    // Commit
    let bundle_id = peer.engine.commit_overlay(overlay_id)?;

    // All ops should be in one bundle
    let bundle_ops = peer.engine.get_ops_by_bundle(bundle_id)?;
    assert_eq!(bundle_ops.len(), 2, "both SetField ops should be in the same bundle");

    // Both fields should reflect committed values
    let name = peer.engine.get_field(entity_id, "name")?;
    let status = peer.engine.get_field(entity_id, "status")?;
    assert_eq!(name, Some(FieldValue::Text("new_name".into())));
    assert_eq!(status, Some(FieldValue::Text("closed".into())));

    Ok(())
}

#[test]
fn commit_blocked_by_unresolved_drift() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Bob creates overlay and edits field
    let overlay_id = bob.engine.create_overlay("draft")?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob_overlay".into()))?;

    // Alice edits canonically and syncs to Bob → causes drift on Bob's overlay
    alice.set_field(entity_id, "name", FieldValue::Text("alice_canonical".into()))?;
    let _conflicts = sync_latest_bundle(&alice, &mut bob)?;

    // Bob's overlay should have drift
    assert!(bob.engine.has_unresolved_drift(overlay_id)?);

    // Commit should be blocked
    let result = bob.engine.commit_overlay(overlay_id);
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("drift"), "error should mention drift: {err_msg}");

    Ok(())
}

#[test]
fn canonical_drift_detected_on_foreign_bundle() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Bob creates overlay editing the field
    let overlay_id = bob.engine.create_overlay("draft")?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob_overlay".into()))?;

    // No drift yet
    let drift = bob.engine.check_drift(overlay_id)?;
    assert!(drift.is_empty(), "no drift before foreign bundle");

    // Alice modifies the same field canonically
    alice.set_field(entity_id, "name", FieldValue::Text("alice_canonical".into()))?;

    // Sync Alice's edit to Bob → should trigger drift detection
    let _conflicts = sync_latest_bundle(&alice, &mut bob)?;

    // Now Bob's overlay should have drift
    let drift = bob.engine.check_drift(overlay_id)?;
    assert_eq!(drift.len(), 1);
    assert_eq!(drift[0].entity_id, entity_id);
    assert_eq!(drift[0].field_key, "name");
    assert_eq!(drift[0].overlay_value, Some(FieldValue::Text("bob_overlay".into())));
    assert_eq!(drift[0].canonical_value, Some(FieldValue::Text("alice_canonical".into())));

    Ok(())
}

#[test]
fn keep_mine_clears_drift_commit_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Bob creates overlay
    let overlay_id = bob.engine.create_overlay("draft")?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob_overlay".into()))?;

    // Alice edits and syncs → drift
    alice.set_field(entity_id, "name", FieldValue::Text("alice_canonical".into()))?;
    let _conflicts = sync_latest_bundle(&alice, &mut bob)?;

    assert!(bob.engine.has_unresolved_drift(overlay_id)?);

    // Acknowledge drift ("Keep Mine")
    bob.engine.acknowledge_drift(overlay_id, entity_id, "name")?;

    // Drift should be cleared
    assert!(!bob.engine.has_unresolved_drift(overlay_id)?);
    let drift = bob.engine.check_drift(overlay_id)?;
    assert!(drift.is_empty());

    // Commit should now succeed
    let bundle_id = bob.engine.commit_overlay(overlay_id)?;
    let bundle_ops = bob.engine.get_ops_by_bundle(bundle_id)?;
    assert!(!bundle_ops.is_empty());

    // Bob's overlay value should be canonical now
    let val = bob.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("bob_overlay".into())));

    Ok(())
}

#[test]
fn knockout_removes_overlay_op() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;

    // Create overlay and edit
    let overlay_id = peer.engine.create_overlay("draft")?;
    peer.set_field(entity_id, "name", FieldValue::Text("overlay_value".into()))?;

    // Verify overlay value is active
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("overlay_value".into())));

    // Knockout the field
    peer.engine.knockout_field(overlay_id, entity_id, "name")?;

    // Should fall through to canonical value
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("original".into())));

    Ok(())
}

#[test]
fn no_drift_on_different_field() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Bob overlays edit on "name"
    let overlay_id = bob.engine.create_overlay("draft")?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob_overlay".into()))?;

    // Alice edits "status" (different field) and syncs to Bob
    alice.set_field(entity_id, "status", FieldValue::Text("active".into()))?;
    let _conflicts = sync_latest_bundle(&alice, &mut bob)?;

    // No drift should be detected (different field)
    let drift = bob.engine.check_drift(overlay_id)?;
    assert!(drift.is_empty(), "editing different field should not cause drift");
    assert!(!bob.engine.has_unresolved_drift(overlay_id)?);

    Ok(())
}

#[test]
fn discard_after_drift_works() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Bob creates overlay
    let overlay_id = bob.engine.create_overlay("draft")?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob_overlay".into()))?;

    // Alice edits and syncs → drift
    alice.set_field(entity_id, "name", FieldValue::Text("alice_canonical".into()))?;
    let _conflicts = sync_latest_bundle(&alice, &mut bob)?;

    assert!(bob.engine.has_unresolved_drift(overlay_id)?);

    // Discard should work even with drift
    bob.engine.discard_overlay(overlay_id)?;

    // No active overlay
    assert!(bob.engine.active_overlay().is_none());

    // Value should be Alice's canonical value
    let val = bob.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("alice_canonical".into())));

    Ok(())
}

#[test]
fn overlay_commit_updates_conflicted_field() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Create a conflict
    alice.set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob".into()))?;
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert_eq!(conflicts.len(), 1);

    // Bob creates overlay and edits the conflicted field
    let overlay_id = bob.engine.create_overlay("fix")?;
    bob.set_field(entity_id, "name", FieldValue::Text("overlay_fix".into()))?;

    // Commit the overlay
    let _bundle_id = bob.engine.commit_overlay(overlay_id)?;

    // The canonical value should now be the overlay's value
    let val = bob.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("overlay_fix".into())));

    Ok(())
}

#[test]
fn commit_overlay_a_drifts_stashed_overlay_b() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;

    // Create overlay A — edit "name"
    let overlay_a = peer.engine.create_overlay("A")?;
    peer.set_field(entity_id, "name", FieldValue::Text("value_a".into()))?;

    // Create overlay B — auto-stashes A — also edit "name"
    let overlay_b = peer.engine.create_overlay("B")?;
    peer.set_field(entity_id, "name", FieldValue::Text("value_b".into()))?;

    // Verify A is stashed and has no drift yet
    assert!(!peer.engine.has_unresolved_drift(overlay_a)?);

    // Commit B → should cause drift on stashed A
    let _bundle_id = peer.engine.commit_overlay(overlay_b)?;

    // Stashed overlay A should now have drift
    assert!(peer.engine.has_unresolved_drift(overlay_a)?);
    let drift = peer.engine.check_drift(overlay_a)?;
    assert_eq!(drift.len(), 1);
    assert_eq!(drift[0].entity_id, entity_id);
    assert_eq!(drift[0].field_key, "name");
    assert_eq!(drift[0].overlay_value, Some(FieldValue::Text("value_a".into())));
    assert_eq!(drift[0].canonical_value, Some(FieldValue::Text("value_b".into())));

    Ok(())
}

// ============================================================================
// Batch 1 Fixes: Additional Tests
// ============================================================================

#[test]
fn resolve_conflict_with_none_clears_field() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    // Create conflict
    alice.set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob".into()))?;
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    assert_eq!(conflicts.len(), 1);
    let conflict_id = conflicts[0].conflict_id;

    // Resolve with None (clear the field)
    bob.engine.resolve_conflict(conflict_id, None)?;

    // Field should be gone
    let val = bob.engine.get_field(entity_id, "name")?;
    assert_eq!(val, None, "resolving with None should clear the field");

    // Metadata should still exist (tombstone)
    let meta = bob.engine.get_field_metadata(entity_id, "name")?;
    assert!(meta.is_some(), "tombstone metadata should exist after resolve-as-clear");

    Ok(())
}

#[test]
fn resolve_conflict_survives_rebuild() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    let entity_id = setup_shared_entity(&mut alice, &mut bob, "name", FieldValue::Text("original".into()))?;

    alice.set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    bob.set_field(entity_id, "name", FieldValue::Text("bob".into()))?;
    let conflicts = sync_latest_bundle(&alice, &mut bob)?;
    let conflict_id = conflicts[0].conflict_id;

    let chosen = FieldValue::Text("final_answer".into());
    bob.engine.resolve_conflict(conflict_id, Some(chosen.clone()))?;

    // Verify value before rebuild
    let val_before = bob.engine.get_field(entity_id, "name")?;
    assert_eq!(val_before, Some(chosen.clone()));

    // Rebuild from oplog
    bob.engine.storage_mut().rebuild_from_oplog()?;

    // Verify value after rebuild
    let val_after = bob.engine.get_field(entity_id, "name")?;
    assert_eq!(val_after, Some(chosen), "resolved value should survive rebuild_from_oplog");

    Ok(())
}

/// Regression test: acknowledge_drift on one field must NOT corrupt
/// canonical_value_at_creation for a different field on the same entity.
#[test]
fn acknowledge_drift_does_not_corrupt_other_fields() -> Result<(), Box<dyn std::error::Error>> {
    let mut alice = TestPeer::new()?;
    let mut bob = TestPeer::new()?;

    // 1. Alice creates entity with "name" and "status" fields, replicate to Bob
    let entity_id = alice.create_record("Task", vec![
        ("name", FieldValue::Text("original-name".into())),
        ("status", FieldValue::Text("open".into())),
    ])?;
    sync_latest_bundle(&alice, &mut bob)?;

    // 2. Alice creates overlay and edits both fields
    let overlay_id = alice.engine.create_overlay("feature-branch")?;
    alice.engine.set_field(entity_id, "name", FieldValue::Text("overlay-name".into()))?;
    alice.engine.set_field(entity_id, "status", FieldValue::Text("closed".into()))?;

    // 3. Stash overlay so we can cause canonical drift
    alice.engine.stash_overlay(overlay_id)?;

    // 4. Bob edits both fields, sync to Alice to cause drift
    bob.engine.set_field(entity_id, "name", FieldValue::Text("bob-name".into()))?;
    bob.engine.set_field(entity_id, "status", FieldValue::Text("in-progress".into()))?;

    // Sync Bob's edits to Alice
    let ops = bob.engine.get_ops_canonical()?;
    // Find the last two bundle_ids (bob's two edits)
    let mut seen_bundles = Vec::new();
    for op in ops.iter().rev() {
        if !seen_bundles.contains(&op.bundle_id) {
            seen_bundles.push(op.bundle_id);
        }
        if seen_bundles.len() == 2 {
            break;
        }
    }
    // Ingest Bob's bundles into Alice
    for &bid in seen_bundles.iter().rev() {
        let bundle_ops = bob.engine.get_ops_by_bundle(bid)?;
        let vc = bob.engine.storage().get_bundle_vector_clock(bid)?;
        let bundle = Bundle::new_signed(
            bid,
            bob.engine.identity(),
            bundle_ops[0].hlc,
            BundleType::UserEdit,
            &bundle_ops,
            vc,
        )?;
        alice.engine.ingest_bundle(&bundle, &bundle_ops)?;
    }

    // 5. Activate overlay — both fields should be drifted
    alice.engine.activate_overlay(overlay_id)?;
    let drift = alice.engine.check_drift(overlay_id)?;
    assert_eq!(drift.len(), 2, "both name and status should have drifted");

    // 6. Acknowledge drift on "name" ONLY
    alice.engine.acknowledge_drift(overlay_id, entity_id, "name")?;

    // 7. Verify "status" still has unresolved drift
    let drift_after = alice.engine.check_drift(overlay_id)?;
    assert_eq!(drift_after.len(), 1, "only status should still be drifted");
    assert_eq!(drift_after[0].field_key, "status", "status field should still show drift");

    // 8. Verify that acknowledge_drift didn't overwrite the canonical_value_at_creation for "status"
    //    by checking that the overlay ops for "status" still reflect the OLD canonical value (before Bob's edit)
    //    We can verify this indirectly: after acknowledging "name", the overlay should still block commit
    //    because "status" drift is unresolved
    assert!(alice.engine.has_unresolved_drift(overlay_id)?, "should still have unresolved drift for status");

    Ok(())
}

#[test]
fn lww_tiebreak_by_op_id_larger_wins() -> Result<(), Box<dyn std::error::Error>> {
    // When two ops have the exact same HLC, the one with the larger op_id wins.
    // We control this by creating ops and checking which op_id is larger.
    let identity = ActorIdentity::generate();
    let mut storage = SqliteStorage::open_in_memory()?;

    let entity_id = EntityId::new();
    let hlc = Hlc::new(1000, 0);
    let same_hlc = Hlc::new(2000, 0);

    // Create entity
    let bid1 = BundleId::new();
    let create_op = Operation::new_signed(&identity, hlc, bid1, BTreeMap::new(),
        OperationPayload::CreateEntity { entity_id, initial_table: None })?;
    let b1 = Bundle::new_signed(bid1, &identity, hlc, BundleType::UserEdit, std::slice::from_ref(&create_op), None)?;
    storage.append_bundle(&b1, std::slice::from_ref(&create_op))?;

    // Two SetFields with identical HLC — track which op_id is larger
    let bid2 = BundleId::new();
    let set_a = Operation::new_signed(&identity, same_hlc, bid2, BTreeMap::new(),
        OperationPayload::SetField { entity_id, field_key: "x".into(), value: FieldValue::Text("A".into()) })?;
    let bid3 = BundleId::new();
    let set_b = Operation::new_signed(&identity, same_hlc, bid3, BTreeMap::new(),
        OperationPayload::SetField { entity_id, field_key: "x".into(), value: FieldValue::Text("B".into()) })?;

    // Determine expected winner by op_id comparison
    let expected_winner = if set_a.op_id.as_bytes() > set_b.op_id.as_bytes() {
        FieldValue::Text("A".into())
    } else {
        FieldValue::Text("B".into())
    };

    // Ingest both
    let b2 = Bundle::new_signed(bid2, &identity, same_hlc, BundleType::UserEdit, std::slice::from_ref(&set_a), None)?;
    storage.append_bundle(&b2, std::slice::from_ref(&set_a))?;
    let b3 = Bundle::new_signed(bid3, &identity, same_hlc, BundleType::UserEdit, std::slice::from_ref(&set_b), None)?;
    storage.append_bundle(&b3, std::slice::from_ref(&set_b))?;

    let val = storage.get_field(entity_id, "x")?;
    assert_eq!(val, Some(expected_winner.clone()), "larger op_id should win when HLC is equal");

    // Also verify rebuild produces same result
    storage.rebuild_from_oplog()?;
    let val_after = storage.get_field(entity_id, "x")?;
    assert_eq!(val_after, Some(expected_winner), "tiebreak should be deterministic after rebuild");

    Ok(())
}

// ============================================================================
// Batch 5: TestNetwork + TestPeer Integration Tests
// ============================================================================

#[test]
fn network_sync_to_transfers_bundles() -> Result<(), Box<dyn std::error::Error>> {
    let mut net = TestNetwork::new();
    let a = net.add_peer()?;
    let b = net.add_peer()?;

    // Peer A creates entity
    let entity_id = net.peer_mut(a).create_record("Task", vec![("name", FieldValue::Text("hello".into()))])?;

    // Sync A → B
    let conflicts = net.sync_to(a, b)?;
    assert!(conflicts.is_empty());

    // B should see the entity and field
    let val = net.peer(b).engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("hello".into())));

    Ok(())
}

#[test]
fn network_sync_pair_bidirectional() -> Result<(), Box<dyn std::error::Error>> {
    let mut net = TestNetwork::new();
    let a = net.add_peer()?;
    let b = net.add_peer()?;

    // A creates entity with field
    let entity_id = net.peer_mut(a).create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;
    net.sync_to(a, b)?;

    // Both peers edit different fields offline
    net.peer_mut(a).set_field(entity_id, "name", FieldValue::Text("alice_name".into()))?;
    net.peer_mut(b).set_field(entity_id, "status", FieldValue::Text("active".into()))?;

    // Bidirectional sync
    let conflicts = net.sync_pair(a, b)?;
    assert!(conflicts.is_empty(), "different fields should not conflict");

    // Both peers should have both fields
    let a_name = net.peer(a).engine.get_field(entity_id, "name")?;
    let a_status = net.peer(a).engine.get_field(entity_id, "status")?;
    let b_name = net.peer(b).engine.get_field(entity_id, "name")?;
    let b_status = net.peer(b).engine.get_field(entity_id, "status")?;
    assert_eq!(a_name, Some(FieldValue::Text("alice_name".into())));
    assert_eq!(a_status, Some(FieldValue::Text("active".into())));
    assert_eq!(b_name, a_name);
    assert_eq!(b_status, a_status);

    Ok(())
}

#[test]
fn network_sync_all_convergence() -> Result<(), Box<dyn std::error::Error>> {
    let mut net = TestNetwork::new();
    let a = net.add_peer()?;
    let b = net.add_peer()?;
    let c = net.add_peer()?;

    // A creates entity, sync to all
    let entity_id = net.peer_mut(a).create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;
    net.sync_all()?;

    // Each peer edits a different field offline
    net.peer_mut(a).set_field(entity_id, "name", FieldValue::Text("from_a".into()))?;
    net.peer_mut(b).set_field(entity_id, "status", FieldValue::Text("from_b".into()))?;
    net.peer_mut(c).set_field(entity_id, "priority", FieldValue::Text("from_c".into()))?;

    // Full mesh sync
    let _conflicts = net.sync_all()?;

    // All peers should converge
    for idx in [a, b, c] {
        let name = net.peer(idx).engine.get_field(entity_id, "name")?;
        let status = net.peer(idx).engine.get_field(entity_id, "status")?;
        let priority = net.peer(idx).engine.get_field(entity_id, "priority")?;
        assert_eq!(name, Some(FieldValue::Text("from_a".into())));
        assert_eq!(status, Some(FieldValue::Text("from_b".into())));
        assert_eq!(priority, Some(FieldValue::Text("from_c".into())));
    }

    // All vector clocks should match
    let vc_a = net.peer(a).engine.get_vector_clock()?;
    let vc_b = net.peer(b).engine.get_vector_clock()?;
    let vc_c = net.peer(c).engine.get_vector_clock()?;
    assert_eq!(vc_a, vc_b);
    assert_eq!(vc_b, vc_c);

    Ok(())
}

#[test]
fn network_sync_detects_conflicts() -> Result<(), Box<dyn std::error::Error>> {
    let mut net = TestNetwork::new();
    let a = net.add_peer()?;
    let b = net.add_peer()?;

    // Setup shared entity
    let entity_id = net.peer_mut(a).create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;
    net.sync_to(a, b)?;

    // Concurrent edits
    net.peer_mut(a).set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    net.peer_mut(b).set_field(entity_id, "name", FieldValue::Text("bob".into()))?;

    // Sync → conflict
    let conflicts = net.sync_to(a, b)?;
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].entity_id, entity_id);
    assert_eq!(conflicts[0].field_key, "name");
    assert_eq!(conflicts[0].status, ConflictStatus::Open);

    Ok(())
}

#[test]
fn network_sync_all_three_way_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let mut net = TestNetwork::new();
    let a = net.add_peer()?;
    let b = net.add_peer()?;
    let c = net.add_peer()?;

    // Setup shared entity on all peers
    let entity_id = net.peer_mut(a).create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;
    net.sync_all()?;

    // All three edit the same field offline
    net.peer_mut(a).set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    net.peer_mut(b).set_field(entity_id, "name", FieldValue::Text("bob".into()))?;
    net.peer_mut(c).set_field(entity_id, "name", FieldValue::Text("charlie".into()))?;

    // Full mesh sync — should detect 3-way conflict
    let _conflicts = net.sync_all()?;

    // All peers should have the same open conflict
    for idx in [a, b, c] {
        let open = net.peer(idx).engine.get_open_conflicts_for_entity(entity_id)?;
        assert_eq!(open.len(), 1, "peer {idx} should have exactly one open conflict");
        assert_eq!(open[0].values.len(), 3, "peer {idx} should have 3 branch tips");
    }

    Ok(())
}

#[test]
fn peer_convenience_overlay_lifecycle() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;

    // Full lifecycle using convenience methods
    let overlay_id = peer.create_overlay("draft")?;
    peer.set_field(entity_id, "name", FieldValue::Text("overlay_edit".into()))?;

    // Stash and recall
    peer.stash_overlay(overlay_id)?;
    assert!(peer.engine.active_overlay().is_none());

    peer.engine.activate_overlay(overlay_id)?;
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("overlay_edit".into())));

    // Commit
    let bundle_id = peer.commit_overlay(overlay_id)?;
    let ops = peer.engine.get_ops_by_bundle(bundle_id)?;
    assert!(!ops.is_empty());

    // Canonical value updated
    let val = peer.engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("overlay_edit".into())));

    Ok(())
}

#[test]
fn peer_convenience_conflict_resolution() -> Result<(), Box<dyn std::error::Error>> {
    let mut net = TestNetwork::new();
    let a = net.add_peer()?;
    let b = net.add_peer()?;

    let entity_id = net.peer_mut(a).create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;
    net.sync_to(a, b)?;

    // Concurrent edits
    net.peer_mut(a).set_field(entity_id, "name", FieldValue::Text("alice".into()))?;
    net.peer_mut(b).set_field(entity_id, "name", FieldValue::Text("bob".into()))?;

    let conflicts = net.sync_to(a, b)?;
    let conflict_id = conflicts[0].conflict_id;

    // Resolve using convenience method
    let open = net.peer(b).get_open_conflicts(entity_id)?;
    assert_eq!(open.len(), 1);

    let _bundle_id = net.peer_mut(b).resolve_conflict(conflict_id, Some(FieldValue::Text("resolved".into())))?;

    let val = net.peer(b).engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("resolved".into())));

    let open = net.peer(b).get_open_conflicts(entity_id)?;
    assert!(open.is_empty());

    Ok(())
}

#[test]
fn network_sync_with_overlay_causes_drift() -> Result<(), Box<dyn std::error::Error>> {
    let mut net = TestNetwork::new();
    let a = net.add_peer()?;
    let b = net.add_peer()?;

    let entity_id = net.peer_mut(a).create_record("Task", vec![("name", FieldValue::Text("original".into()))])?;
    net.sync_to(a, b)?;

    // B creates overlay and edits
    let overlay_id = net.peer_mut(b).create_overlay("draft")?;
    net.peer_mut(b).set_field(entity_id, "name", FieldValue::Text("overlay_value".into()))?;

    // A edits canonically
    net.peer_mut(a).set_field(entity_id, "name", FieldValue::Text("canonical_update".into()))?;

    // Sync A → B causes drift on B's overlay
    let _conflicts = net.sync_to(a, b)?;

    let drift = net.peer(b).check_drift(overlay_id)?;
    assert_eq!(drift.len(), 1);
    assert_eq!(drift[0].field_key, "name");
    assert_eq!(drift[0].overlay_value, Some(FieldValue::Text("overlay_value".into())));
    assert_eq!(drift[0].canonical_value, Some(FieldValue::Text("canonical_update".into())));

    // Acknowledge drift and commit
    net.peer_mut(b).acknowledge_drift(overlay_id, entity_id, "name")?;
    let _bundle_id = net.peer_mut(b).commit_overlay(overlay_id)?;

    let val = net.peer(b).engine.get_field(entity_id, "name")?;
    assert_eq!(val, Some(FieldValue::Text("overlay_value".into())));

    Ok(())
}

// ============================================================================
// Additional Error + Edge Property LWW + Idempotency Tests
// ============================================================================

#[test]
fn error_commit_empty_overlay() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Create overlay but don't add any ops
    let overlay_id = peer.create_overlay("empty-draft")?;

    // Try to commit — should fail with EmptyOverlay
    let result = peer.engine.commit_overlay(overlay_id);
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("empty"),
        "error should mention 'empty': {err_msg}"
    );

    Ok(())
}

#[test]
fn edge_property_lww_older_set_does_not_overwrite_newer() -> Result<(), Box<dyn std::error::Error>> {
    let mut net = TestNetwork::new();
    let a = net.add_peer()?;
    let b = net.add_peer()?;

    // A creates entity + edge, sync to B
    let entity_a = net.peer_mut(a).create_record("Node", vec![])?;
    let entity_b_node = net.peer_mut(a).create_record("Node", vec![])?;
    let edge_id = net.peer_mut(a).create_edge("link", entity_a, entity_b_node)?;
    net.sync_to(a, b)?;

    // A sets edge property (will have a newer HLC since A acts after B)
    // B sets same edge property first (older HLC)
    net.peer_mut(b).set_edge_property(edge_id, "weight", FieldValue::Integer(10))?;
    net.peer_mut(a).set_edge_property(edge_id, "weight", FieldValue::Integer(99))?;

    // Sync B -> A (B's older set arrives at A which already has newer value)
    let _conflicts = net.sync_to(b, a)?;

    // A should still have its own newer value (99), not B's older value (10)
    let val = net.peer(a).engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(
        val,
        Some(FieldValue::Integer(99)),
        "newer edge property set should not be overwritten by older"
    );

    Ok(())
}

#[test]
fn edge_property_lww_clear_older_does_not_delete_newer_set() -> Result<(), Box<dyn std::error::Error>> {
    let mut net = TestNetwork::new();
    let a = net.add_peer()?;
    let b = net.add_peer()?;

    // A creates entity + edge with initial property, sync to B
    let entity_a = net.peer_mut(a).create_record("Node", vec![])?;
    let entity_b_node = net.peer_mut(a).create_record("Node", vec![])?;
    let edge_id = net.peer_mut(a).create_edge_with_properties(
        "link",
        entity_a,
        entity_b_node,
        vec![("weight", FieldValue::Integer(5))],
    )?;
    net.sync_to(a, b)?;

    // B clears the property (older HLC)
    net.peer_mut(b).clear_edge_property(edge_id, "weight")?;
    // A sets the property to a new value (newer HLC)
    net.peer_mut(a).set_edge_property(edge_id, "weight", FieldValue::Integer(42))?;

    // Sync B -> A (B's older clear arrives at A which has newer set)
    let _conflicts = net.sync_to(b, a)?;

    // A should still have 42 — the older clear tombstone should NOT win
    let val = net.peer(a).engine.get_edge_property(edge_id, "weight")?;
    assert_eq!(
        val,
        Some(FieldValue::Integer(42)),
        "older ClearEdgeProperty tombstone should not delete newer SetEdgeProperty"
    );

    Ok(())
}

#[test]
fn idempotent_bundle_ingestion() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = TestPeer::new()?;

    // Perform some operations
    let entity_id = peer.create_record("Task", vec![("name", FieldValue::Text("test".into()))])?;
    peer.set_field(entity_id, "status", FieldValue::Text("active".into()))?;

    // Capture state before re-ingestion attempt
    let op_count_before = peer.engine.op_count()?;
    let val_name_before = peer.engine.get_field(entity_id, "name")?;
    let val_status_before = peer.engine.get_field(entity_id, "status")?;

    // Extract the latest bundle
    let ops = peer.engine.get_ops_canonical()?;
    let last_op = ops.last().unwrap();
    let bundle_id = last_op.bundle_id;
    let bundle_ops = peer.engine.get_ops_by_bundle(bundle_id)?;
    let vc = peer.engine.storage().get_bundle_vector_clock(bundle_id)?;
    let bundle = Bundle::new_signed(
        bundle_id,
        peer.engine.identity(),
        last_op.hlc,
        BundleType::UserEdit,
        &bundle_ops,
        vc,
    )?;

    // Re-ingesting the same bundle should be idempotent (silently accepted)
    let result = peer.engine.ingest_bundle(&bundle, &bundle_ops);
    assert!(result.is_ok(), "re-ingesting duplicate bundle should succeed silently");

    // State should be unchanged after idempotent re-ingestion
    let op_count_after = peer.engine.op_count()?;
    assert_eq!(op_count_before, op_count_after, "op count should not change after duplicate ingestion");

    let val_name_after = peer.engine.get_field(entity_id, "name")?;
    let val_status_after = peer.engine.get_field(entity_id, "status")?;
    assert_eq!(val_name_before, val_name_after);
    assert_eq!(val_status_before, val_status_after);

    Ok(())
}
