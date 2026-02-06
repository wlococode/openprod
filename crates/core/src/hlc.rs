use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::CoreError;

pub const MAX_DRIFT_MS: u64 = 300_000; // 5 minutes

/// Returns the current wall-clock time as milliseconds since Unix epoch.
pub fn physical_now() -> Result<u64, CoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .map_err(|_| CoreError::InvalidData("system clock before epoch".into()))
}

/// A 12-byte Hybrid Logical Clock timestamp: 8 bytes wall_ms (big-endian u64)
/// followed by 4 bytes counter (big-endian u32).
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct Hlc {
    wall_ms: u64,
    counter: u32,
}

impl Hlc {
    pub fn new(wall_ms: u64, counter: u32) -> Self {
        Self { wall_ms, counter }
    }

    pub fn wall_ms(&self) -> u64 {
        self.wall_ms
    }

    pub fn counter(&self) -> u32 {
        self.counter
    }

    pub fn to_bytes(&self) -> [u8; 12] {
        let mut buf = [0u8; 12];
        buf[..8].copy_from_slice(&self.wall_ms.to_be_bytes());
        buf[8..].copy_from_slice(&self.counter.to_be_bytes());
        buf
    }

    pub fn from_bytes(bytes: &[u8; 12]) -> Result<Self, CoreError> {
        let wall_ms = u64::from_be_bytes(bytes[..8].try_into().unwrap());
        let counter = u32::from_be_bytes(bytes[8..].try_into().unwrap());
        Ok(Self { wall_ms, counter })
    }
}

impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> Ordering {
        self.to_bytes().cmp(&other.to_bytes())
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Serialize for Hlc {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.to_bytes())
    }
}

impl<'de> Deserialize<'de> for Hlc {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
        let arr: [u8; 12] = bytes
            .try_into()
            .map_err(|v: Vec<u8>| serde::de::Error::invalid_length(v.len(), &"12 bytes"))?;
        Hlc::from_bytes(&arr).map_err(serde::de::Error::custom)
    }
}

/// A clock that generates monotonically increasing HLC timestamps.
pub struct HlcClock {
    wall_ms: u64,
    counter: u32,
}

impl HlcClock {
    pub fn new() -> Self {
        Self {
            wall_ms: 0,
            counter: 0,
        }
    }

    /// Generate the next monotonically increasing timestamp.
    pub fn tick(&mut self) -> Result<Hlc, CoreError> {
        let now = physical_now()?;

        let hlc = if now > self.wall_ms {
            Hlc::new(now, 0)
        } else {
            Hlc::new(self.wall_ms, self.counter + 1)
        };

        self.wall_ms = hlc.wall_ms;
        self.counter = hlc.counter;
        Ok(hlc)
    }

    /// Merge with a remote timestamp, producing a timestamp greater than both.
    pub fn receive(&mut self, remote: &Hlc) -> Result<Hlc, CoreError> {
        let now = physical_now()?;

        // Reject remote timestamps too far in the future
        if remote.wall_ms > now + MAX_DRIFT_MS {
            return Err(CoreError::HlcDriftTooLarge {
                delta_ms: remote.wall_ms - now,
                max_ms: MAX_DRIFT_MS,
            });
        }

        let hlc = if now > self.wall_ms && now > remote.wall_ms {
            // Physical time is greatest
            Hlc::new(now, 0)
        } else if self.wall_ms == remote.wall_ms && self.wall_ms == now {
            // All three equal
            Hlc::new(self.wall_ms, self.counter.max(remote.counter) + 1)
        } else if self.wall_ms == remote.wall_ms {
            // Local and remote tied, both ahead of physical
            Hlc::new(self.wall_ms, self.counter.max(remote.counter) + 1)
        } else if self.wall_ms > remote.wall_ms {
            // Local is greatest
            if self.wall_ms == now {
                Hlc::new(now, self.counter + 1)
            } else {
                Hlc::new(self.wall_ms, self.counter + 1)
            }
        } else {
            // Remote is greatest
            if remote.wall_ms == now {
                Hlc::new(now, remote.counter + 1)
            } else {
                Hlc::new(remote.wall_ms, remote.counter + 1)
            }
        };

        self.wall_ms = hlc.wall_ms;
        self.counter = hlc.counter;
        Ok(hlc)
    }
}

impl Default for HlcClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_monotonicity() {
        let mut clock = HlcClock::new();
        let mut prev = clock.tick().unwrap();
        for _ in 0..100 {
            let next = clock.tick().unwrap();
            assert!(next > prev, "expected {next:?} > {prev:?}");
            prev = next;
        }
    }

    #[test]
    fn same_wall_time_increments_counter() {
        let mut clock = HlcClock::new();
        // Set the clock's wall_ms far into the future so physical_now() < wall_ms
        let future_ms = physical_now().unwrap() + 100_000;
        clock.wall_ms = future_ms;
        clock.counter = 0;

        let t1 = clock.tick().unwrap();
        assert_eq!(t1.wall_ms(), future_ms);
        assert_eq!(t1.counter(), 1);

        let t2 = clock.tick().unwrap();
        assert_eq!(t2.wall_ms(), future_ms);
        assert_eq!(t2.counter(), 2);

        let t3 = clock.tick().unwrap();
        assert_eq!(t3.wall_ms(), future_ms);
        assert_eq!(t3.counter(), 3);
    }

    #[test]
    fn byte_roundtrip() {
        let hlc = Hlc::new(1_700_000_000_000, 42);
        let bytes = hlc.to_bytes();
        let recovered = Hlc::from_bytes(&bytes).unwrap();
        assert_eq!(hlc, recovered);
    }

    #[test]
    fn ordering_matches_bytes() {
        let pairs = vec![
            (Hlc::new(100, 0), Hlc::new(200, 0)),
            (Hlc::new(100, 0), Hlc::new(100, 1)),
            (Hlc::new(100, 999), Hlc::new(101, 0)),
            (Hlc::new(0, 0), Hlc::new(0, 1)),
        ];

        for (a, b) in &pairs {
            let bytes_a = a.to_bytes();
            let bytes_b = b.to_bytes();
            assert_eq!(
                a.cmp(b),
                bytes_a.cmp(&bytes_b),
                "Hlc ordering doesn't match byte ordering for {a:?} vs {b:?}"
            );
            // Also confirm a < b for all our test pairs
            assert!(a < b, "expected {a:?} < {b:?}");
        }
    }

    #[test]
    fn drift_rejection() {
        let mut clock = HlcClock::new();
        let now = physical_now().unwrap();
        let remote = Hlc::new(now + MAX_DRIFT_MS + 1, 0);
        let result = clock.receive(&remote);
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::HlcDriftTooLarge { delta_ms, max_ms } => {
                assert!(delta_ms > MAX_DRIFT_MS);
                assert_eq!(max_ms, MAX_DRIFT_MS);
            }
            other => panic!("expected HlcDriftTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn within_drift_accepted() {
        let mut clock = HlcClock::new();
        let now = physical_now().unwrap();
        // Exactly at the boundary should be accepted
        let remote = Hlc::new(now + MAX_DRIFT_MS, 5);
        let result = clock.receive(&remote);
        assert!(result.is_ok());
        let hlc = result.unwrap();
        // Result should be greater than the remote
        assert!(hlc > remote);
    }

    #[test]
    fn concurrent_timestamp_merging() {
        let mut clock = HlcClock::new();
        // Advance local clock
        let local = clock.tick().unwrap();

        // Create a remote timestamp slightly ahead
        let remote = Hlc::new(local.wall_ms() + 1, 10);

        let merged = clock.receive(&remote).unwrap();
        assert!(merged > local, "merged {merged:?} should be > local {local:?}");
        assert!(merged > remote, "merged {merged:?} should be > remote {remote:?}");
    }
}
