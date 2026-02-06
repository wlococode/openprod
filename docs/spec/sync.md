# Sync Specification

This document defines replication, peer discovery, leader election, partition handling, and catch-up protocols across three sync modes: cloud server, LAN session, and offline.

---

## Sync Modes

All three modes use the same underlying oplog-based replication protocol. The difference is transport and peer discovery.

| Mode | Transport | Discovery | Sequencer | Internet Required |
|------|-----------|-----------|-----------|-------------------|
| Cloud server | WebSocket (TLS) | Server registration | Cloud server | Yes |
| LAN session | WebSocket (local) | mDNS | Elected leader | No |
| Offline | None | None | None | No |

A client may operate in multiple modes simultaneously. A device on both WAN and a local subnet syncs to the cloud server over WAN and to LAN peers over the local subnet. This is not a conflict; the oplog deduplicates by operation ID.

**Mode selection is per-workspace.** A workspace is configured with:

```yaml
SyncConfig:
  cloud_server: "wss://sync.example.com/ws"   # optional
  cloud_token: "<auth_token>"                  # optional
  lan_enabled: true                            # default true
```

If `cloud_server` is set and reachable, the client syncs to it. If `lan_enabled` is true, the client also discovers and syncs with LAN peers via mDNS. If neither is available, the client operates offline.

---

## Design Goals

| Goal | Target |
|------|--------|
| Deterministic ordering | All peers derive identical order from same ops |
| Partition tolerance | Any number of partitions reconverge automatically |
| Live propagation (cloud) | <200ms end-to-end |
| Live propagation (LAN) | <500ms end-to-end |
| Catch-up sync | <5 seconds for typical reconnection |
| Consistency | Strong eventual consistency (SEC) |

---

## Consistency Model

The system provides **strong eventual consistency**:

- All peers with the same operations derive identical state
- Any peer can read and write while disconnected from all other peers
- Peers continue operating independently, merge on reconnect
- No leader or server required for correctness

---

## Cloud Server Mode (Post-v1 / Deferred)

> **Note:** Cloud server mode is deferred to post-v1. V1 supports LAN session mode and offline mode only.

The cloud server is a Rust process that acts as a well-known, always-on peer. It participates in the same oplog-based sync protocol as every other peer, but with three additional responsibilities: relay, persistence, and sequencing.

### Server Responsibilities

1. **Relay.** Accept operations from any connected client and broadcast them to all other connected clients.
2. **Persistence.** Maintain a complete copy of the oplog in durable storage (SQLite). Clients reconnecting after extended absence catch up from the server rather than requiring another online peer.
3. **Sequencing.** Assign monotonic sequence numbers to operations as they arrive. This eliminates ordering churn during real-time editing.

The server does NOT have elevated trust. Every client validates operations independently. The server cannot forge operations or override conflict resolution.

### Cloud Server Protocol

Transport: WebSocket over TLS (`wss://`).

**Connection handshake:**

```
Client                              Server
  |                                    |
  |--- WS connect (/ws) ------------>|
  |--- AuthHello ---------------------->|
  |    { workspace_id, actor_id,       |
  |      auth_token, vector_clock }    |
  |                                    |
  |<--- AuthAck ----------------------|
  |     { server_vector_clock,         |
  |       current_sequence }           |
  |                                    |
  |<--- CatchUp (missing ops) --------|
  |--- CatchUp (server-missing ops) ->|
  |                                    |
  |     (bidirectional streaming)      |
```

**Message types (client to server):**

| Message | Fields | Description |
|---------|--------|-------------|
| `AuthHello` | `workspace_id`, `actor_id`, `auth_token`, `vector_clock` | Authenticate and declare sync state |
| `OpSubmit` | `operations[]` | Submit new operations for sequencing |
| `VectorClockSync` | `vector_clock` | Request missing ops |
| `StateHashCheck` | `oplog_head`, `state_hash` | Verify convergence |
| `Ping` | `timestamp` | Keepalive (sent every 15s) |

**Message types (server to client):**

| Message | Fields | Description |
|---------|--------|-------------|
| `AuthAck` | `server_vector_clock`, `current_sequence` | Confirm connection |
| `AuthReject` | `reason` | Reject connection |
| `OpBroadcast` | `operations[]`, `sequence_start` | Sequenced operations from any client |
| `OpAck` | `op_ids[]`, `sequence_numbers[]` | Confirm receipt and sequencing |
| `CatchUpBatch` | `operations[]`, `remaining` | Batch of missing operations during catch-up |
| `Pong` | `timestamp` | Keepalive response |

**Sequence numbers:**

The server assigns a monotonic sequence number to each operation it receives. This sequence is server-authoritative and used only for ordering during live editing. It does not replace HLC ordering for canonical state derivation.

```yaml
ServerSequence:
  next_sequence: 4823
  # Assigned to ops in arrival order
  # Ties (simultaneous arrival) broken by HLC then op_id hash
```

When clients receive `OpBroadcast`, they apply operations in sequence order for display purposes. The canonical oplog order remains `(HLC, op_id)` for deterministic convergence.

**Delivery tracking:**

The server tracks which operations each connected client has acknowledged:

```yaml
ServerDeliveryState:
  sequence: 4823
  clients:
    alice_laptop: { acked: 4823, connected: true }
    bob_tablet: { acked: 4820, connected: true }
    carol_phone: { acked: 4790, connected: false, last_seen: <HLC> }
```

Unacknowledged operations are retransmitted. When a client reconnects, catch-up begins from its last acknowledged sequence number (or falls back to vector clock exchange if sequence state is unavailable).

**Connection lifecycle:**

1. Client opens WebSocket to `cloud_server` URL.
2. Client sends `AuthHello` with workspace ID, actor ID, auth token, and current vector clock.
3. Server validates auth token, replies `AuthAck` with its own vector clock and current sequence number.
4. Server computes delta from client vector clock, streams `CatchUpBatch` messages.
5. Client computes delta from server vector clock, sends missing ops via `OpSubmit`.
6. Once caught up, the connection enters steady-state: client sends `OpSubmit` for local edits, server broadcasts `OpBroadcast` for all edits.
7. Client sends `Ping` every 15 seconds. Server replies `Pong`. If no `Pong` received within 10 seconds, client considers the connection lost and falls back to LAN or offline mode.

### Self-Hosted vs. Provider-Hosted

The cloud server is the same Rust binary in both cases. The only difference is who runs it.

- **Self-hosted:** User runs the server on their own hardware (backstage Mac Mini, a VM, a NAS). The server URL is a LAN IP or a hostname the user controls.
- **Provider-hosted:** A service provider runs the server. The server URL is a public hostname. Auth tokens are issued by the provider.

The client does not distinguish between these. It connects to whatever URL is in `SyncConfig.cloud_server`.

---

## LAN Session Mode

LAN mode enables sync between peers on the same local network without internet access. This is the primary mode for isolated production networks (lighting ETCNet, sound Dante networks, backstage WiFi) where WAN is unavailable.

### Peer Discovery via mDNS

Peers advertise and discover each other using mDNS (Multicast DNS / DNS-SD).

**Service advertisement:**

```
Service type: _openprod._tcp.local.
Port: <sync_port>
TXT records:
  workspace=<workspace_id>
  actor=<actor_id>
  version=<protocol_version>
```

**Discovery behavior:**

1. On workspace open (with `lan_enabled: true`), the client begins advertising its mDNS service and browsing for peers.
2. Discovered peers with the same `workspace` value are candidates for sync.
3. Peers with mismatched `version` are ignored (logged as warning).
4. Discovery is continuous. New peers appearing on the network are detected automatically.

**Network constraint:** mDNS operates on the local subnet only (multicast to `224.0.0.251:5353`). Peers on different subnets without multicast routing will not discover each other. This is acceptable and expected for isolated production networks. Users on isolated subnets sync with each other; cross-subnet sync requires a cloud server or manual IP entry.

### LAN Sync Protocol

Transport: WebSocket (plaintext `ws://` on local network, or `wss://` if TLS is configured).

Once two LAN peers discover each other via mDNS, they establish a WebSocket connection and perform the same vector clock-based catch-up protocol used in cloud mode, minus authentication.

**LAN handshake:**

```
Peer A                              Peer B
  |                                    |
  |--- WS connect ------------------->|
  |--- PeerHello -------------------->|
  |    { workspace_id, actor_id,      |
  |      vector_clock, protocol_ver } |
  |                                    |
  |<--- PeerHello --------------------|
  |     { workspace_id, actor_id,     |
  |       vector_clock, protocol_ver }|
  |                                    |
  |    (bidirectional delta exchange)  |
```

**LAN message types:**

| Message | Fields | Description |
|---------|--------|-------------|
| `PeerHello` | `workspace_id`, `actor_id`, `vector_clock`, `protocol_version` | Identify and declare sync state |
| `OpExchange` | `operations[]` | Send missing operations |
| `StateHashCheck` | `oplog_head`, `state_hash` | Verify convergence |
| `Ping` / `Pong` | `timestamp` | Keepalive (every 15s) |

### Gossip Protocol

LAN mode uses a gossip protocol for operation propagation. This is the core replication mechanism when no cloud server is available.

**Each peer maintains:**

- `oplog` -- all operations it has seen
- `vector_clock` -- `{actor_id -> max_HLC_seen}` for each known actor
- `peer_list` -- known LAN peers and their last-known vector clocks

**Propagation strategies:**

| Strategy | When Used | Behavior |
|----------|-----------|----------|
| Push on write | Real-time editing | Immediately push new ops to all connected LAN peers |
| Pull on interval | Background sync | Exchange vector clocks with a random peer every 30s |
| Pull on reconnect | Partition healing | Full vector clock reconciliation |

**Convergence guarantee:**
- All operations have unique `(HLC, op_id)`
- Canonical order is deterministic: `sort by (HLC, op_id)`
- Operations only flow forward (never deleted during sync)
- Any connected peer eventually receives all ops from any other connected peer
- Therefore: all peers converge to identical oplog and identical derived state

### Leader Election (LAN Only) (Post-v1 / Deferred)

> **Note:** Leader election is deferred to post-v1. V1 LAN mode operates in leaderless mode only (gossip-based sync with HLC ordering).

Leader election is used only in LAN mode when no cloud server is reachable. The leader provides sequencing for lower-latency ordering during real-time collaboration.

**Leader election is NOT required for correctness.** LAN mode works without a leader. Leader election is an optimization that reduces ordering churn.

**When leader election triggers:**
- LAN session starts with 2+ peers and no cloud server available
- Current leader's heartbeat times out (5 second timeout)
- Manual trigger by user (rare)

**Protocol (Bully algorithm variant):**

1. Peer detects no leader (no heartbeat received within 5s).
2. Peer broadcasts: `ElectionStart { candidate: <actor_id>, priority: <priority> }`
3. Priority is a deterministic function: lowest `hash(actor_id)` wins.
4. Peers that receive an `ElectionStart` from a higher-priority candidate respond `ElectionAck` and stand down.
5. If a peer receives no `ElectionAck` from a higher-priority candidate within 2 seconds, it declares itself leader.
6. New leader broadcasts: `LeaderAnnounce { leader: <actor_id>, epoch: <N> }`
7. All peers acknowledge and begin routing operations through the leader.

**Epoch prevents stale leaders:**
- Each election increments the epoch number.
- Peers reject sequencing from leaders with old epochs.
- Prevents split-brain when an old leader reconnects after partition.

**Leader responsibilities (LAN):**
- Assign monotonic sequence numbers to operations
- Track delivery state (who has acknowledged which ops)
- Manage presence (who is connected on the LAN)
- Broadcast operations in assigned order

**Leader is NOT:**
- Authoritative for truth (all peers validate independently)
- Required for correctness (gossip mode works without a leader)
- A permission escalation (same access as other peers)

### LAN Leader Protocol

```
Alice                    Leader (Bob)              Carol
  |                        |                        |
  |---- OpSubmit (A) ----->|                        |
  |<--- OpAck seq=42 ------|                        |
  |                        |--- OpBroadcast 42: A ->|
  |                        |<-- OpAck 42 -----------|
  |                        |                        |
  |   (Bob tracks: alice=42, carol=42)              |
```

**Redirect protocol:**

When a peer sends an operation directly to a non-leader peer:

```
Dave                     Alice                   Bob (Leader)
  |                        |                        |
  |--- OpSubmit (direct) ->|                        |
  |                        | (I know Bob is leader) |
  |<-- Redirect            |                        |
  |    { leader: bob,      |                        |
  |      epoch: 5 }        |                        |
  |                        |                        |
  |--- OpSubmit (D) ------------------------------>|
  |<-- OpAck seq=43 -------------------------------|
```

### Leaderless LAN Mode

When no leader is available (election failed, single peer, or intentional), LAN peers operate in leaderless mode.

**Leaderless behavior:**
- Operations created locally, applied optimistically
- Ordering via HLC + tiebreaker (not sequence numbers)
- Sync via vector clock exchange when peers connect
- State re-derived on receiving new operations

| Aspect | Leader Mode | Leaderless Mode |
|--------|-------------|-----------------|
| Ordering | Sequence numbers, no churn | HLC sort, may reorder on sync |
| Latency | <500ms (one hop to leader) | <2s (ordering stabilization) |
| Delivery | Leader tracks ACKs | Vector clock reconciliation |
| Presence | Authoritative member list | Heartbeat-based, peers may disagree |

Leaderless mode is correct but slightly rougher UX. Acceptable for partitioned or single-peer scenarios.

---

## Offline Mode

When a peer has no network connectivity (or sync is disabled), it operates offline.

**Behavior:**
- All data is local. Reads and writes proceed with zero latency.
- Operations accumulate in the local oplog.
- On reconnect (to cloud server or LAN peer), the standard catch-up protocol runs.
- Conflicts are detected and surfaced after merge.

There is no ceremony to enter or exit offline mode. The system does not declare "you are offline." It simply cannot reach any peers and continues working.

---

## Vector Clocks

Each peer maintains a vector clock tracking what operations it has seen from each actor:

```yaml
VectorClock:
  alice_device1: <HLC_1234567890>
  bob_laptop: <HLC_1234567800>
  carol_tablet: <HLC_1234567750>
```

**When vector clocks are used:**
- Every catch-up sync (cloud reconnect, LAN reconnect, offline reconnect)
- Every periodic background sync in LAN mode
- Partition merge after any split

**Catch-up protocol:**

1. Reconnecting peer sends its vector clock to the remote peer (or cloud server).
2. Remote computes delta: all operations not covered by the reconnecting peer's vector clock.
3. Remote sends missing operations.
4. Reconnecting peer integrates operations and re-derives state.
5. Both exchange state hashes to verify convergence.

**Efficiency:** Vector clock size scales with the number of actors, not the number of operations. For a typical production (5-30 active devices), vector clocks are trivially small.

---

## State Hash Verification

State hashes verify convergence after sync. Two-layer verification catches different classes of divergence.

### Two-Layer Hash Verification

```yaml
SyncVerification:
  oplog_head: blake3(last_N_operations)      # Quick sync check
  state_hash: blake3(canonical_derived_state) # Catches derivation bugs
```

**Layer purposes:**

| Layer | Purpose | When Used |
|-------|---------|-----------|
| `oplog_head` | Quick "are we in sync?" check | Every sync exchange |
| `state_hash` | Verify identical derived state | After oplog sync completes |

**Verification flow:**

1. `oplog_head` matches -- likely in sync, done.
2. `oplog_head` differs -- exchange vector clocks, sync missing ops.
3. After sync, `state_hash` matches -- confirmed convergent.
4. After sync, `state_hash` differs -- derivation bug. Alert and trigger diagnostic mode.

### State Hash Composition

**Included in `state_hash`:**
- All entity canonical field values (sorted by `entity_id`, then `field_key`)
- All edge states (sorted by `edge_id`)
- Entity existence/deletion state
- Conflict resolution outcomes

**Excluded from `state_hash`:**
- Overlays (local-only, not synced)
- Detailed conflict history (only resolution outcomes matter)

### Garbage Collection Compatibility

GC can remove detailed conflict history but must retain resolution outcomes:

| Retained (affects hash) | GC-eligible (does not affect hash) |
|-------------------------|------------------------------------|
| Conflict ID | Full list of competing values |
| Resolution outcome (chosen value) | Original conflict operation payloads |
| Resolver identity | Actor timestamps for non-chosen values |
| Resolution HLC | |

**On mismatch:**
- Indicates a bug, corruption, or missing operations
- Triggers diagnostic mode (compare oplogs, identify divergence point)
- Never auto-resolved; requires investigation

---

## Partition Behavior

**Partitions are implicit, not declared.** No ceremony to enter or exit partition mode. The system does not detect "we are partitioned." Peers simply can or cannot reach each other.

**What happens:**

1. Peer loses connectivity to other peers (cloud server, LAN peers, or both).
2. Peer continues working locally (offline mode).
3. Operations accumulate in local oplog with HLC timestamps.
4. On reconnection, the catch-up protocol exchanges missing ops via vector clocks.
5. State re-derived from merged canonical order.
6. Conflicts detected and surfaced for user resolution.

### Multi-Partition Merge (N-way)

The gossip protocol (LAN) and cloud server relay handle any number of partitions:

```
Partition A (peers 1, 2) -- connected to cloud server:
  - ops 1-50 (shared before split)
  - ops 51-80 (created during split)

Partition B (peers 3, 4) -- isolated LAN (sound network):
  - ops 1-50 (shared)
  - ops 81-120 (created during split)

Partition C (peer 5) -- fully offline:
  - ops 1-50 (shared)
  - ops 121-130 (created offline)

On reconnection:
  - Peer 3 gets WAN access, connects to cloud server
  - Cloud server and Peer 3 exchange deltas (ops 51-120 merge)
  - Cloud server relays to Peers 1, 2 on next sync
  - Peer 5 reconnects to LAN, syncs with Peer 3
  - Eventually all peers converge to identical state
```

**No special N-way coordination needed.** Pairwise sync (whether through the cloud server or direct LAN exchange) automatically propagates all operations to all reachable peers.

### Partition Leader Coexistence (LAN)

During a LAN partition (where the cloud server is unavailable), multiple LAN leaders may exist in separate partitions:

```
Partition A (LAN segment 1):      Partition B (LAN segment 2):
  Leader: Alice (epoch 5)           Leader: Carol (epoch 3)
  Peers: Bob, Dave                  Peers: Eve, Frank
```

**On reconnection:**

1. Leaders discover each other (mDNS or via cloud server).
2. Higher epoch wins (Alice, epoch 5).
3. Carol steps down.
4. All peers exchange vector clocks and sync missing operations.
5. If cloud server becomes available, both partitions sync to it and the LAN leader may step down (cloud server handles sequencing).

---

## Sync Behavior

### Non-Blocking Sync

- Sync is non-blocking; users continue working while sync is in progress.
- A user's local edits remain visible to them until sequenced or explicitly rejected.
- Sync never silently overwrites a user's unsequenced local edits.
- Temporary divergence between local optimistic state and canonical state is acceptable during sync.
- Sync progress is monotonic; peers only advance forward in acknowledged state.
- The system guarantees eventual convergence of all valid operations.

### Provisional State Visibility (Author)

- Local edits are applied optimistically and immediately visible to the author.
- A peer never blocks or delays local edits waiting for sequencing acknowledgment.
- Local optimistic state persists across temporary disconnection.
- Unsequenced local edits remain visible until either sequenced or rejected through conflict resolution.
- The UI may indicate provisional/unacknowledged state but never withholds, reverts, or delays the edited value.

### Provisional State Visibility (Other Peers)

- A peer never observes another peer's unsequenced optimistic edits.
- A peer sees another peer's edit only once the operation has been received through the sync protocol (via cloud server broadcast, LAN leader broadcast, or gossip exchange).

### State Presentation

- Primary views present current semantic state, not operation history.
- Users are not required to reason about individual operations to continue work.
- All operations remain individually inspectable in history/audit views.
- History presentation may group operations meaningfully without violating deterministic ordering.

### Sync Reliability

- Sync progress is checkpointed periodically.
- Interrupted sync resumes from last checkpoint.
- No ops are applied until a checkpoint is complete.
- Partial sync never leaves a peer in an inconsistent state.
- Sync rejects corrupt ops from peers (invalid signature, malformed payload).
- System attempts to recover corrupt ops from other peers before quarantining.

### Consistency States

| State | Consistency Level |
|-------|-------------------|
| Connected to cloud server, synced | Strong (identical state hashes) |
| Connected to LAN peers, synced | Strong (identical state hashes) |
| Connected, syncing | Eventual (converging) |
| Partitioned / offline | Local (divergent until reconnect) |

---

## Peer Discovery

Peers are discovered through three mechanisms, in order of priority:

| Mechanism | Scope | Configuration |
|-----------|-------|---------------|
| Cloud server registration | WAN | `SyncConfig.cloud_server` URL |
| mDNS | Local subnet | Automatic when `lan_enabled: true` |
| Manual IP | Any reachable network | User enters `<ip>:<port>` |

**Cloud server registration:** When a client connects to the cloud server, the server is aware of all connected clients for that workspace. The server handles relay; clients do not need to know each other's addresses.

**mDNS:** Zero-configuration discovery on the local subnet. Peers advertise `_openprod._tcp.local.` and browse for the same service type. Filtering by `workspace` TXT record ensures peers only connect to the same workspace.

**Manual IP:** For peers on reachable networks where mDNS does not work (different subnets without multicast routing, VPNs). The user enters an IP address and port. The client connects via WebSocket and performs the standard LAN handshake.

---

## Security Model

Peers are assumed trusted within a workspace. The protocol prevents injection and corruption using **Ed25519 signatures**:

- Each actor has an Ed25519 keypair. The actor's **public key is the actor_id**.
- All operations are signed by the originating actor's **Ed25519 private key**.
- Peers verify each operation's signature using the actor's **public key** (which is the actor_id).
- Invalid signatures are rejected immediately.
- Operations are validated independently by each peer (schema conformance, field types).
- No peer can inject operations attributed to another actor (they lack the private key).
- Corruption is detected via signature verification and state hash comparison.
- The cloud server cannot forge operations (it only relays and sequences).
- LAN leader cannot forge operations (it only sequences).

**Cloud server auth:** Token-based authentication. The `auth_token` in `AuthHello` is validated by the server. Tokens are opaque to the sync protocol; issuance and revocation are outside this spec's scope.

**LAN auth:** No authentication by default. Any device on the local subnet that advertises the same workspace ID can join. For environments requiring LAN auth, a shared workspace secret can be configured (details in a future security spec).

**Permissions:** Role-based sync permissions (restricting who can edit what) are deferred to post-v1. In V1, all peers within a workspace have full read/write access.

---

## Catch-up Performance

Target: <5 seconds for typical reconnection (hours to days offline).

**Strategies:**
- Vector clock exchange (minimal metadata overhead)
- Delta sync (only send missing operations)
- Streaming application (apply while receiving, do not wait for complete delta)
- `CatchUpBatch` messages with `remaining` count for progress indication
- Cloud server maintains full oplog, enabling catch-up even when no other peer was online during the absence

---

## CRDT Field Sync

CRDT fields use Yjs state vectors for efficient delta synchronization. See [crdt.md](crdt.md) for CRDT field specification.

### Full State Sync

For initial sync or when state vectors are unavailable:

```yaml
CRDTFullSync:
  entity_id: <uuid>
  field: "description"
  state: <yjs_state_bytes>        # Full Yjs document state
  state_vector: <yjs_sv_bytes>    # Current state vector
```

The receiving peer merges the incoming state with local state using `apply_update_v2`.

### Delta Sync Protocol

For incremental sync, peers exchange state vectors to compute minimal deltas:

```
Peer A                              Peer B
  |                                    |
  |--- CRDTSyncRequest -------------->|
  |    { field, state_vector_a }      |
  |                                    |
  |<-- CRDTSyncResponse --------------|
  |    { delta_for_a, state_vector_b }|
  |                                    |
  |--- CRDTSyncResponse ------------>|
  |    { delta_for_b }               |
  |                                    |
```

**Message formats:**

```yaml
CRDTSyncRequest:
  entity_id: <uuid>
  field: "description"
  state_vector: <yjs_sv_bytes>    # Peer's current state vector

CRDTSyncResponse:
  entity_id: <uuid>
  field: "description"
  delta: <yjs_update_bytes>       # Computed via encode_state_as_update(their_sv)
  state_vector: <yjs_sv_bytes>    # Sender's state vector (for reverse sync)
```

**Delta computation:**

```rust
// Peer B computes delta for Peer A
let delta = doc.encode_state_as_update_v2(&peer_a_state_vector);
```

### State Vector Tracking

Each peer maintains a local state vector per CRDT field:

```yaml
CRDTStateVectors:
  "entity_123:description": <yjs_sv_bytes>
  "entity_456:notes": <yjs_sv_bytes>
```

State vectors are updated after each `ApplyCRDT` operation and after receiving sync updates.

### CRDT Sync Triggers

CRDT delta sync occurs:

1. **On reconnection:** After oplog catch-up completes (cloud or LAN).
2. **Periodically:** Background sync every 30 seconds for active CRDT fields.
3. **On demand:** When a user opens an entity with CRDT fields.

CRDT sync is transported over the same WebSocket connection used for oplog sync (cloud or LAN). CRDT messages are interleaved with oplog messages.

### Conflict-Free Merge

Unlike scalar fields, CRDT sync never produces conflicts:

- Incoming deltas are merged via `apply_update_v2`
- Yjs handles concurrent edits automatically
- No conflict detection or resolution required
- State vectors ensure no duplicate operations applied

### Bandwidth Optimization

For large documents with small changes:

- Delta size is proportional to changes, not document size
- State vectors are small (~100 bytes per active editor)
- Full state sync only needed for new peers or corruption recovery

---

## Mode Interaction Summary

This table summarizes behavior for common network topologies in production environments:

| Scenario | Cloud Server | LAN | Behavior |
|----------|-------------|-----|----------|
| Full internet, all devices on WAN | Connected | Optional | All clients sync through cloud server. LAN sync provides redundant path if enabled. |
| Backstage WiFi, no internet | Unavailable | Active | LAN gossip with optional leader election. Fully functional. |
| Lighting ETCNet (isolated) | Unavailable | Active | LAN gossip on subnet. Only lighting devices sync with each other. |
| Sound Dante network (isolated) | Unavailable | Active | LAN gossip on subnet. Only sound devices sync with each other. |
| Mixed: SM laptop on both WiFi and ETCNet | Connected (via WiFi) | Active (both subnets) | SM laptop syncs to cloud over WiFi, syncs to lighting devices over ETCNet, syncs to sound devices over Dante. SM laptop bridges all three partitions. |
| Single user, no network | Unavailable | No peers | Fully offline. Local oplog only. Merge on any future reconnect. |

---

## Related Documents

- [hlc.md](hlc.md) -- Hybrid logical clock specification
- [crdt.md](crdt.md) -- CRDT field specification (collaborative text editing)
- [ordered-edges.md](ordered-edges.md) -- Ordered edge specification
