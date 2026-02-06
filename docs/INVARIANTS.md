# Core Invariants

This document defines the rules that must always hold true, regardless of implementation details. These invariants reflect the revised architecture (Feb 2026 design review). For detailed specifications, see [spec/](spec/README.md).

---

## Operations: Mutation Model

- Operations are immutable once committed
- Operations must be schema-versioned
- Replay of operation sequence is deterministic
- Idempotency: duplicate operations must always converge to identical state
- Operations attribute an **actor ID** and **timestamp** (HLC)
- State is never mutated without an explicit operation

### Granularity & Bundles

- Operations are stored at field granularity
- Operations are grouped into **bundles** for atomicity
- A bundle either fully commits or fully fails
- Undo/redo operates on bundles, not individual ops
- Scripts produce exactly one bundle per execution

### Undo/Redo

**Anchor invariant:** Undo/redo is per-user, operates on bundles, and gracefully handles conflicts.

---

## Oplog & History

- Append-only source of canonical truth
- Full deterministic state reconstructable from oplog
- History is never deleted, only superseded
- Every op has a checksum (hash of content)
- Corrupt ops are detected and quarantined, not applied

### Canonical History Ordering

- Every valid operation has a deterministic position in canonical history
- Canonical history ordering is identical across all peers that have integrated the same operations
- Wall-clock timestamps are untrusted metadata and must not determine history order

---

## State Model

- State is always derived from oplog
- No state is authoritative
- Rebuilding state is always legal
- Restarting the app produces identical state

---

## Tables & User-Facing Data Model

**Anchor invariant:** Users see tables, records, and fields. The entity/facet model is internal architecture, not user-facing. Modules declare tables; the core maps table operations to entity/facet operations internally.

### Table Declarations

- Modules declare tables with schemas (TOML manifests)
- A table declaration creates a corresponding facet internally
- Querying a table returns all entities that have the corresponding facet attached
- Creating a record in a table creates an entity and attaches the table's facet

### Per-Entity Table Membership

- An entity may belong to zero or more tables simultaneously
- Table membership is per-entity, not per-table-pair
- Table-level linking ("all contacts are attendees") is a convenience shortcut that applies per-entity membership in bulk
- Users can manually add or remove individual records from tables
- Rules can automate per-entity membership (e.g., "cues in Lighting table where `is_called == true` also appear in SM Cues table")
- Adding an entity to a table attaches the corresponding facet; removing it detaches the facet

### Table Linking

- Table linking replaces kind-compatibility as the cross-module integration mechanism
- When tables are linked, the system warns on unlikely combinations; the user decides
- Unlinking tables detaches facets and copies shared data to new standalone entities
- Dedup/matching rules are scoped to tables, not global

---

## Field Mapping

**Anchor invariant:** Field mappings are always user-controlled. The system may suggest mappings, but no mapping activates without explicit user confirmation.

- Modules declare shared key *suggestions* in their manifests (developer intent)
- Shared key suggestions do not auto-activate on module install
- On module adoption or first table-linking, the system presents suggested field mappings based on shared key overlap
- The user reviews and confirms or rejects each suggested mapping
- Confirmed mappings behave identically to the legacy shared key model (single semantic field across modules)
- Users can create custom field mappings beyond what modules suggest
- Templates (e.g., "Stage Management" starter) may pre-confirm mappings for zero-friction onboarding
- All field-mapping confirmations and rejections are recorded as operations in the oplog

### Field Namespacing

- Shared (mapped) fields: multiple modules can read/write the same data
- Namespaced fields: private to a module (`module.field` format)
- All writes are attributed to their source for auditability

---

## Staging Overlays

**Anchor invariant:** Overlays answer "Show me what this will do before it becomes real." All overlay operations are local-only until explicitly committed.

- Overlays are isolated from canonical state
- Overlay operations do not affect canonical history until explicitly committed
- Queries and conflicts operate identically within overlays
- Overlays may be discarded without affecting canonical state or history
- Overlay actions are atomic: commit all or discard all
- Knockout removes individual operations before commit
- Commit is atomic: overlay either fully commits or fully fails
- Overlays stored in local SQLite, never synced to peers
- Overlays persist across restarts; no auto-resume on startup
- Committing overlay that touches conflicted field resolves the conflict
- Display priority: overlay > canonical (overlay values shown when active)
- Overlay commit is atomic (wrapped in BEGIN IMMEDIATE / COMMIT transaction)
- Overlay deactivated before commit to prevent `execute_internal` re-routing ops back into the overlay
- `scan_overlay_drift` called from both `ingest_bundle` and `commit_overlay` for cross-overlay drift detection
- Commit blocked if unresolved drift exists (`UnresolvedDrift` error)
- ClearField uses tombstones (value=NULL + LWW guard) for correct out-of-order sync ingestion
- ClearEdgeProperty uses tombstones (value=NULL + LWW guard) for correct out-of-order sync ingestion
- Overlay undo/redo is per-overlay and non-persistent (in-memory, cleared on overlay switch or restart)

---

## Actor Identity

- Actor identity is an Ed25519 keypair (public key = actor ID)
- Every operation is signed by the author's Ed25519 private key
- Every operation has exactly one actor
- Each device has its own keypair
- Actor identity is immutable once created
- Public keys are exchanged during workspace join
- Operation signatures are verified by all peers on receipt

---

## Workspace Model

- **Workspace** = isolated oplog namespace with unique ID
- Entities belong to exactly one workspace
- Entity IDs are workspace-scoped, not globally unique
- Sync only occurs between peers with the same workspace ID
- Different workspaces never leak or mutate each other's data

---

## Assets & Blobs

- Blobs are immutable and content-addressed (hash = identity)
- Modifying content creates a new blob with a new hash
- Identical content = identical hash = automatic deduplication
- Asset deletions are recorded as operations
- Blobs may be GC'd after retention window if unreferenced
- Oplog replay reconstructs entity state without requiring blob data

---

## Persistence and Storage

- Atomic append of operations
- Crash safety: partial writes never corrupt oplog (SQLite WAL guarantees)
- On crash recovery, database is consistent (WAL replay)
- Incomplete bundles are discarded on recovery (never partially committed)

---

## Entity Model (Internal Architecture)

**Anchor invariant:** An entity is pure identity. All data lives in fields attached to the entity. The entity/facet model is the internal engine; users interact with tables and records.

- Entities have stable IDs (within workspace)
- An entity is a UUID with whatever fields have been attached
- Edges are explicit and typed relationships between entities
- Redirect resolution is transparent to queries
- Entity "type" is derived from table membership (which facets are attached), not from a dedicated kind field

### Entity Deletion

**Anchor invariant:** Deleting an entity never deletes other entities implicitly; only relationships (edges) and facets are affected.

- Cascade deletion of edges is atomic within the same bundle
- Undo restores entity and conditionally restores edges

---

## Facets (Internal Architecture)

**Anchor invariant:** Facets are module-owned. Modules define which fields belong to a facet. Each table declaration corresponds to a facet.

- Facet compatibility is enforced with warnings, not blocks
- For rule-triggered attachments: incompatible entities are skipped and logged
- Facets are the internal mechanism behind table membership

---

## Unified Rules

**Anchor invariant:** Rules are deterministic. Given the same state, a rule produces the same action. All rule actions are either proposed for user confirmation or auto-executed based on configuration.

- Rules are scoped to tables (not global, not kind-based)
- Rules are evaluated when relevant fields change
- Rules execute at most once per triggering event
- Rules must not self-trigger or create cycles
- Cycle detection at rule creation time (static analysis)
- Runtime safety limits: max depth 1000, max time 30s
- On-condition-lost default is propose_detach (not auto)
- Rules trigger in overlay context (output goes to overlay)
- Match rules: `null == null` is NOT a match (null is "missing data")
- Rules with `auto_accept: true` mutate data automatically; all other rules propose changes for user confirmation
- All automated mutations are user-configured, visible, and auditable

---

## Expression Language (Post-V1)

The expression language is deferred to post-v1. The anchor invariant below will apply when the expression language is implemented.

**Anchor invariant:** All computed values are user-configured, visible, and auditable. The system does not compute values unless the user has explicitly set up an expression, reference, or rule. There are no hidden formulas or implicit calculations.

**Post-v1 design direction:**
- Expressions will be the unified mechanism for field-level data transforms and queries
- A field in expression/query mode will replace the former "derived field" concept
- A field configured to reference another field replaces the former "interface slot" concept (references are V1)
- Computed fields cannot be directly edited; only their source expression can be changed
- Expression evaluation is deterministic: same inputs produce same outputs

---

## Configuration Hierarchy

**Anchor invariant:** Modules ship with sensible defaults. Users customize at workspace level. Per-entity overrides are rare but possible.

---

## Cross-Module Interoperability

**Anchor invariant:** Modules use field mappings for common data. Users configure rules for automatic facet attachment and entity matching. Nothing fuses data without explicit user consent.

- Multi-facet entities (multi-table membership) are the preferred model
- Modules function fully without cross-module awareness
- All matching and facet attachment defaults to "propose and wait for confirmation"

---

## Edges

**Anchor invariant:** Edges represent relationships, not data. The relationship itself can have properties. Edge constraints across modules are user-configured, not hardcoded.

- Edges are always directed (source -> target)
- Modules can constrain edges to entities within their own tables only
- Edge deletion cascades atomically with entity deletion

---

## Identity Repair

**Anchor invariant:** Identity repair is corrective maintenance, not primary modeling. The preferred model is multi-facet entities (multi-table membership) from the start.

- Merge is explicit, auditable, and reversible (split)
- Merge does not rewrite history
- Merge uses deterministic survivor selection (lexicographically smaller UUID survives)
- Absorbed IDs are recorded in MergeResolution table
- References to absorbed IDs resolve to survivor transparently
- Resolution chains supported (A->B->C resolves A to C)

---

## Scripts

**Anchor invariant:** Scripts emit operations like manual user actions. Scripts execute in sessions (overlay transactions) for safe preview and atomic commit.

- Scripts are written in Lua 5.4 (mature, lightweight, cross-platform)
- V1 script modes: manual (user-triggered) and on-change (triggered by data changes)
- Background scripts (OSC listeners, file watchers) are deferred to post-v1
- Async via coroutines: `core.await()` yields to Rust runtime, resumes on I/O completion
- Scripts emit operations incrementally (streaming)
- Scripts subject to normal conflict detection
- Scripts must not auto-resolve conflicts
- Scripts require user permissions and capabilities
- Capabilities control module exposure (sandboxed by default)
- Bulk conflict detection for script operations (>100)
- Trigger cycle detection at configuration time (like rules)
- Permission revocation during execution: script exits immediately
- Multi-peer scripts: operations not deduplicated, state derivation is idempotent
- Cross-platform: mlua (desktop/mobile), wasmoon (web)

---

## Conflicts & Resolution

- Conflict occurs when two or more peers edit the same field while disconnected
- Conflict detection operates at field level
- N-way detection, presentation, resolution supported
- Resolution produces an operation
- Conflicts are auditable and reversible
- Unresolved conflicts display LWW value with conflict flag
- Awareness events: short-lived notifications for near-simultaneous edits (UX only)

### Key Conflict Semantics

- Concurrent edits to different fields of same entity do not conflict
- Mapped fields (confirmed field mappings) are a single semantic field for conflict purposes
- Conflict resolution requires `can_edit` permission on the field (post-v1; no permission enforcement in V1)
- At most one resolution per conflict state
- Resolution is immutable once recorded; revision creates new operation
- Conflict branch tips stored exclusively in `conflict_values` table (keyed by `conflict_id, actor_id`), not inline on `ConflictRecord`
- `add_conflict_value` upserts by actor (ON CONFLICT DO UPDATE) for N-way extension

### Late-Arriving Edits

- Conflicts may be reopened by edits from peers unaware of prior resolution
- Canonical state remains determined by most recent explicit resolution

### Garbage Collection

- GC may remove conflicting operation payloads after retention
- GC must not remove resolution operations
- GC must preserve: conflict identifier, actors, chosen outcome, logical placement

---

## Queries & Derived Views

- Queries are read-only and deterministic
- Queries target tables (user-facing) which resolve to facet-based entity queries internally
- Derived entity sets are read-only (cannot be directly mutated)
- Materializing derived entities requires explicit user action
- Queries evaluated against consistent state snapshots

---

## Modules

- Modules declare tables with schemas (TOML) and provide views (TypeScript) and scripts (Lua)
- Modules own their facets and namespaced fields
- Modules declare field shared key suggestions in their manifests
- No direct state mutation; everything produces operations
- Capabilities granted per-user and enforced by core

### Scripts

- Scripts produce operation bundles, never mutate state directly
- Script output staged until complete; failed scripts produce no ops
- Partial script output discarded entirely

### Imports

- Imports run inside staging overlays by default
- Imports must support dry-run/preview before committing
- Failed imports produce no operations

---

## Replication & Sync

**Anchor invariant:** Given the same set of valid operations, all peers converge to identical state. Partitions require no setup or detection -- they just work.

### Sync Modes (V1)

V1 supports two sync modes, both using the same underlying oplog-based protocol:

- **LAN session** -- Devices discover each other via mDNS and sync directly. No internet required.
- **Offline** -- No sync. Changes accumulate locally. Merge on reconnect.

**Post-v1:** Cloud server sync (a central Rust server that clients sync to via WebSocket) is deferred to post-v1.

### Sync Invariants

- Local-first: all data lives on device, always works offline
- Oplog-based sync: "send me all ops I don't have"
- Deterministic ordering via HLC + op_id tiebreaker (LWW: HLC comparison first, then op_id for deterministic total ordering)
- Strong eventual consistency (SEC)
- Partitions are implicit; no ceremony to enter or exit
- Multi-partition merge via pairwise gossip (no special N-way protocol needed)
- Conflicts detected via causal concurrency (branch tips only)
- Users on isolated networks (e.g., lighting ETCNet, sound network) without WAN access must still be able to sync on their local subnet; network topology cannot be assumed
- Peer discovery: mDNS (LAN mode); server registration deferred to post-v1 (cloud mode)

### Sync Behavior

- Sync is non-blocking; users continue working during sync
- Local edits visible immediately to author (optimistic)
- Peers cannot see others' unsequenced edits
- Eventual convergence guaranteed

---

## Trust & Validation Model

- No peer is inherently trusted
- All peers independently validate all operations
- Operations signed by author's Ed25519 private key
- Signatures verified on receipt using the author's known public key
- Invalid signatures cause operation rejection
- Hash-linked history; modification detectable
- Authorization evaluated locally using known history

---

## Permissions & Roles (Post-V1)

Permissions and role-based access control are deferred to post-v1. In V1, there is no role enforcement -- all participants in a workspace can edit all data.

**Post-v1 design direction:**
- Roles will be workspace-scoped
- Multiple roles per user; permissions union together
- Permission scopes: Global -> Table -> Facet -> Field (most specific wins)
- Rule-triggered actions use triggering user's authority
- Offline permission revocation converts ops to proposals
- Default roles: Viewer (read-only), Editor (view/edit/create/delete), Admin (full access)

---

## Workspace Lifecycle

- Bootstrap is self-signed and self-authorizing (first operation)
- Recovery key generated at workspace creation
- Join modes: Open, Access Key, Request
- Any user can fork a workspace (permission-gated `can_read` post-v1)
- Fork creates new workspace (no history, forker as creator)

---

## Summary: Confidence Levels

| Decision | Status | Confidence |
| -------- | ------ | ---------- |
| Field-level ops with bundle grouping | Decided | High |
| Blob immutability (content-addressed) | Decided | High |
| Workspace isolation | Decided | High |
| SQLite WAL crash safety | Decided | High |
| Bundle-level entity create/delete | Decided | High |
| Field-level conflict granularity | Decided | High |
| Sync is non-blocking | Decided | High |
| Optimistic local edits visible immediately | Decided | High |
| Canonical history ordering deterministic | Decided | High |
| Wall-clock timestamps untrusted | Decided | High |
| Operations signed by author (Ed25519) | Decided | High |
| Ed25519 keypair identity (public key = actor ID) | Decided | High |
| Authorization evaluated locally | Decided | High |
| Tables as user-facing data model (replaces kind) | Decided | High |
| Per-entity table membership | Decided | High |
| Table-linking replaces kind-compatibility | Decided | High |
| Field mappings user-confirmed (no auto-binding) | Decided | High |
| Shared key suggestions presented but not auto-activated | Decided | High |
| Unified rules scoped to tables | Decided | High |
| Rules deterministic (same state -> same action) | Decided | High |
| Multi-facet entities preferred model | Decided | High |
| Identity repair is corrective, not primary | Decided | High |
| Entity deletion never cascades to other entities | Decided | High |
| Edges directed (source -> target) | Decided | High |
| Staging overlays isolated from canonical | Decided | High |
| One overlay active at a time | Decided | High |
| Display priority: overlay > canonical | Decided | High |
| Overlays local-only, never synced | Decided | High |
| Scripts in Lua 5.4, async via coroutines | Decided | High |
| Scripts subject to normal conflict detection | Decided | High |
| Deterministic ordering via HLC + op_id tiebreaker | Decided | High |
| HLC format: 12-byte (8 wall + 4 counter), no node ID | Decided | High |
| HLC future drift: reject >5min, don't poison local | Decided | High |
| HLC stale ops: accept but flag for review >7 days | Decided | High |
| LAN sync via mDNS (no internet required) | Decided | High |
| Offline mode with merge on reconnect | Decided | High |
| Isolated network sync (no WAN assumption) | Decided | High |
| Partitions implicit, no detection required | Decided | High |
| Actor identity via Ed25519 keypair | Decided | High |
| Join modes: Open, Access Key, Request | Decided | High |
| Fork creates new workspace without history | Decided | High |
| Overlay isolation (no cross-overlay awareness) | Decided | High |
| Stale ops apply to canonical, flagged for review | Decided | High |
| Rule query isolation sees overlay state | Decided | High |
| Entity creation is explicit CreateEntity operation | Decided | High |
| Edge cascade computed at deletion, stored in operation | Decided | High |
| Facet compatibility: user=warn, rule=skip | Decided | High |
| Overlay undo stack non-persistent | Decided | High |
| Query context follows triggering operation context | Decided | High |
| Query overlay merging: per-field override, sparse | Decided | High |
| Overlay isolation: queries cannot see other overlays | Decided | High |
| Query results not permission-filtered (permissions post-v1) | Decided | High |
| Smart Fields: Discrete + Reference in V1, Query mode post-v1 | Decided | High |
| All computed values user-configured and auditable | Decided | High |
| All automated mutations user-configured and auditable | Decided | High |
| Unified Lua scripting (replaces WASM jobs) | Decided | High |
| Rules engine included in V1 | Decided | High |
| Scripts capability-gated | Decided | High |
| Conflicts based on causal concurrency (branch tips only) | Decided | High |
| SetField on CRDT fields rejected at validation | Decided | High |
| Bundle atomicity via WAL with rollback | Decided | High |
| Rule authority frozen at triggering op's HLC | Decided | High |
| Rule + triggering op = atomic unit | Decided | High |
| Script cancellation: overlay mode discards, committed bundles stay | Decided | High |
| Script shutdown timeout configurable (max 60s) | Decided | High |
| Overlay drift: Keep Mine = causal acknowledgment, no conflict | Decided | High |
| Overlay drift: Commit without addressing = courtesy notification | Decided | High |
| Proposals deferred to post-v1 | Decided | High |
| Approval workflows deferred to post-v1 | Decided | High |
| Schema evolution deferred to post-v1 | Decided | High |
| Snapshots, segments, GC deferred to post-v1 | Decided | High |
| Wire format deferred to post-v1 | Decided | High |
| Permissions deferred to post-v1 (no role enforcement in V1) | Decided | High |
| Expression language deferred to post-v1 | Decided | High |
| Cloud sync deferred to post-v1 (V1 is LAN-only) | Decided | High |
| Background scripts deferred to post-v1 (V1: manual + triggers) | Decided | High |
| Script dependency management | Open | Medium |
| Complex rule conditions (AND, OR, comparisons) | Open | Medium |
| Snapshot format (MessagePack vs SQLite dump) | Open | Medium |
