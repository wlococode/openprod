# Core Feature Set and Invariants

## Operations: Mutation Model

- Operations are immutable once committed
- Operations must be schema-versioned
- Replay of operation sequence is deterministic
- Idempotency: Duplicate operations must always converge to identical state
- Operations attibute an **actor ID** and **timestamp** (HLC)
- State is never mutated without an explicit operation

### Open:

- Ordering semantics
- Cryptographic signing details
- Compression
- Stream formats
- Batch/bundled operation pipeline

## Oplog & History

- Append-only source of canonical truth
- Full deterministic state reconstructable from oplog
- History is never deleted, only superseded

### Open:

- Snapshotting for new clients, catch-up
- Dedupe/skip superseded ops when building snapshot?
  - Could be problematic for leader sequencing
- Pruning strategies
- Archiving
- Checkpoints, flattening, compression

## Actor Identity

- Stable actor identifiers
- Actor ID ≠ user account
- Every operation has exactly 1 actor
- Actor identity is immutable, survives through workspace lifecycle

### Open:

- Hardware binding
- Anti-spoofing
- User account linkage

## Persistence and Storage

- Atomic append of operations
- Crash safety guarantees
- Snapshot-at-index semantics
- Asset references are content-addressed or immutable

### Open:

- Sharding
- Cloud storage/sync
- DB abstraction layer (SQLite local, Postgres cloud)

## Entity/Facet/Edge Model

- Stable entity IDs
- Facets are plugin-owned
- Edges are explicit and typed
- Entity merging via redirects
- Redirect resolution rules

### Open:

- Optimized graph traversal
- Indexing

## Cross-Plugin Identity

- Plugins never assume shared identity
- Cross-plugin equivalence is explicit
- User-defined binding/merging
- Identity never changes without user interaction

### Open:

- Model: "Concepts", RDF, etc.
- Canonical identity resolution
- Advanced schema UX

## Conflicts & Resolution

- Conflict occurs any time two or more peers edit the same data (entity, field, relationship) while not actively syncing/connected
- N-way detection, presentation, resolution
- Detected any time two peers sync
- When open but not resolved, canonical data is HLC LWW value, with interface flag/note
- Resolved conflicts are re-opened if new peer adds new conflicting state, but last resolved state is favored over LWW
- Resolution produces an operation
- Conflicts are auditable and reversible

### Open:

- UI presentation
- Auto-resolution heuristics
- Batch conflict tooling
- Avoid conflicts with CRDT model, could be wrong use case

## Queries & Derived Views

- Queries are read-only
- Deterministic results given same state
- Binding-aware semantics
- No hidden mutations

### Open:

- Query language syntax
- Performance caching
- Incremental materialization

## Plugins

- TOML schema declaration
- Plugins own facets
- No direct state mutation; everything is an operation
- Capability request/gating
- Backend "jobs" runtime for intensive tasks, produce op bundles

### Open:

- Distribution/marketplace
- Sandboxing
- WASM/process isolation
- Runtime view and job registry, avoid restarts

## Replication & Sync

- peer to peer LAN collaboration
- Avoid host-client, goal is failover safety
- Supports partitions (same workspace, isolated sessions can work and resync)
- Deterministic ordering after merge
- Conflict states representable and available offline
- Sequencing

### Open:

- Peer discovery (mDNS?)
- Leader election
- CRDT/rich text fields
- Transport
- Cloud sync
- Permission structures and enforcement, particularly with no centralized host
- Open to exploring other collaboration structures
