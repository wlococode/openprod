# V1 Core Design

## Overview

OpenProd is a local-first, offline-capable collaborative workspace for live entertainment production teams. The core insight: production paperwork is logically derived from underlying data with explicit relationships. One system with explicit relationships replaces manual recalculation across disconnected documents.

V1 delivers a **collaborative workspace**: users create tables, add records, edit fields, sync over LAN, resolve conflicts, automate with Lua scripts, and attach files. Modules provide schema, views, and scripting as composable packages.

## V1 Scope

### In scope

- Entity/facet model (internal architecture)
- Tables, records, fields (user-facing data model)
- Field mapping (suggested by modules, confirmed by users)
- Ed25519 keypair identity with operation signing
- Oplog + HLC (append-only log, deterministic ordering)
- Operations + bundles (atomic field-level mutations)
- Edges (directed relationships, including ordered edges with fractional index)
- Conflict detection + resolution (causal concurrency, branch tips, UI)
- Overlays (staging areas with commit/discard, delta-based drift handling)
- CRDT text fields (via Yrs)
- Rules engine (table membership rules, query scope, cycle detection, runtime safety)
- Module system (schema + views + Lua scripts)
- Lua scripting (manual + on-change triggers, sandboxed)
- LAN sync (mDNS discovery, peer-to-peer replication)
- Blobs/assets (content-addressed store, attach to entities, LAN sync, thumbnails as fast-follow)
- Undo/redo (per-user, bundle-level)

### Out of scope (deferred)

- Background scripts (long-running processes like OSC listeners)
- Expression language / Smart Fields (query mode, computed fields)
- Cloud sync server / cloud relay
- Leader election (sync optimization)
- Proposals and approval workflows
- Schema evolution / migrations
- Snapshots and GC
- Permissions / roles (Viewer, Editor, Admin — V1: everyone who joins a workspace can edit)
- AssignRole / RevokeRole operations

### Simplified for V1

- **Overlays**: delta-based drift model. Non-conflicting canonical changes are irrelevant to overlay state. On conflict: warn user with Keep Mine / Use Canonical options.
- **Sync**: LAN only (mDNS discovery, peer-to-peer). Offline is implicit when no peers are found.
- **Permissions**: none — everyone who joins a workspace can edit.

## System Architecture

Three processes communicate over local network protocols:

```
┌─────────────────────┐     HTTP/WS      ┌──────────────────┐
│   Electron App      │◄────────────────►│   Rust Engine     │
│   (UI Client)       │  localhost:PORT   │   (Core Server)   │
└─────────────────────┘                   ├──────────────────┤
                                          │  SQLite (local)   │
┌─────────────────────┐     HTTP/WS      │  Oplog + State    │
│   Bot Harness       │◄────────────────►│                   │
│   (Test Client)     │  localhost:PORT   └────────┬─────────┘
└─────────────────────┘                           │
                                            mDNS + TCP
                                                  │
                                          ┌───────▼─────────┐
                                          │  Other Peers     │
                                          │  (LAN Engines)   │
                                          └─────────────────┘
```

**Rust Engine** — the core. Owns the oplog, derives state, handles sync. Exposes:
- **HTTP API** — commands (create table, add record, set field, commit overlay) and queries (list records, get conflicts, get history). Stateless request/response. RPC-style, not REST.
- **WebSocket** — real-time event stream. Pushes state changes, sync progress, conflict notifications, overlay updates to connected clients.

**Electron App** — pure UI client. Renders tables, fields, conflict resolution, overlay management. All state comes from the engine. No business logic in the frontend.

**Bot Harness** — multi-peer simulator. Spins up N engine instances in-process, each with its own SQLite database. Simulates network topology (connect/disconnect peers). Tests are Rust integration tests that create scenarios and assert convergence. Any operation the UI can perform, the bot harness can perform.

**Database** — one SQLite database file per workspace (`oplog.db`), with logical separation by table naming. The engine talks to storage through a `Storage` trait, not directly to SQLite. V1 implements `SqliteStorage`. This trait boundary is where Postgres or remote storage plugs in later.

## Engine Internal Structure

```
┌──────────────────────────────────────────────────────────────────┐
│                     HTTP / WebSocket API                         │
├──────────────────────────────────────────────────────────────────┤
│                     Command / Query Bus                          │
├──────────┬─────────┬───────────┬────────┬─────────┬─────────────┤
│ Overlays │ Modules │ Conflicts │ Rules  │ Scripts │    Blobs    │
├──────────┴─────────┴───────────┴────────┴─────────┴─────────────┤
│                     State Derivation                             │
├──────────────────────────────────────────────────────────────────┤
│                     Oplog                                        │
├──────────────────────────────────────────────────────────────────┤
│                     Storage Trait                                │
├──────────────────────────────────────────────────────────────────┤
│                     SQLite (oplog.db per workspace)              │
└──────────────────────────────────────────────────────────────────┘
```

**Storage Trait** — defines operations: append ops, query ops by range/actor, read/write state tables, read/write CRDT documents.

**Oplog** — append-only log of operations. Nothing is ever deleted in v1.

**State Derivation** — replays the oplog in canonical order to produce current state. State is cached and incrementally updated as new ops arrive. Full replay only on startup or verification.

**Domain Services:**
- **Overlay Manager** — maintains overlays (one user overlay, any number of script overlays). Overlays store deltas only, not full snapshots. Can be swapped (activating one puts the current in background), stashed/recalled, and committed/canceled independently. On commit, staged ops go to the oplog as a bundle. On conflicting canonical drift, user is warned with Keep Mine / Use Canonical options.
- **Module Manager** — loads module schemas (TOML), registers tables and facet definitions, tracks field mapping suggestions. Serves TypeScript views to the Electron app as static assets.
- **Rules Engine** — declarative table membership rules and query scope. Includes cycle detection and runtime safety. Rules determine which entities appear in which tables based on field values and edge relationships.
- **Conflict Manager** — detects causal concurrency using vector clocks, tracks unresolved conflicts per field, surfaces them to clients.
- **Script Engine** — embeds Lua 5.4 via `mlua`. Runs scripts in isolated overlay sessions. Manages capability grants. On-change triggers watch for field/entity changes.
- **Blob Manager** — content-addressed store (BLAKE3 hash to file), attachment tracking (entity field to blob hash), LAN blob sync (on-demand). Thumbnails as fast-follow.

**Command/Query Bus** — separates writes (commands that produce operations) from reads (queries against derived state). Commands go through an overlay or directly to the oplog. Queries read derived state with the active overlay applied on top.

## Data Model

### User-facing: Tables, Records, Fields

- A **table** is a named collection of records with a defined schema (field names + types).
- A **record** is a row in a table with values for each field.
- A **field** has a type and a mode:
  - **Discrete** — plain value (text, integer, float, boolean, date, datetime, duration, enum, entity-reference, blob-reference)
  - **Reference** — points to another entity's field, stays in sync

### Internal: Entities, Facets, Edges

- An **entity** is a pure identity container — just a UUIDv7. No data of its own.
- A **facet** is a module-owned grouping of fields attached to an entity. Each table maps to exactly one facet. Adding a record to a table attaches the corresponding facet to the entity.
- One entity can have multiple facets — "Jane Doe" can appear in both Contacts and Cast tables. Edits to shared fields propagate because they're the same underlying data.
- **Field mapping**: when two facets have fields representing the same concept, modules suggest the mapping and users confirm it. No auto-binding.
- An **edge** is a directed relationship between two entities with an optional type label and optional properties. Edges can be **ordered** using a fractional index for conflict-free reordering.

### Identity operations

- **Merge**: two entities discovered to be the same thing — one absorbs the other's facets. Reversible, auditable.
- **Split**: one entity discovered to be two things — facets redistributed. Reversible.

## Oplog and Operations

### Operation structure

```
Operation {
    op_id:      UUIDv7          // globally unique, time-sortable
    hlc:        HLC             // 12 bytes (8 wall-clock + 4 counter)
    actor_id:   Ed25519PubKey   // 32-byte public key — who performed this
    signature:  Ed25519Sig      // 64-byte signature over the operation
    bundle_id:  UUIDv7 | null   // groups atomic operations
    payload:    OperationPayload
}
```

Every operation is signed by the actor's Ed25519 private key. Peers verify signatures on receipt. The actor's identity is their public key — no UUIDs, no registration.

### Operation types

- `CreateEntity { entity_id }` / `DeleteEntity { entity_id }`
- `AttachFacet { entity_id, facet_type }` / `DetachFacet { entity_id, facet_type }`
- `SetField { entity_id, facet_type, field_name, value }` — scalar fields only. **SetField on a CRDT field is a type error; use SetCRDTField instead.**
- `SetCRDTField { entity_id, facet_type, field_name, delta }` — text/list CRDT fields (via Yrs)
- `CreateEdge { edge_id, source, target, edge_type, properties }` / `DeleteEdge { edge_id }`
- `CreateOrderedEdge { edge_id, source, target, edge_type, fractional_index, properties }` / `MoveOrderedEdge { edge_id, new_index }` / `RebalanceOrderedEdges { parent_entity, edge_type, new_ordering }`
- `ConfirmFieldMapping { facet_a, field_a, facet_b, field_b }`
- `MergeEntities { survivor, absorbed }` / `SplitEntity { source, new_entity, facets }`

**Deferred operation types (post-v1):** `AssignRole`, `RevokeRole`.

### Bundles

A group of operations that succeed or fail atomically. Example: creating a record = `CreateEntity` + `AttachFacet` + N x `SetField`, all in one bundle.

### Canonical ordering

Operations are sorted by `(hlc, op_id)`. Raw op_id comparison as tiebreaker — no hashing. Every peer with the same set of operations derives the identical order. No coordination needed.

### Incremental state derivation

On startup, full replay from oplog. During runtime, new operations are applied incrementally to cached state. A state hash allows peers to verify convergence.

## LAN Sync Protocol

### Discovery

Each engine advertises via mDNS (`_openprod._tcp.local`) with workspace ID and connection port. Engines only connect to peers advertising the same workspace ID.

### Connection and catch-up

Peers connect via TCP. After handshake (workspace ID verification), they exchange vector clocks (`{ Ed25519PubKey -> max_HLC_seen }`). Each side computes the diff and sends missing ops in canonical order. Recipient verifies Ed25519 signatures, appends to oplog, and incrementally updates state.

### Ongoing sync

After initial catch-up, peers stay connected. New local operations are pushed to all connected peers immediately via direct fan-out. With small team sizes (2-20 people), full mesh is practical. Cloud relay and leader election are deferred to post-v1.

### Disconnection and partition

Implicit. If a peer disappears, the engine stops sending. On reconnect, vector clock catch-up handles the gap. No special partition detection. Works for N-way partitions because every pairwise reconnection syncs transitively.

### Blob sync

Separate from oplog sync. Blobs are identified by their BLAKE3 hash. When a peer encounters a blob hash it doesn't have locally, it requests the blob content from any connected peer that has it. On-demand, not proactive.

## Conflict Detection and Resolution

### Detection

Based on causality, not timestamps. When a new operation arrives, the engine checks whether its vector clock shows the author had seen the previous value. If yes, sequential edit (no conflict). If no, concurrent edit (conflict).

### Granularity

Field-level. Edits to different fields of the same entity do not conflict. Edits to the same scalar field do.

### CRDT fields are exempt

Text and list fields use Yrs for automatic merge. Conflicts only apply to scalar fields.

### Conflict state

Stores competing branch tips: each causal branch's latest value plus the actor/HLC that produced it. Typical case is two values; N-way conflicts show N values.

### Resolution

User picks a winning value or enters a new one. Resolution produces a `SetField` operation in the oplog — auditable and syncable.

### Late-arriving conflicts

If a peer syncs an old edit that conflicts with an already-resolved field, the conflict reopens with the new competing value. No data silently lost.

### Mapped fields

Fields confirmed as the same concept across facets are treated as a single field for conflict purposes.

## Overlays

### Mechanics

When an overlay is active, all writes route to the overlay's local buffer instead of the oplog. The UI shows overlay values on top of canonical state.

### Commit and discard

- **Commit** — staged operations written to oplog as a single bundle. Atomic.
- **Discard** — staged operations thrown away. No trace in oplog.

### Canonical drift (delta model)

Overlays represent deltas (changes only), not full snapshots. Non-conflicting canonical changes are irrelevant to overlay state — they do not block commit. If canonical state changes a field the overlay also touches (a true conflict), the user is warned and presented with two options: **Keep Mine** (overwrite canonical on commit) or **Use Canonical** (drop their edit for that field). Only actual conflicts require resolution before commit.

### Overlay management

Multiple overlays can exist simultaneously — one user overlay and any number of script overlays. The active overlay's changes are visible in the UI. Users can:
- **Swap** — switch which overlay is active
- **Stash/Recall** — put any overlay in background, bring it back
- **Commit/Cancel** — each overlay independently, whether active or in background

### Script overlays

When a Lua script runs, it gets its own overlay automatically. The script's operations accumulate there. On completion, the user previews the result and decides to commit or discard. Script overlays are independent from the user overlay.

## Module System and Scripting

### Module structure

```
my-module/
  module.toml      # schema: tables, fields, types, capabilities, shared key suggestions
  views/           # TypeScript UI components
  scripts/         # Lua automation scripts
```

### Installation vs adoption

- **Install** — per-user. Downloads the module, makes views available locally. No effect on workspace data.
- **Adopt** — workspace-scoped. Registers module's tables and facet definitions. All peers see new tables. Field mapping suggestions presented for user confirmation.

### Lua scripting

- Embedded via `mlua` (Rust Lua 5.4 binding)
- **Manual scripts** — user-triggered. Script executes in its own overlay.
- **On-change scripts** — trigger when specific fields/entities change. Produce a script overlay for user review.
- **Sandbox** — scripts start with empty environment. Core selectively exposes APIs (`core.get_field()`, `core.set_field()`, `core.create_entity()`, etc.). External capabilities (network, filesystem) require explicit grants.
- **Async** — `core.await()` for I/O, backed by tokio.

## HTTP and WebSocket API

### HTTP API (RPC-style)

```
POST /workspace/create          POST /workspace/join

POST /tables/create             GET  /tables/list
POST /records/create            GET  /records/list?table=...
POST /records/update            GET  /records/:id
POST /records/delete

POST /edges/create              GET  /edges/list?entity=...
POST /edges/reorder

POST /overlays/create           GET  /overlays/list
POST /overlays/activate         POST /overlays/stash
POST /overlays/commit           POST /overlays/discard

POST /modules/install           GET  /modules/list
POST /modules/adopt
POST /field-mappings/confirm

POST /conflicts/resolve         GET  /conflicts/list

POST /scripts/run               GET  /scripts/list

POST /blobs/upload              GET  /blobs/:hash
GET  /blobs/:hash/thumbnail

POST /undo                      POST /redo
```

### WebSocket events

```
{ "type": "records_changed",     "table": "...", "entity_ids": [...] }
{ "type": "conflict_opened",     "entity_id": "...", "field": "..." }
{ "type": "conflict_resolved",   "entity_id": "...", "field": "..." }
{ "type": "overlay_drift",       "overlay_id": "...", "fields": [...] }
{ "type": "sync_peer_connected", "peer_id": "..." }
{ "type": "sync_progress",       "remaining": 42 }
{ "type": "script_completed",    "script": "...", "overlay_id": "..." }
{ "type": "blob_available",      "hash": "..." }
```

The client queries once on startup, then relies on WebSocket events to know when to re-query.

## Bot Harness

Multi-peer simulator for testing, implemented as `#[cfg(test)]` code in the engine crate.

### Usage

```rust
let mut network = TestNetwork::new();
let peer_a = network.add_peer().await;
let peer_b = network.add_peer().await;

peer_a.create_table("Contacts", &[("name", Text), ("email", Text)]).await;
peer_a.create_record("Contacts", &[("name", "Jane"), ("email", "jane@co")]).await;

network.connect(peer_a, peer_b).await;
network.wait_for_sync().await;

assert_eq!(peer_b.list_records("Contacts").await.len(), 1);
```

### Capabilities

- **N peers** with independent engine + SQLite
- **Connect/disconnect** to control topology
- **Partition simulation** — disconnect subsets, edit on both sides, reconnect and verify merge
- **Concurrent edit simulation** — edit same field while disconnected, verify conflict surfaces
- **Ordering verification** — all peers derive identical state hash
- **Script execution** — trigger on one peer, sync to others
- **Blob replication** — upload on one peer, verify availability after sync

### Test categories

- **Convergence** — N peers, arbitrary edits, identical state hashes after full sync
- **Conflict** — concurrent scalar edits, CRDT merges, mapped field conflicts, late-arriving edits
- **Overlay** — commit, discard, delta-based drift (Keep Mine / Use Canonical), script overlay independence, swap/stash
- **Sync** — partition/reconnect, vector clock correctness, blob on-demand fetch
- **Module** — install, adopt, field mapping confirmation, script triggers

## Project Structure

```
openprod/
├── Cargo.toml                    # workspace root
├── crates/
│   ├── core/                     # domain types, operation types, HLC, vector clocks, identity
│   │   └── src/
│   │       ├── entity.rs
│   │       ├── operations.rs
│   │       ├── hlc.rs
│   │       ├── identity.rs       # Ed25519 keypair, signing, verification
│   │       ├── vector_clock.rs
│   │       └── types.rs
│   │
│   ├── storage/                  # Storage trait + SQLite implementation
│   │   └── src/
│   │       ├── trait.rs
│   │       └── sqlite.rs
│   │
│   ├── engine/                   # state derivation, conflicts, overlays, rules
│   │   └── src/
│   │       ├── state.rs
│   │       ├── conflicts.rs
│   │       ├── overlays.rs
│   │       ├── rules.rs          # table membership rules, query scope, cycle detection
│   │       ├── modules.rs
│   │       ├── blobs.rs
│   │       └── undo.rs
│   │
│   ├── scripts/                  # Lua scripting sandbox
│   │   └── src/
│   │       ├── runtime.rs
│   │       └── triggers.rs
│   │
│   ├── sync/                     # LAN sync protocol
│   │   └── src/
│   │       ├── mdns.rs
│   │       ├── peer.rs
│   │       └── blob_sync.rs
│   │
│   ├── server/                   # HTTP + WebSocket API
│   │   └── src/
│   │       ├── routes.rs
│   │       └── events.rs
│   │
│   └── harness/                  # test framework
│       └── src/
│           ├── network.rs
│           └── assertions.rs
│
├── electron/                     # Electron frontend
│   ├── package.json
│   └── src/
│
└── modules/                      # built-in modules
    └── contacts/
        ├── module.toml
        ├── views/
        └── scripts/
```

### Dependency flow (one direction only)

```
server → engine → storage → core
              ↘ scripts
sync → engine
harness → engine + sync (test only)
```

## Implementation Phases

### Phase 1 — Core types + Storage + Oplog

Build `core` and `storage` crates. Entity/facet types, operation types, Ed25519 identity and signing, HLC (12-byte), single SQLite database per workspace (`oplog.db`) with logical table separation, oplog append/query. First harness tests: append operations, verify signatures, verify state derivation, verify canonical ordering with `(hlc, op_id)` sort.

### Phase 2 — Engine fundamentals

State derivation, command/query separation, undo/redo. Harness tests: create tables, add records, edit fields, undo, redo, verify state at each step.

### Phase 3 — Conflicts + Overlays

Vector clocks, conflict detection, overlay management (create, activate, swap, stash, commit, discard, delta-based drift with Keep Mine / Use Canonical). Harness tests: concurrent edits produce conflicts, overlay commit/discard works, conflicting drift warns user.

### Phase 4 — LAN Sync

mDNS discovery, peer connections, vector clock exchange, op replication. Harness tests: two peers sync, partition/reconnect converges, N-peer mesh converges, state hash verification.

### Phase 5 — Modules + Scripting

Module loading (TOML schema), field mapping suggestions/confirmation, rules engine (table membership rules, query scope, cycle detection, runtime safety), Lua sandbox, manual + on-change trigger scripts, script overlays. Harness tests: module adoption creates tables, rules correctly manage table membership, scripts produce operations in overlays, on-change triggers fire correctly.

### Phase 6 — Blobs

BLAKE3 content-addressed store, attach to entities, LAN blob sync on-demand. Harness tests: upload blob, attach to record, sync to peer, verify retrieval by BLAKE3 hash.

### Phase 7 — HTTP/WebSocket API + Electron shell

API layer on top of engine, WebSocket event stream, minimal Electron app that connects and renders tables.

### Phase 8 — Thumbnails (fast-follow)

Image/PDF preview generation, served via blob API.
