use std::collections::BTreeSet;

use openprod_core::{
    hlc::Hlc,
    ids::*,
    operations::{Bundle, BundleType, Operation},
    vector_clock::VectorClock,
};
use openprod_storage::{ConflictRecord, Storage, StorageError};

use crate::TestPeer;

pub struct TestNetwork {
    peers: Vec<TestPeer>,
}

impl Default for TestNetwork {
    fn default() -> Self {
        Self::new()
    }
}

impl TestNetwork {
    pub fn new() -> Self {
        Self { peers: Vec::new() }
    }

    pub fn add_peer(&mut self) -> Result<usize, StorageError> {
        let peer = TestPeer::new()?;
        let index = self.peers.len();
        self.peers.push(peer);
        Ok(index)
    }

    pub fn peer(&self, index: usize) -> &TestPeer {
        &self.peers[index]
    }

    pub fn peer_mut(&mut self, index: usize) -> &mut TestPeer {
        &mut self.peers[index]
    }

    /// Sync bundles from peer `from_idx` to peer `to_idx`.
    /// Uses vector clock diff to determine what needs syncing.
    /// Returns any conflicts detected during ingestion.
    pub fn sync_to(
        &mut self,
        from_idx: usize,
        to_idx: usize,
    ) -> Result<Vec<ConflictRecord>, Box<dyn std::error::Error>> {
        // 1. Extract vector clock from `to` and canonical ops from `from` (immutable borrows)
        let to_vc = self.peers[to_idx].engine.get_vector_clock()?;
        let from_ops = self.peers[from_idx].engine.get_ops_canonical()?;

        // 2. Find unseen bundle_ids: ops whose actor+hlc is ahead of `to`'s vector clock
        let mut unseen_bundle_ids = Vec::new();
        let mut seen = BTreeSet::new();
        for op in &from_ops {
            let is_new = match to_vc.get(&op.actor_id) {
                Some(max_hlc) => op.hlc > *max_hlc,
                None => true,
            };
            if is_new && seen.insert(op.bundle_id) {
                unseen_bundle_ids.push((op.bundle_id, op.hlc));
            }
        }

        // Sort by HLC for correct causal ingestion order
        unseen_bundle_ids.sort_by(|a, b| a.1.cmp(&b.1));

        // 3. Extract all bundle data from `from` peer into owned structures
        struct BundleData {
            bundle_id: BundleId,
            hlc: Hlc,
            ops: Vec<Operation>,
            vc: Option<VectorClock>,
        }

        let mut bundles_to_sync = Vec::new();
        for (bundle_id, hlc) in &unseen_bundle_ids {
            let ops = self.peers[from_idx].engine.get_ops_by_bundle(*bundle_id)?;
            let vc = self.peers[from_idx]
                .engine
                .storage()
                .get_bundle_vector_clock(*bundle_id)?;
            bundles_to_sync.push(BundleData {
                bundle_id: *bundle_id,
                hlc: *hlc,
                ops,
                vc,
            });
        }

        // 4. Build signed bundles (immutable borrow of `from` peer for identity)
        let mut signed_bundles: Vec<(Bundle, Vec<Operation>)> = Vec::new();
        for data in bundles_to_sync {
            let bundle = Bundle::new_signed(
                data.bundle_id,
                self.peers[from_idx].engine.identity(),
                data.hlc,
                BundleType::UserEdit,
                &data.ops,
                data.vc,
            )?;
            signed_bundles.push((bundle, data.ops));
        }

        // 5. Ingest into `to` peer (mutable borrow, no overlap with `from`)
        let mut all_conflicts = Vec::new();
        for (bundle, ops) in &signed_bundles {
            let conflicts = self.peers[to_idx].engine.ingest_bundle(bundle, ops)?;
            all_conflicts.extend(conflicts);
        }

        Ok(all_conflicts)
    }

    /// Bidirectional sync between two peers.
    /// Syncs a -> b, then b -> a. Returns all detected conflicts.
    pub fn sync_pair(
        &mut self,
        a: usize,
        b: usize,
    ) -> Result<Vec<ConflictRecord>, Box<dyn std::error::Error>> {
        let mut conflicts = self.sync_to(a, b)?;
        conflicts.extend(self.sync_to(b, a)?);
        Ok(conflicts)
    }

    /// Full mesh sync: repeat pairwise syncing until all peers are quiescent
    /// (all vector clocks are equal). Returns all detected conflicts.
    pub fn sync_all(&mut self) -> Result<Vec<ConflictRecord>, Box<dyn std::error::Error>> {
        let mut all_conflicts = Vec::new();
        let n = self.peers.len();
        if n <= 1 {
            return Ok(all_conflicts);
        }

        loop {
            let mut synced_any = false;
            for i in 0..n {
                for j in 0..n {
                    if i != j {
                        let conflicts = self.sync_to(i, j)?;
                        if !conflicts.is_empty() {
                            synced_any = true;
                        }
                        all_conflicts.extend(conflicts);
                    }
                }
            }

            // Check quiescence: all vector clocks should be equal
            let vc0 = self.peers[0].engine.get_vector_clock()?;
            let all_equal = (1..n).all(|i| {
                self.peers[i]
                    .engine
                    .get_vector_clock()
                    .map(|vc| vc == vc0)
                    .unwrap_or(false)
            });
            if all_equal || !synced_any {
                break;
            }
        }

        Ok(all_conflicts)
    }
}
