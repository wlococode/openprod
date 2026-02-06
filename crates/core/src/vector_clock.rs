use std::collections::BTreeMap;

use crate::hlc::Hlc;
use crate::ids::ActorId;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VectorClock {
    entries: BTreeMap<ActorId, Hlc>,
}

impl VectorClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the clock for a given actor. Keeps the max HLC per actor.
    pub fn update(&mut self, actor_id: ActorId, hlc: Hlc) {
        let entry = self.entries.entry(actor_id).or_insert(hlc);
        if hlc > *entry {
            *entry = hlc;
        }
    }

    /// Get the latest HLC for a given actor.
    pub fn get(&self, actor_id: &ActorId) -> Option<&Hlc> {
        self.entries.get(actor_id)
    }

    /// Merge another vector clock into this one (take max per actor).
    pub fn merge(&mut self, other: &VectorClock) {
        for (actor_id, hlc) in &other.entries {
            self.update(*actor_id, *hlc);
        }
    }

    /// Find actors where `other` has a newer HLC than we do.
    /// Returns (actor_id, our_hlc_or_None) for each such actor.
    pub fn diff(&self, other: &VectorClock) -> Vec<(ActorId, Option<Hlc>)> {
        let mut result = Vec::new();
        for (actor_id, other_hlc) in &other.entries {
            match self.entries.get(actor_id) {
                Some(our_hlc) if our_hlc >= other_hlc => {} // we're caught up
                Some(our_hlc) => result.push((*actor_id, Some(*our_hlc))),
                None => result.push((*actor_id, None)),
            }
        }
        result
    }

    /// Check if we've seen everything `other` has seen.
    pub fn covers(&self, other: &VectorClock) -> bool {
        self.diff(other).is_empty()
    }

    /// Iterate over all entries.
    pub fn entries(&self) -> &BTreeMap<ActorId, Hlc> {
        &self.entries
    }

    /// Serialize to msgpack bytes. Entries stored as Vec<(actor_bytes, hlc_bytes)>.
    pub fn to_msgpack(&self) -> Result<Vec<u8>, crate::CoreError> {
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = self
            .entries
            .iter()
            .map(|(actor, hlc)| (actor.as_bytes().to_vec(), hlc.to_bytes().to_vec()))
            .collect();
        rmp_serde::to_vec(&pairs).map_err(|e| crate::CoreError::Serialization(e.to_string()))
    }

    /// Deserialize from msgpack bytes.
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, crate::CoreError> {
        let pairs: Vec<(Vec<u8>, Vec<u8>)> =
            rmp_serde::from_slice(bytes).map_err(|e| crate::CoreError::Serialization(e.to_string()))?;
        let mut vc = VectorClock::new();
        for (actor_bytes, hlc_bytes) in pairs {
            let actor_arr: [u8; 32] = actor_bytes
                .try_into()
                .map_err(|_| crate::CoreError::Serialization("invalid actor_id length".into()))?;
            let hlc_arr: [u8; 12] = hlc_bytes
                .try_into()
                .map_err(|_| crate::CoreError::Serialization("invalid hlc length".into()))?;
            vc.update(
                crate::ids::ActorId::from_bytes(actor_arr),
                crate::hlc::Hlc::from_bytes(&hlc_arr),
            );
        }
        Ok(vc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(byte: u8) -> ActorId {
        ActorId::from_bytes([byte; 32])
    }

    #[test]
    fn update_tracks_max() {
        let mut vc = VectorClock::new();
        let a = actor(1);

        // Insert increasing HLCs
        vc.update(a, Hlc::new(100, 0));
        assert_eq!(vc.get(&a), Some(&Hlc::new(100, 0)));

        vc.update(a, Hlc::new(200, 0));
        assert_eq!(vc.get(&a), Some(&Hlc::new(200, 0)));

        vc.update(a, Hlc::new(300, 5));
        assert_eq!(vc.get(&a), Some(&Hlc::new(300, 5)));

        // Now update with a lower HLC — max should be preserved
        vc.update(a, Hlc::new(150, 0));
        assert_eq!(vc.get(&a), Some(&Hlc::new(300, 5)));

        // Same wall_ms but lower counter — still no change
        vc.update(a, Hlc::new(300, 2));
        assert_eq!(vc.get(&a), Some(&Hlc::new(300, 5)));

        // Same wall_ms but higher counter — should update
        vc.update(a, Hlc::new(300, 10));
        assert_eq!(vc.get(&a), Some(&Hlc::new(300, 10)));
    }

    #[test]
    fn merge_takes_max() {
        let a = actor(1);
        let b = actor(2);
        let c = actor(3);

        let mut clock1 = VectorClock::new();
        clock1.update(a, Hlc::new(100, 0));
        clock1.update(b, Hlc::new(200, 0));

        let mut clock2 = VectorClock::new();
        clock2.update(a, Hlc::new(50, 0)); // lower than clock1
        clock2.update(b, Hlc::new(300, 0)); // higher than clock1
        clock2.update(c, Hlc::new(400, 0)); // not in clock1

        clock1.merge(&clock2);

        // Actor a: clock1 had 100, clock2 had 50 -> keep 100
        assert_eq!(clock1.get(&a), Some(&Hlc::new(100, 0)));
        // Actor b: clock1 had 200, clock2 had 300 -> take 300
        assert_eq!(clock1.get(&b), Some(&Hlc::new(300, 0)));
        // Actor c: only in clock2 -> take 400
        assert_eq!(clock1.get(&c), Some(&Hlc::new(400, 0)));
    }

    #[test]
    fn diff_finds_missing() {
        let a = actor(1);
        let b = actor(2);
        let c = actor(3);

        let mut clock_a = VectorClock::new();
        clock_a.update(a, Hlc::new(100, 0));
        clock_a.update(b, Hlc::new(200, 0));
        // clock_a has no entry for actor c

        let mut clock_b = VectorClock::new();
        clock_b.update(a, Hlc::new(100, 0)); // same as clock_a
        clock_b.update(b, Hlc::new(300, 0)); // ahead of clock_a
        clock_b.update(c, Hlc::new(400, 0)); // clock_a doesn't have this

        let diff = clock_a.diff(&clock_b);

        // Should contain actor b (we're behind) and actor c (we don't have it)
        assert_eq!(diff.len(), 2);

        let b_entry = diff.iter().find(|(id, _)| *id == b);
        assert_eq!(b_entry, Some(&(b, Some(Hlc::new(200, 0)))));

        let c_entry = diff.iter().find(|(id, _)| *id == c);
        assert_eq!(c_entry, Some(&(c, None)));

        // Actor a should NOT be in the diff (we're caught up)
        assert!(diff.iter().all(|(id, _)| *id != a));
    }

    #[test]
    fn covers_detects_completeness() {
        let a = actor(1);
        let b = actor(2);
        let c = actor(3);

        let mut full = VectorClock::new();
        full.update(a, Hlc::new(100, 0));
        full.update(b, Hlc::new(200, 0));
        full.update(c, Hlc::new(300, 0));

        let mut partial = VectorClock::new();
        partial.update(a, Hlc::new(100, 0));
        partial.update(b, Hlc::new(200, 0));

        // full covers partial (has everything partial has, and more)
        assert!(full.covers(&partial));

        // partial does NOT cover full (missing actor c)
        assert!(!partial.covers(&full));

        // A clock covers itself
        assert!(full.covers(&full));

        // Empty clock is covered by everything
        let empty = VectorClock::new();
        assert!(full.covers(&empty));
        assert!(partial.covers(&empty));
        assert!(empty.covers(&empty));

        // Empty does not cover a non-empty clock
        assert!(!empty.covers(&full));
    }
}
