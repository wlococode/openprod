# Hybrid Logical Clock Specification

This document defines the Hybrid Logical Clock (HLC) format, algorithm, and handling of clock drift.

---

## Overview

HLCs provide deterministic ordering of operations across distributed peers without requiring synchronized clocks. They combine physical wall-clock time with a logical counter to preserve causality while remaining close to real time.

**Anchor invariant:** Given the same set of operations, all peers derive identical canonical ordering using HLC comparison. Clock skew between peers does not affect convergence correctness.

---

## HLC Format

An HLC is a 12-byte value with two components:

```
┌─────────────────────────────────────────────────────────┐
│  Wall Time (8 bytes)       │  Counter (4 bytes)         │
│  big-endian uint64         │  big-endian uint32         │
│  milliseconds since epoch  │  logical counter           │
└─────────────────────────────────────────────────────────┘
```

| Component | Size | Range | Description |
|-----------|------|-------|-------------|
| Wall Time | 8 bytes | 0 to 2^64-1 ms | Milliseconds since Unix epoch (1970-01-01 00:00:00 UTC) |
| Counter | 4 bytes | 0 to 4,294,967,295 | Logical counter for ordering within same millisecond |

### Why This Format

| Decision | Rationale |
|----------|-----------|
| **12 bytes total** | Compact yet sufficient; provides headroom for high-throughput scenarios |
| **No node ID component** | Operations have unique `op_id` (UUIDv7) for tiebreaking; node ID would be redundant |
| **8-byte wall time** | 64-bit milliseconds supports dates until year 292,278,994 |
| **4-byte counter** | 4 billion ops/ms provides ample headroom; practically impossible to overflow |
| **Big-endian encoding** | Enables lexicographic byte comparison for ordering |

### Canonical Representation

- **Wire format:** 12-byte binary, big-endian (MessagePack ext type `0x01`)
- **Storage:** 12-byte BLOB in SQLite
- **Debug/display:** ISO 8601 timestamp + counter suffix: `2024-01-15T10:30:00.123Z/42`

---

## HLC Algorithm

Each actor maintains a local HLC state:

```
State:
  wall: uint64    # last known wall time (ms)
  counter: uint32 # logical counter
```

### Local Event (Send)

When creating a new operation:

```
function tick(state, physical_now):
    if physical_now > state.wall:
        state.wall = physical_now
        state.counter = 0
    else:
        state.counter += 1
        # Counter overflow practically impossible with 4-byte counter
        # (would require 4 billion ops in same millisecond)

    return HLC(state.wall, state.counter)
```

### Receive Event

When receiving an operation from a peer:

```
function receive(state, physical_now, remote_hlc, max_drift):
    remote_wall = remote_hlc.wall
    remote_counter = remote_hlc.counter

    # Reject if remote is too far in the future
    if remote_wall > physical_now + max_drift:
        return Error::FutureHLC(remote_wall - physical_now)

    # Update local state
    if remote_wall > state.wall and remote_wall > physical_now:
        # Remote is ahead but within acceptable drift
        state.wall = remote_wall
        state.counter = remote_counter + 1
    else if remote_wall > state.wall:
        # Remote is ahead, but physical time is even further ahead
        state.wall = physical_now
        state.counter = 0
    else if remote_wall == state.wall:
        # Same wall time: increment counter
        state.counter = max(state.counter, remote_counter) + 1
    else:
        # Remote is behind: use our time
        if physical_now > state.wall:
            state.wall = physical_now
            state.counter = 0
        else:
            state.counter += 1

    # Counter overflow practically impossible with 4-byte counter

    return Ok(())
```

**Key property:** The local HLC is never set to a value beyond `physical_now + max_drift`. This prevents clock poisoning.

---

## HLC Comparison

HLCs are compared lexicographically as byte strings:

```
function compare(a, b):
    # Compare wall time first (most significant)
    if a.wall != b.wall:
        return a.wall <=> b.wall

    # Then compare counter
    return a.counter <=> b.counter
```

### Canonical Operation Ordering

Operations are ordered by `(HLC, op_id)`:

1. Compare HLCs lexicographically
2. If HLCs are equal, compare `op_id` bytes lexicographically

This produces a deterministic total order. Two operations with identical HLCs are distinct events that happened at the same logical moment; the `op_id` tiebreaker ensures consistent ordering across all peers.

**Anchor invariant:** The comparison `(HLC, op_id)` is deterministic across all peers. Given the same operations, all peers derive identical canonical order.

---

## Clock Drift Handling

### Configuration

| Parameter | Default | Description |
|-----------|---------|-------------|
| `max_future_drift` | 5 minutes | Maximum acceptable HLC ahead of local physical time |
| `stale_threshold` | 7 days | Operations older than this are flagged for review |

Both parameters are configurable per-workspace.

### Future HLC (Clock Ahead)

When receiving an operation with HLC more than `max_future_drift` ahead of local physical time:

**Behavior:**
1. Reject the operation (do not apply)
2. Do not update local HLC state (prevents poisoning)
3. Send `NACK` with reason `future_hlc` and drift amount
4. Peer should check their clock synchronization

```yaml
BundleNack:
  bundle_id: <UUID>
  reason: future_hlc
  details: "HLC is 847000ms ahead of local time (max: 300000ms)"
  local_time: <current physical time>
```

**Rationale:** A peer with a misconfigured clock (far ahead) should not poison other peers' HLCs. Rejection forces the issue to surface without corrupting the cluster's logical time.

**Recovery:** The sending peer can:
- Fix their clock and resend
- Wait until their HLC naturally falls within acceptable range
- The operation is not lost (still in sender's oplog), just not yet accepted

### Past HLC (Stale Operations)

When receiving an operation with HLC more than `stale_threshold` behind current time:

**Behavior:**
1. Accept the operation into the oplog (valid operation)
2. Apply to canonical state immediately (stale ops are legitimate work)
3. Flag for UI review (informational, not blocking)
4. Surface in "recently arrived" dashboard with staleness indicator

```yaml
StaleOperationFlag:
  op_id: <UUID>
  bundle_id: <UUID>
  hlc: <stale HLC>
  age: 864000000  # milliseconds (10 days)
  threshold: 604800000  # 7 days
  status: flagged_for_review  # informational only
```

**Rationale:** Stale operations are legitimate committed work, not suggestions. The author intended them to apply. Treating them as proposals would misrepresent intent. The flag is a courtesy warning that helps users notice "old changes just arrived" rather than being surprised.

**Why this differs from proposals:**
- Proposals are intentionally non-canonical ("I'm suggesting this")
- Stale ops are unintentionally late ("I was offline, this is my real work")
- Stale ops that actually conflict with newer edits are caught by conflict detection
- The staleness flag addresses semantic staleness (outdated context), not technical validity

**UI behavior:**
- Stale operations appear in a "recently synced" or "arrived late" section
- Badge indicates age and origin actor
- User can review but cannot "reject" (would discard legitimate work)
- Conflicts with newer edits are handled by normal conflict resolution

**Anchor invariant:** Stale operations are never silently discarded. They apply to canonical state and are flagged for human awareness.

### Backward Clock Jump

If local physical time jumps backward (NTP correction, daylight saving, etc.):

**Behavior:**
1. HLC continues using the higher `wall` value from state
2. Counter increments until physical time catches up
3. No special handling required; algorithm naturally handles this

This may cause HLC to drift ahead of physical time temporarily, but it will reconverge as physical time advances.

---

## Persistence

### SQLite Storage

HLCs are stored as 12-byte BLOBs:

```sql
-- Example: oplog table
hlc BLOB NOT NULL CHECK (length(hlc) = 12)
```

The CHECK constraint ensures format consistency.

### Wire Format

HLCs use MessagePack extension type `0x01`:

```yaml
MessagePack Extension:
  type: 0x01
  data: <12 bytes, big-endian>
```

---

## Implementation Notes

### Initialization

On first startup (no prior state):
- `wall` = current physical time
- `counter` = 0

### Clock Sources

- Use monotonic clock for HLC advancement where available
- Fall back to system time with backward-jump protection
- Never trust remote clocks more than local clock + max_drift

### Testing

Implementations must pass:
- **Monotonicity:** `tick()` always returns HLC > previous HLC
- **Causality:** If A happens-before B, then HLC(A) < HLC(B)
- **Convergence:** All peers with same operations derive same order
- **Drift rejection:** Future HLCs beyond threshold are rejected
- **Poison prevention:** Rejected future HLCs don't advance local state
- **Stale flagging:** Old operations are applied and flagged for awareness

### Recommended Libraries

| Language | Library | Notes |
|----------|---------|-------|
| Rust | Custom or adapted from `uhlc` | Modify for 12-byte format |
| Swift | Custom | Straightforward implementation |
| TypeScript | Custom | Use BigInt for wall time |

---

## Interaction with Other Systems

### Vector Clocks

Vector clocks track `{actor_id: HLC}` for each known actor. The HLC is the last-seen HLC from that actor.

### Bundles

A bundle's HLC is the maximum HLC of its contained operations. This ensures the bundle timestamp reflects "when this bundle completed."

### Sync Protocol

- `VectorClockResponse` contains HLCs per actor
- `OpsRequest.since` specifies HLC threshold per actor
- Stale operations (beyond threshold) are integrated and flagged for user awareness during sync

---

## Security Considerations

### HLC Manipulation

A malicious peer could attempt to:
1. **Send future HLCs:** Mitigated by `max_future_drift` rejection
2. **Replay old operations:** Mitigated by operation deduplication (unique op_id)
3. **Claim false timestamps:** HLCs are informational for ordering; actor signatures prevent forgery of authorship

### Clock Attacks

An attacker controlling NTP could manipulate clocks, but:
- Each peer validates independently
- Future drift rejection limits damage
- Worst case: legitimate operations rejected until clocks resync

---

## Open Questions

- Should `stale_threshold` be per-actor (trust some peers more)?
- Should rejected future-HLC operations be queued for automatic retry?
- Dashboard UX for stale operation review (bundle with conflicts/proposals?)

---

## Related Documents

- [wire-format.md](wire-format.md) — HLC encoding in MessagePack *(archived / deferred to post-v1)*
- [sync.md](sync.md) — Vector clocks and catch-up protocol
- [operations.md](operations.md) — Operation structure including HLC field
- [sqlite-schema.md](sqlite-schema.md) — HLC storage in oplog table
