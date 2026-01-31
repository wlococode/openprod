# Core Feature Set and Invariants

This document defines the rules that must always hold true, regardless of implementation details.

---

## Operations: Mutation Model

- Operations are immutable once committed
- Operations must be schema-versioned
- Replay of operation sequence is deterministic
- Idempotency: Duplicate operations must always converge to identical state
- Operations attribute an **actor ID** and **timestamp** (HLC)
- State is never mutated without an explicit operation

### Granularity & Bundles

- Operations are stored at field granularity
- Operations are grouped into **bundles** for atomicity
- A bundle either fully commits or fully fails
- Undo/redo operates on bundles, not individual ops
- Jobs produce exactly one bundle per execution

### Bulk Operations

Bulk operations are collections of changes applied atomically as a single bundle.

- Bulk operations are represented as bundles
- Bulk operations must be previewable before commit
- Bulk operations may be staged in overlays for review
- Bulk operations must be atomic when committed
- Bulk operations may be proposed instead of directly committed
- Bulk operation preview shows all affected entities and fields

### Open

- Compression format
- Stream/wire formats
- Bundle types (user_edit, job_output, import, merge_resolution)?
- Max bundle size advisory?

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
- New peers reconstruct history deterministically and observe the same ordering as existing peers

### Timestamps & Causality

- Wall-clock timestamps are untrusted metadata and must not determine history order
- Clock skew is treated as untrusted metadata
- Causal metadata (HLC, wall-clock time, author) may be displayed as annotations, not as ordering authority

### Concurrency Presentation

- Concurrent operations may be labeled or grouped as concurrent, even when a deterministic internal order exists
- History presentation must not imply false causality between concurrent operations

### Open

- Snapshotting for new clients, catch-up
- Dedupe/skip superseded ops when building snapshot?
- Pruning strategies
- Archiving
- Checkpoints, flattening, compression

---

## State Model

- State is always derived from oplog
- No state is authoritative
- Rebuilding state is always legal
- Restarting the app produces identical state

### Open

- Caching strategies
- Partial materialization
- In-memory vs disk

---

## Staging Overlays

Staging overlays are temporary, non-canonical layers of operations that enable safe experimentation and preview.

### Core Semantics

- Overlays are isolated from canonical state
- Overlay operations do not affect canonical history until explicitly committed
- Projection, queries, conflicts, and proposals operate identically within overlays
- Overlays may be discarded without affecting canonical state or history
- Committing an overlay produces explicit operations added to canonical history
- Commit is atomic: overlay either fully commits or fully fails

### Overlay Behavior

- Overlays behave like canonical state for all read operations
- Parameterized queries may reference overlay state as input
- Overlays support conflict detection against canonical state
- Multiple overlays may exist simultaneously (per-user, per-task)
- Overlays do not sync between peers until committed

### Use Cases

- Safe experimentation and what-if scenarios
- Preview of bulk operations, transforms, or imports
- Staged data entry (enter data, then commit)
- Draft workflows before committing changes

### Open

- Overlay persistence across sessions
- Overlay sharing between peers (draft collaboration)
- Overlay merge strategies when canonical state changes

---

## Actor Identity

- Stable actor identifiers
- Actor ID ≠ user account
- Every operation has exactly 1 actor
- Actor identity is immutable, survives through workspace lifecycle
- Actor IDs may span multiple workspaces

### Open

- Hardware binding
- Anti-spoofing
- User account linkage

---

## Workspace Model

- **Workspace** = isolated oplog namespace with unique ID
- Entities belong to exactly one workspace
- Entity IDs are workspace-scoped, not globally unique
- Sync only occurs between peers with the same workspace ID
- Different workspaces never leak or mutate each other's data

### Templates & Cloning

- Templates are point-in-time snapshots used to initialize new workspaces
- Clone creates new workspace with current state, no history
- Forking (with history) is not supported in v1
- No automatic cross-workspace sync in v1

### Personal Libraries

- Personal/library data is isolated from workspace data
- Imports from personal library create copies, not references
- Personal library syncs independently from workspace sync

### Open

- Template format: full oplog snapshot or just entity state?
- Workspace archiving (read-only mode)?
- Cross-workspace references in future versions?

---

## Assets & Blobs

- Blobs are immutable and content-addressed (hash = identity)
- Modifying content creates a new blob with a new hash
- Identical content = identical hash = automatic deduplication
- Asset deletions are recorded as operations
- Deleting an asset does not delete the blob immediately (GC handles cleanup)

### Garbage Collection

- Blobs may be GC'd after retention window if unreferenced by active ops
- GC never deletes blobs referenced by ops within retention window
- Ops referencing GC'd blobs remain valid; blob retrieval returns "unavailable"

### Replay & Storage

- Oplog replay reconstructs entity state without requiring blob data
- Ops that reference assets store metadata (hash, filename, size) inline
- Blob absence is a retrieval failure, not a state corruption
- Blobs are stored and synced compressed (transparent to plugins)

### Open

- Retention window duration (90 days? configurable?)
- Cold storage integration for archived blobs?
- Plugin-declared asset types with different retention rules?
- Compression algorithm (zstd?)

---

## Persistence and Storage

- Atomic append of operations
- Crash safety: partial writes never corrupt oplog (SQLite WAL guarantees)
- On crash recovery, database is consistent (WAL replay)
- Incomplete bundles are discarded on recovery (never partially committed)
- Snapshot-at-index semantics
- Asset references are content-addressed or immutable

### Open

- Sharding
- Cloud storage/sync
- DB abstraction layer (SQLite local, Postgres cloud)

---

## Entity/Facet/Edge Model

- Stable entity IDs (within workspace)
- Facets are plugin-owned
- Edges are explicit and typed
- Entity merging via redirects
- Redirect resolution is transparent to queries

### Entity Lifecycle

- Bundles declare entity creation via `creates: [entity_id, ...]` metadata
- Bundles declare entity deletion via `deletes: [entity_id, ...]` metadata
- An entity exists once it has been declared in a `creates` marker
- An entity is deleted once declared in a `deletes` marker (and not subsequently recreated)
- Writing to a non-existent entity ID without a `creates` marker is an error
- Entity creation metadata provides audit trail (created_by, created_at derived from bundle)
- No separate CREATE_ENTITY operation type; creation is bundle metadata

### Open

- Optimized graph traversal
- Indexing strategies
- Entity ID generation strategy (UUIDs? deterministic?)

---

## Cross-Plugin Identity

- Plugins never assume shared identity
- Cross-plugin equivalence is explicit (via Concepts and Bindings)
- User-defined binding/merging
- Identity never changes without user interaction

---

## Concepts

Concepts are the mechanism for unifying identity across plugins while preserving safety, reversibility, auditability, and offline correctness.

**Anchor invariant:** Concept entities own identity; plugin entities describe identity; bindings connect them; nothing else fuses data.

### Three-Layer Model

- **Concept definitions** are schema-level objects that define semantic types (e.g., "Person", "Cue")
- **Bindings** declare that a plugin facet is semantically compatible with a Concept definition
- **Concept entities** are instance-level objects that represent real-world identity

These three layers are distinct and independently created.

### Concept Definition Lifecycle

- Concept definitions are created explicitly by user action
- Creating a Concept definition defines a semantic type but does not create any entities
- Concept definitions are schema-level objects and are auditable
- Concept definitions may declare fields that plugin facets can bind to

### Binding Semantics

- Binding a plugin facet to a Concept definition declares semantic compatibility, not identity equivalence
- Binding does not create Concept entities
- Binding does not move, merge, or rewrite existing plugin entities
- A plugin must bind its facet to a Concept definition before any of its entities can participate in a Concept entity equivalence assertion
- Facet-to-Concept binding is a prerequisite for entity-to-Concept-entity binding

### Concept Entity Creation

- Concept entities are created only through explicit user assertions of entity equivalence
- Asserting that two or more plugin entities refer to the same real-world thing creates a Concept entity
- No Concept entity may be created implicitly or as a side effect of any other operation
- Concept entity creation is an auditable operation distinct from binding

### Canonical Field Values

- Creating a Concept entity does not automatically choose canonical field values
- If bound plugin entities provide differing values for the same Concept field, that field enters a conflicted state immediately
- Canonical field values are established only through explicit conflict resolution
- No plugin field value is discarded or overwritten during Concept entity creation
- All competing values remain preserved and auditable until explicitly resolved
- Binding and Concept entity creation succeed even if conflicts exist
- Conflicts introduced by Concept entity creation are surfaced immediately

### Query Semantics

- Concept queries return only Concept entities
- Plugin entities not bound to a Concept entity are excluded from Concept queries
- The system must not infer Concept membership without explicit user action

### Plugin Entity Independence

- Plugin entities exist independently of Concepts
- Unbound plugin entities retain full plugin-local identity and functionality
- Plugin entities may be edited freely without Concept binding
- Plugins must function fully without any Concept bindings
- Unbound plugin entities are eligible to be bound to Concept entities but are not implicitly Concept members
- Eligibility for binding does not confer semantic identity

### Edit Isolation (Unbound Entities)

- Plugin-local edits to unbound entities do not affect any Concept
- Edits to unbound entities do not create conflicts

### Binding-Time Conflict Detection

- When a plugin entity is bound to a Concept entity, its current field values participate in conflict detection at binding time
- Binding an existing plugin entity to a Concept entity may introduce new conflicts
- If a bound plugin field value differs from the Concept's canonical value, a new conflict is created
- Binding does not silently discard plugin-local values
- Prior conflict resolutions remain valid and are not retroactively invalidated by new bindings
- Canonical state remains unchanged until an explicit conflict resolution occurs
- Binding-time conflicts are new conflicts, not reopenings of prior resolved conflicts

### Identity Assertion Semantics

- Entity equivalence is established through a single explicit equivalence assertion
- An equivalence assertion may create a Concept entity atomically if one does not exist
- Concept entity creation and entity binding are part of one semantic operation
- Users assert identity once; the system handles internal steps atomically
- Equivalence assertions are auditable as single operations

### Concept Membership

- A Concept entity may have one or more plugin entities bound to it
- Concept membership does not require equivalence to multiple plugin entities
- Binding a single plugin entity to a Concept establishes identity, not deduplication
- Concept entities represent identity, not equivalence count

### Relationship Direction

- Plugin entities reference Concept entities to declare semantic identity
- Concept entities do not own or directly reference plugin entities
- Relationship direction is plugin entity → Concept entity
- Identity flows upward; data stays local

### Field-Level Conflict Granularity (Concepts)

- Conflicts are scoped to individual Concept fields, not entire entities
- A Concept entity may have some fields conflicted and others resolved
- Conflicts are evaluated independently per Concept field
- When multiple bound plugin fields provide differing non-null values for the same Concept field, a conflict is created
- Null or missing values do not constitute conflicts
- While a Concept field is conflicted, canonical state remains stable and deterministic
- A conflicted Concept field exposes all competing values for explicit resolution
- Conflicts are non-blocking; canonical state remains usable until resolved

### Value Projection After Resolution

- Resolving a Concept field conflict establishes a canonical value for that field
- After resolution, all bound plugin fields project the canonical Concept value
- Divergent plugin-local values remain auditable in history

### Bound Field Editing

- Bound plugin fields remain editable at all times
- Editing a bound plugin field asserts new semantic intent
- If no competing value exists, the edit updates the Concept's canonical value directly
- Editing a resolved Concept field does not create a conflict unless another competing value exists concurrently
- Sequential edits to a Concept field do not create conflicts
- Conflicts reappear only under concurrent or divergent assertions

### Conflict Auditability (Concepts)

- All conflicting and resolved values are preserved in history
- Conflict creation, resolution, and subsequent edits are fully auditable
- No conflict is created, resolved, or reopened implicitly
- Conflict semantics do not depend on the cause of the conflict
- All conflicts are resolved using the same rules, regardless of how they were introduced

### Unbinding Semantics

- Unbinding a plugin entity from a Concept removes semantic linkage only
- Unbinding does not delete data, revert history, or infer prior values
- After unbinding, plugin entities retain their last known concrete values
- Unbinding freezes the plugin entity at its current state

### Concept Entity Persistence

- Concept entities persist regardless of how many plugin entities are bound
- Concept entities are deleted only through explicit user action
- A Concept entity with a single bound plugin entity remains valid
- A Concept entity with zero bound plugin entities remains valid until explicitly deleted

### Deletion of Concept Entities

- Concept entities with zero bindings remain valid until explicitly deleted
- Deleting a Concept entity is an explicit, authorized operation
- Deleting a Concept entity does not rewrite or erase history
- Historical references to deleted Concept entities remain resolvable for audit
- Deletion produces an explicit operation recorded in history

### Post-Unbinding Behavior

- Edits to unbound plugin entities are plugin-local only
- Unbound edits do not affect Concept entities or create conflicts
- Unbinding restores full plugin-local independence

### Rebinding After Unbinding

- Plugin entities may be rebound to the same Concept entity after unbinding
- Rebinding triggers conflict detection against current canonical values
- Rebinding may introduce conflicts if values differ
- Rebinding does not implicitly resolve conflicts
- Rebinding is semantically identical to initial binding

### Rebinding After Divergence

- Rebinding a plugin entity to a Concept reintroduces shared semantic meaning
- All Concept fields are evaluated independently for conflict at rebind time
- Each differing non-null field value introduces a separate conflict
- Resolving a Concept field after rebinding establishes a canonical value
- Bound plugin fields project the canonical Concept value after resolution

### History Preservation (Unbound Periods)

- All plugin-local edits made while unbound are preserved in history
- Rebinding does not rewrite, discard, or compress unbound-period history
- Unbound edits become inputs to conflict detection upon rebinding
- History during unbound periods remains fully auditable

### Directional / Subset Concepts

- A single Concept may unify entities from multiple plugins with asymmetric participation
- Subset relationships are expressed by which plugin entities are bound to a Concept entity
- Concept membership does not require participation from all bound plugins
- Subset relationships emerge from binding participation, not Concept hierarchy
- No inheritance trees or rigid ontologies; flexible user-configured identity convergence

### Concept vs Plugin Responsibility

- Concepts define shared semantic identity and canonical meaning
- Plugin entities remain plugin-scoped and autonomous
- Concepts must not own plugin-specific lifecycle or UX decisions
- The core must not attempt to infer or auto-heal domain-specific gaps
- Plugins are responsible for presenting domain-appropriate UX for incomplete or missing Concept participation
- The core guarantees correctness; plugins guarantee usability

### Canonical Projection Semantics

- When a Concept entity exists and a field is bound, canonical Concept field values project to all bound plugin fields
- Projection is semantic, not mechanical copying
- Projection does not duplicate state; it enforces consistency of meaning
- Projection never overwrites data silently; all changes are represented by operations

### Local Field Overrides

Local field overrides allow plugins to maintain controlled divergence from canonical values while preserving auditability.

- Canonical fields define shared semantic truth
- Plugins may declare specific fields as supporting local overrides
- Local overrides must not modify canonical Concept fields
- Local overrides are explicitly marked and distinguishable from canonical values
- Overrides are auditable and recorded as operations
- Removing an override restores canonical projection
- Override presence is visible to the user (e.g., "showing local nickname" indicator)
- Local overrides do not participate in conflict detection unless explicitly promoted
- Promoting an override to canonical emits a normal edit operation and enters standard conflict semantics

Use cases:
- Nicknames or local labels for entities
- Temporary display preferences
- User-specific annotations that should not sync to canonical state

### Facet Attachment vs Entity Equivalence

- Attaching a facet to an entity adds information; it does not assert identity equivalence
- Entity equivalence is established only through explicit equivalence operations
- Binding fields or facets does not imply entity equivalence

### Deletion Semantics in Concept Context

- Deleting a plugin entity removes only that plugin's participation
- Deleting a plugin entity must not delete the Concept entity
- Concept entities persist as long as at least one plugin entity remains bound

### Conditional / Rule-Based Binding

- Concept binding membership may be defined by explicit user-defined rules over entity fields
- Binding rules are evaluated deterministically and re-evaluated when relevant data changes
- Conditional binding follows the same unbinding/rebinding semantics as manual actions
- The system must not assert identity or create entities automatically due to rule evaluation

### Assisted Alignment & Entity Creation

- The system may suggest equivalence or entity creation based on explicit user-defined rules
- Suggestions must require explicit user confirmation
- Bulk equivalence operations are treated as collections of explicit equivalence assertions
- Missing counterpart entities remain unbound until explicitly created or bound
- Entity creation as part of alignment must always be an explicit user-approved action

### Incomplete / Unassigned Data

- Incomplete or unassigned data is a valid state
- Missing fields or missing Concept participation must not be treated as errors by the core
- Plugins must provide fallback UX for incomplete data

### Identity Mistake Recovery

- Identity mistakes are corrected by unbinding erroneous entity associations
- Unbinding is the sole mechanism for reversing incorrect identity assertions
- Unbinding does not rewrite or erase historical operations
- After unbinding an erroneously bound entity, it returns to unbound state with its original values intact

### Conflict Removal via Unbinding

- Conflicts exist only while competing values are present
- Removing a competing value removes the conflict
- Unbinding is not a conflict resolution; it removes conflict causes
- When an unbound entity's value was the only competing value, the conflict disappears without requiring resolution

### Post-Mistake Canonical State

- Canonical values established through resolution remain until explicitly changed
- Correcting identity does not retroactively alter prior resolution decisions
- Restoring correctness after a mistaken resolution requires explicit user action
- Resolution decisions are real historical facts even if based on mistaken identity assertions

### Audit Trail for Mistakes

- Erroneous identity assertions and their correction are preserved in history
- History must reflect both mistakes and recoveries truthfully
- No historical operation is erased due to user error
- Mistakes are auditable for forensic review and learning

### Guardrail Invariant

- Concepts unify meaning; plugin entities own data; bindings connect them; nothing else fuses identity implicitly

### Open

- Advanced schema UX for non-technical users
- Concept definition versioning and migration

---

## Transform Bindings

Transform Bindings are explicit, deterministic semantic rules that produce canonical state changes from other canonical state.

**Anchor:** Transform Bindings automate *how* truth is kept consistent, not *what* truth is.

### Core Definition

- Transform Bindings are explicit semantic rules declared by user action
- Transform Bindings read canonical state and emit explicit operations
- Transform Bindings are neither calculations nor projections

### Determinism & Replay

- Transform Bindings must be deterministic functions of canonical state
- Given identical canonical state, Transform Bindings must produce identical outputs
- Transform Bindings must be replayable from history without external context

### Execution Semantics

- Transform Bindings execute automatically when their declared source fields change
- Transform Bindings must not self-trigger or create execution cycles
- Transform Bindings execute at most once per triggering state change

### Mutation Rules

- Transform Bindings emit normal canonical operations
- All Transform Binding outputs must be represented as explicit operations
- Transform Binding outputs must be attributable and auditable
- Transform Bindings must not silently overwrite canonical state
- Transform Bindings do not suppress or merge concurrent outputs

### Conflict Interaction

- Transform Binding outputs are subject to normal conflict detection
- If concurrent transform outputs differ, a conflict is created
- Transform Bindings must not automatically resolve conflicts
- Transform Bindings must not overwrite unresolved conflicting fields

### Authority & Permissions

- Transform Binding execution is subject to authorization rules
- Executing a Transform Binding requires permission to write all affected fields
- Transform Bindings may be disabled or removed explicitly

### Safety & Transparency

- Transform Bindings must record causal metadata indicating they produced an operation
- Transform Bindings must be visible in history and audit views
- Users must be able to trace which Transform Binding produced which changes

### Explicit Non-Goals

- Transform Bindings must not infer user intent
- Transform Bindings must not merge entities
- Transform Bindings must not create Concept entities

### Lifecycle Changes

- Disabling a Transform Binding does not revert previously emitted operations
- Re-enabling a Transform Binding may emit new operations based on current canonical state
- Transform Binding lifecycle changes are explicit, auditable operations

### Open

- Transform Binding dependency ordering
- Transform Binding versioning and migration
- Transform Binding testing/simulation tools

---

## Conflicts & Resolution

- Conflict occurs when two or more peers edit the same semantic field while disconnected
- Conflict detection operates at field level, after binding resolution
- Conflict presentation groups by bundle for user clarity
- N-way detection, presentation, resolution supported
- Detected any time two peers sync
- When open but not resolved, canonical data is HLC LWW value, with interface flag
- Resolved conflicts are re-opened if new peer adds new conflicting state (last resolved state is favored over LWW)
- Resolution produces an operation
- Conflicts are auditable and reversible

### Field-Level Conflict Granularity

- Conflicts are defined at semantic field granularity, not entity granularity
- Concurrent edits to different semantic fields of the same entity do not constitute a conflict
- All non-conflicting field edits are preserved after sync; no edit is discarded due to sequencing or wall-clock timing alone
- Wall-clock ordering does not determine precedence when fields do not overlap
- After sync, canonical entity state may reflect contributions from multiple actors
- Concurrent edits to the same entity may surface as informational "awareness events" even when no conflict exists

### Conflict Surfacing

- Conflicts may be detected during sync but are surfaced only after a stable merge point
- The set of surfaced conflicts represents the complete known conflict set at that point
- On reconnection, the author continues to see their optimistic value initially
- After reconciliation reaches a stable point, conflicts are detected and surfaced
- Both conflicting values are visible for resolution; neither is silently overwritten

### Resolution as History

- Conflict resolution is always recorded as a new operation in history
- Original conflicting operations are immutable and are never modified retroactively
- Resolution operations are part of canonical history and participate in deterministic replay
- The existence of the conflict itself is never erased by resolution

### Resolution Content

- Resolution operations explicitly reference the conflict they resolve
- Resolution operations declare the chosen outcome as an explicit decision
- Resolution operations record the identity of the resolving actor
- Resolution operations record the chosen value and/or a reference to the accepted conflict state
- Resolution operations record logical time, not wall-clock time as authority

### Resolution Auditability & Retention

- Original conflicting operations remain auditable until explicitly garbage collected
- Conflict records may be garbage collected under explicit retention policy
- Garbage collection does not alter historical meaning or invalidate prior resolution decisions
- Resolution operations must remain interpretable even if referenced conflict records are no longer present

### Resolution Immutability & Revision

- Conflict resolution operations are immutable and are never modified or deleted
- Revisiting or changing a prior resolution produces a new operation
- A later resolution may supersede the outcome of an earlier resolution without invalidating it
- Canonical state reflects the most recent valid resolution or edit in sequence order
- Reversing a resolution is semantically distinct from making a new edit
- Resolution revision operations represent a change in decision about a prior conflict
- Normal edit operations represent new intent applied to the current canonical state
- Resolution revisions explicitly reference the resolution they supersede
- Normal edits do not retroactively alter conflict history
- The full lineage is preserved: original conflict → first resolution → later override

### Late-Arriving Edits & Conflict Reopening

- Conflicts are scoped to divergence since a shared known sequence
- A conflict represents competing edits made without knowledge of a prior resolution
- Conflict resolution records a decision that closes a divergence window
- A resolved conflict remains resolved until explicitly superseded by a new resolution operation
- A conflict may be reopened or extended by edits originating before the prior resolution was known
- Late-arriving edits from peers unaware of a resolution extend the existing conflict rather than creating a new one
- Edits made after a resolution was known create new conflicts, not reopen old ones
- Canonical state remains determined by the most recent explicit resolution decision
- While a conflict is unresolved or reopened, canonical state is derived deterministically and remains stable
- The reconnecting peer sees the current canonical state (determined by prior resolution)
- The reconnecting peer's offline edit is preserved and not discarded
- The reconnecting peer is notified of the conflict their edit has reopened or extended
- The system acknowledges that prior resolutions were made without knowledge of late-arriving edits

### Conflicts Across Bound Fields

- When multiple concrete fields are bound to the same Concept field, they represent a single semantic field
- Conflicts are detected and resolved at the Concept level when bindings exist
- Concurrent edits to bound fields constitute a single conflict, not multiple independent conflicts
- Resolving a Concept-level conflict updates all bound concrete fields
- Conflict resolution operations reference semantic (Concept) fields, not individual plugin fields
- Plugins project resolved semantic values into their local schema via bindings
- Resolution records semantic intent, not low-level storage details
- Fields that are not bound to a shared Concept are semantically independent
- Concurrent edits to unbound fields do not constitute a conflict, even if they belong to the same entity
- The system must not infer conflicts across plugins without explicit bindings

### Resolution Authority

- Conflict resolution is a write operation and is subject to normal authorization rules
- A conflict may be resolved only by a user authorized to write to the affected semantic field(s)
- Resolution authority is evaluated at the semantic (Concept or field) level
- Resolution requires the same permission as writing the field normally
- No special "resolve conflicts" permission exists separate from field write permission
- No special authority is granted solely by the presence of a conflict
- The system does not enforce neutrality or conflict-of-interest rules
- Users without write permission must not be allowed to resolve conflicts
- Unauthorized conflict resolution attempts are rejected and do not affect history

### Concurrent Resolution Attempts

- Conflict resolution operations target a specific unresolved conflict state
- Resolution validity is determined by whether the conflict was unresolved at the time the resolution is applied
- At most one resolution may be accepted for a given conflict state
- The first valid resolution sequenced against an unresolved conflict is accepted
- Subsequent resolution attempts targeting an already-resolved conflict are invalid
- Rejected resolution attempts do not affect canonical state
- Rejected resolution attempts may be recorded as invalid operations for audit purposes
- Resolution operations must not conflict with each other or create recursive conflicts

### Resolution Stability

- Canonical state must not oscillate due to concurrent resolution attempts
- Resolution acceptance is conditional on conflict state, not arrival order alone
- Once a conflict is resolved, canonical state remains stable until explicitly changed
- Changing a resolved outcome requires explicitly revisiting or reopening the conflict

### Conflict Garbage Collection

- Garbage collection may remove original conflicting operation payloads after retention
- Garbage collection must not remove conflict resolution operations
- Garbage collection must not erase the fact that a conflict occurred
- After GC, conflict history remains interpretable at a summary level
- GC must preserve: conflict identifier, actors involved, chosen outcome, logical placement in history
- Conflict resolution operations remain valid even if referenced conflicting ops are garbage collected
- Resolution operations must remain self-sufficient to preserve correctness after GC
- Audit trails must preserve the chosen outcome even if rejected values are unavailable
- GC must not retroactively invalidate historical meaning
- GC does not change knowledge boundaries for conflict scoping

### Conflict Compaction

- Conflict compaction must not modify or delete existing operations
- Compaction produces new summary operations
- Summaries must preserve actors, decisions, and ordering
- Detailed history may be hidden or expired, but summary truth must remain
- Compaction is interpretive summarization, not rewrite

### Partial History & Reopening

- Conflicts may be reopened even if some historical conflict details have been garbage collected
- Partial conflict history does not invalidate resolution
- Resolution decisions are based on available competing intents and canonical state

### Conflict Audit Requirements

- Auditability requirements define a minimum retained conflict summary
- Conflict summaries may replace detailed payloads after retention
- Minimum audit record: conflict identifier, semantic field, resolver identity, resolution time, chosen outcome, authorization proof
- We may forget details, but we never forget decisions

### Retention Policy

- Retention policy is explicit, workspace-scoped, and auditable
- Changes to retention policy are recorded as operations
- Retention policy changes are subject to authorization

### Open

- UI presentation patterns
- Auto-resolution heuristics (never for critical fields?)
- Batch conflict tooling
- CRDT model for specific field types (rich text?)

---

## Proposals & Suggestions

Proposals are non-authoritative suggested changes that are visible to collaborators but do not alter canonical state until explicitly accepted. They are distinct from conflicts (which arise from concurrent edits) and from transforms (which are deterministic operations).

### Core Semantics

- Proposals do not modify canonical state
- Proposals are derived from explicit operations or transforms
- Proposals must be explicitly accepted to produce canonical operations
- Rejecting a proposal produces no canonical mutation
- Proposal acceptance produces explicit operations recorded in history
- Proposals are auditable and reference their origin

### Proposal Lifecycle

- Proposals are created by explicit user or plugin action
- Proposals may be reviewed by any user with read access
- Proposals may be accepted only by users authorized to write to affected fields
- Proposals may be rejected by the proposer or by authorized users
- Proposals may expire or be withdrawn without affecting canonical state

### Proposal Visibility

- Proposals are visible to all peers within the workspace
- Proposals sync like other operations but do not affect canonical state
- Proposals may be filtered or grouped in UI for review
- Multiple proposals may exist for the same field simultaneously

### Relationship to Other Concepts

- Proposals may be created from overlay changes (commit as proposal instead of direct commit)
- Proposals may be created from transform previews
- Proposals do not create conflicts; conflicts arise only from committed canonical operations

### Proposals and Overlays

- Proposals may be created from overlay state
- Proposals reference canonical operations that would result if accepted
- Accepting a proposal emits canonical operations, not overlay operations
- Discarding an overlay does not discard proposals created from it
- Proposals created from overlays become independent once created

### Proposals and Conflicts Independence

- Proposals and conflicts are independent derived states
- Proposals do not suppress, replace, or affect conflict derivation
- Proposals never affect conflict detection or resolution until accepted
- A field may have both an open conflict and pending proposals simultaneously
- Conflict resolution and proposal acceptance are orthogonal operations

### Proposal Acceptance Semantics

- Accepting a proposal is semantically equivalent to performing the proposed operation directly
- Proposal acceptance must not bypass conflict detection or resolution rules
- If the proposed field is not conflicted, proposal acceptance emits a normal edit operation
- If the proposed field is conflicted, proposal acceptance emits a resolve_conflict operation
- Proposal acceptance respects the same authorization rules as direct edits
- Accepting a proposal for a conflicted field resolves the conflict to the proposed value

### Use Cases

- Collaborative review workflows (designer proposes → SM approves)
- Safe cross-plugin suggestions
- Bulk change review before commit
- Non-destructive experimentation shared with team

### Open

- Proposal expiration policies
- Proposal notification/subscription model
- Proposal dependencies (accept A requires accepting B)

---

## Queries & Derived Views

- Queries are read-only
- Deterministic results given same state
- Binding-aware semantics (query "people" returns all bound facet types)
- No hidden mutations

### Parameterized Queries

- Parameterized queries are read-only and deterministic
- Parameterized queries must not emit operations
- Query parameters may reference canonical state or local overlay state
- Given the same canonical state and parameter values, query results are deterministic
- Parameterized queries must not implicitly depend on untracked external state
- Parameters enable dynamic selection (e.g., "all people in these scenes", "all cues related to this event")

### Derived Entity Sets

- Derived entity sets are read-only query results
- Derived entities do not accept direct mutation
- Derived sets may feel like "virtual entities" but are never writable
- Materializing derived entities into canonical entities requires explicit user action
- Materialization produces explicit operations recorded in history
- Canonical state must not contain back-references to derived sets
- Canonical entities must not reference derived entities by ID
- Derived sets exist only as query results; they have no persistent identity

### Query Determinism Under Overlay + Sync

- Queries are evaluated against consistent snapshots of state
- Overlay queries reflect overlay state layered atop canonical state
- Queries must not observe partially applied canonical operations
- Sync application is atomic from the perspective of query evaluation

### Open

- Query language syntax
- Performance caching
- Incremental materialization

---

## Plugins

- TOML schema declaration
- Plugins own their facets
- No direct state mutation; everything produces operations
- Capability request/gating (filesystem, network, MIDI/OSC, etc.)
- Capabilities are granted per-user and enforced by core

- TODO: Plugins specify human readable display field and optional semantic identifier key

### Local-Only Plugins

Local-only plugins produce data that exists only for one user and never syncs to canonical state.

- Local-only plugin data is excluded from canonical sync
- Local-only data may reference canonical entities
- Local-only plugins must not emit canonical operations
- Local-only facets are stored separately from canonical facets
- Local-only data follows the same operation/bundle model locally
- Local-only plugins function fully offline

Use cases:
- Personal notes and annotations
- Scratch data and working calculations
- Private workflows and experiments
- User-specific display preferences

### Jobs

- Jobs produce operation bundles, never mutate state directly
- Job output is staged until job completes successfully
- Failed jobs produce no ops
- Partial job output is discarded entirely
- Job failure surfaces error to user with context

### Structured Imports & Exports

Imports and exports are treated as deterministic, auditable, previewable jobs.

- Imports run inside staging overlays by default
- Imports produce explicit operations recorded in history only upon commit
- Imports must support dry-run/preview before committing
- Import preview is the overlay state before commit
- Exports operate on derived views (read-only)
- Import/export operations are auditable
- Import source metadata is preserved in operation attribution
- Failed imports produce no operations
- Discarding an import overlay discards all imported data without affecting canonical state

### Views

- Plugin view crashes do not corrupt state
- Crashed views show error boundary with retry option

### UI/UX Integration Elements

These elements do not affect correctness but are explicitly allowed and encouraged.

- Conflict and proposal indicators are derived UI state
- Badges and indicators must reflect underlying semantic state
- Plugins may embed conflict/proposal/overlay components contextually
- UI state (expanded/collapsed, scroll position, selection) is local-only and non-syncing
- Plugins may provide custom conflict resolution UI for their facet types
- Plugins may provide custom proposal review UI

### Open

- Distribution/marketplace
- Sandboxing model (WASM? process isolation?)
- Runtime view and job registry (avoid restarts)

---

## Replication & Sync

- Peer-to-peer LAN collaboration
- Avoid host-client model; goal is failover safety
- If leader disconnects, new leader is elected
- Supports network partitions (same workspace, isolated sessions can work and resync)
- Deterministic ordering after merge
- Conflict states representable and available offline

### Sync Behavior

- Sync is non-blocking; users may continue working while sync is in progress
- A user's local edits remain visible to them until sequenced or explicitly rejected
- Sync must not silently overwrite a user's unsequenced local edits
- Temporary divergence between local optimistic state and canonical state is acceptable during sync
- Sync progress is monotonic; peers only advance forward in acknowledged canonical sequence
- The system guarantees eventual convergence of all valid operations

### Sequencing Authority

- During network partitions, multiple independent sequencers may exist
- Sequencer authority is provisional until reconciliation
- No sequencer is globally authoritative until all partitions have merged
- Sequencing does not imply authorization or validity; all operations remain subject to independent peer validation

### Provisional State Visibility (Author)

- Local edits are applied optimistically and immediately visible to the author
- A peer must not block or delay local edits waiting for sequencing acknowledgment
- Local optimistic state persists across temporary disconnection
- Unsequenced local edits remain visible until either sequenced or rejected through conflict resolution
- The system may indicate provisional or unacknowledged state, but must not withhold, revert, or delay the edited value

### Provisional State Visibility (Other Peers)

- A peer must not observe another peer's unsequenced optimistic edits
- A peer sees another peer's edit only once the operation is sequenced and broadcast

### State Presentation

- Primary views present current semantic state, not operation history
- Users are not required to reason about individual operations to continue work
- All operations remain individually inspectable in history/audit views
- History presentation may group operations meaningfully without violating deterministic ordering

### Sync Reliability

- Sync progress is checkpointed periodically
- Interrupted sync resumes from last checkpoint
- No ops are applied until checkpoint is complete
- Partial sync never leaves peer in inconsistent state
- Sync rejects corrupt ops from peers
- System attempts to recover corrupt ops from other peers before quarantining

### Open

- Peer discovery mechanism (mDNS?)
- Leader election algorithm
- CRDT for rich text fields
- Transport protocol
- Cloud sync (future)

---

## Trust & Validation Model

### Peer Trust

- No peer is inherently trusted
- No peer, including an elected leader, is authoritative for truth or authorization
- All peers independently validate all operations they apply

### Cryptographic Integrity

- Every operation is cryptographically signed by its author
- An operation without a valid signature is invalid and must not be applied
- Operation payloads are immutable once signed
- Operation history is hash-linked such that insertion, deletion, or reordering is detectable
- History modification is always detectable by validating peers
- A peer must not apply operations if the history chain is invalid

### Authorization

- Permissions and roles are expressed exclusively through operations
- Role assignments and revocations are immutable once recorded; changes are expressed as new operations
- Authorization is evaluated locally by each peer using its known history
- Authorization decisions are based on the set of operations known to the validating peer
- A peer must not reject an operation solely due to missing historical context
- Operations that depend on unseen history may be accepted provisionally
- Final authorization validity is determined once all relevant history is integrated

### Schema-Scoped Permissions

Permissions may be scoped not just to entities, but to Concepts, fields, transforms, and proposals.

- Permissions may be scoped to Concepts, fields, and operation types
- Proposing changes may require fewer permissions than applying them directly
- Permission changes are explicit operations and are auditable
- Authorization for conflict resolution equals authorization for field write
- Transform execution may be gated by permissions distinct from direct field edit
- Read permissions and write permissions are independently assignable

Use cases:
- Designers propose changes → Stage Managers approve and commit
- Field-level restrictions (e.g., only SM can edit cue numbers)
- Concept-level restrictions (e.g., only certain roles can create Person entities)

### Leader Role

- An elected leader exists solely to coordinate ordering of operations
- A leader does not authorize, validate, or approve operations
- A leader may be replaced at any time without invalidating history

### Peer Validation

- Each peer independently validates every operation before applying it
- A peer must reject operations that violate schema, signature, or authorization rules
- A peer must not accept operations solely because they were sequenced by a leader
- A peer must not assume completeness of history without explicit evidence
- Missing history is treated as uncertainty, not invalidity
- Peers track known gaps in history explicitly

### Offline & Reconciliation

- Peers may perform valid operations while offline
- Offline operations must be sequenced and reconciled upon reconnection
- Reconciliation is additive; valid operations are never silently discarded

### Guarantees

- Given the same set of valid operations, all peers will converge to the same state
- Malicious peers may delay progress but cannot corrupt valid state
- Invalid or unauthorized operations are rejected or quarantined
- Corruption is detectable and auditable
- All operations, including permission changes, are permanently auditable
- Historical operations remain inspectable even if superseded
- No valid operation is ever silently removed

### Non-Guarantees

- The system does not guarantee immediate global consistency
- The system does not guarantee awareness of all operations at any moment
- The system does not attempt to detect network partitions explicitly

---

## Failure & Recovery

### Crash Safety

- Partial writes never corrupt oplog
- Incomplete bundles are discarded on crash recovery
- Database is consistent after WAL replay

### Corruption Handling

- Corrupt ops are detected via checksum on read/sync
- Corrupt ops are quarantined, not applied
- Quarantined ops are logged for manual review
- System attempts recovery from peers before quarantine

### Recovery Tooling

- System provides oplog inspection tools (view history, search ops)
- System provides conflict history review (see past resolutions)
- System provides quarantine review (see rejected/corrupt ops)
- System provides "export current state" for emergency backup
- Emergency export is always available

### User Experience

- Failures surface human-readable explanations
- Recovery actions are explicit user choices, not automatic

---

## Summary: Confidence Levels

| Decision                                                        | Status  | Confidence |
| --------------------------------------------------------------- | ------- | ---------- |
| Field-level ops with bundle grouping                            | Decided | High       |
| Blob immutability (content-addressed)                           | Decided | High       |
| Blob GC with retention window                                   | Decided | Medium     |
| Blob deletes as ops                                             | Decided | High       |
| Blobs not required for replay                                   | Decided | High       |
| Workspace isolation                                             | Decided | High       |
| Templates for new workspaces                                    | Decided | High       |
| Personal libraries separate                                     | Decided | Medium     |
| SQLite WAL crash safety                                         | Decided | High       |
| Checkpoint-based sync resume                                    | Decided | Medium     |
| Checksum + quarantine for corruption                            | Decided | High       |
| Jobs staged until complete                                      | Decided | High       |
| Bundle-level entity create/delete                               | Decided | High       |
| Field-level conflict granularity                                | Decided | High       |
| Sync is non-blocking                                            | Decided | High       |
| Optimistic local edits visible immediately                      | Decided | High       |
| Peers cannot see others' unsequenced edits                      | Decided | High       |
| Conflicts surfaced after stable merge point                     | Decided | High       |
| Sequencing authority is provisional                             | Decided | High       |
| Canonical history ordering deterministic                        | Decided | High       |
| Wall-clock timestamps untrusted                                 | Decided | High       |
| No peer inherently trusted                                      | Decided | High       |
| Operations cryptographically signed                             | Decided | High       |
| Hash-linked history chain                                       | Decided | High       |
| Authorization evaluated locally                                 | Decided | High       |
| Leader role is coordination only                                | Decided | High       |
| Eventual convergence guaranteed                                 | Decided | High       |
| Resolution as new operation in history                          | Decided | High       |
| Resolution references conflict explicitly                       | Decided | High       |
| Resolution immutable once recorded                              | Decided | High       |
| Resolution revision distinct from normal edit                   | Decided | High       |
| Late-arriving edits reopen conflicts                            | Decided | High       |
| Conflicts at Concept level when bound                           | Decided | High       |
| Resolution authority = field write permission                   | Decided | High       |
| At most one resolution per conflict state                       | Decided | High       |
| Resolution stability (no oscillation)                           | Decided | High       |
| Conflict GC preserves resolution ops                            | Decided | High       |
| Conflict GC preserves summary/audit                             | Decided | High       |
| Compaction produces new ops, not rewrites                       | Decided | High       |
| Partial history does not invalidate resolution                  | Decided | High       |
| Retention policy is auditable                                   | Decided | High       |
| Three-layer Concept model (definitions, bindings, entities)     | Decided | High       |
| Concept definitions are schema-level, explicit                  | Decided | High       |
| Bindings declare semantic compatibility only                    | Decided | High       |
| Concept entities created only by explicit equivalence assertion | Decided | High       |
| Facet binding prerequisite for entity equivalence               | Decided | High       |
| Canonical fields start conflicted when values differ            | Decided | High       |
| No implicit Concept entity creation                             | Decided | High       |
| Concept queries return Concept entities only                    | Decided | High       |
| Plugins function fully without Concepts                         | Decided | High       |
| Unbound entities editable without Concept                       | Decided | High       |
| Binding-time conflict detection                                 | Decided | High       |
| New bindings create new conflicts (not reopen old)              | Decided | High       |
| Equivalence assertion is atomic single operation                | Decided | High       |
| Single-entity Concept membership valid                          | Decided | High       |
| Plugin → Concept reference direction                            | Decided | High       |
| Field-level Concept conflicts (not entity-level)                | Decided | High       |
| Null values do not create conflicts                             | Decided | High       |
| Bound fields project canonical value after resolution           | Decided | High       |
| Edits to bound fields update Concept directly                   | Decided | High       |
| Sequential edits do not create conflicts                        | Decided | High       |
| Unbinding retains last concrete values                          | Decided | High       |
| Concept entities persist regardless of binding count            | Decided | High       |
| Post-unbinding edits are plugin-local only                      | Decided | High       |
| Rebinding allowed; triggers conflict detection                  | Decided | High       |
| Rebinding after divergence creates per-field conflicts          | Decided | High       |
| Unbound-period history fully preserved                          | Decided | High       |
| Single Concept unifies asymmetric plugin participation          | Decided | High       |
| Subset via binding participation, not hierarchy                 | Decided | High       |
| Concept queries return Concept entities only                    | Decided | High       |
| Plugin deletion does not delete Concept entity                  | Decided | High       |
| Core guarantees correctness; plugins guarantee usability        | Decided | High       |
| Projection is semantic, not mechanical copying                  | Decided | High       |
| Facet attachment ≠ entity equivalence                           | Decided | High       |
| Rule-based binding follows manual semantics                     | Decided | High       |
| Assisted alignment requires explicit confirmation               | Decided | High       |
| Incomplete data is valid state                                  | Decided | High       |
| Identity mistakes corrected by unbinding                        | Decided | High       |
| Unbinding removes conflicts (not resolves them)                 | Decided | High       |
| Post-mistake canonical state persists until explicit change     | Decided | High       |
| Full audit trail for mistakes and recoveries                    | Decided | High       |
| Parameterized queries are read-only and deterministic           | Decided | High       |
| Query parameters may reference overlay state                    | Decided | High       |
| Derived entity sets are read-only                               | Decided | High       |
| Materializing derived entities requires explicit action         | Decided | High       |
| Staging overlays isolated from canonical state                  | Decided | High       |
| Overlay commit produces explicit operations                     | Decided | High       |
| Overlay discard affects no canonical state                      | Decided | High       |
| Proposals do not modify canonical state                         | Decided | High       |
| Proposal acceptance produces explicit operations                | Decided | High       |
| Proposal rejection produces no mutation                         | Decided | High       |
| Local field overrides do not modify canonical fields            | Decided | High       |
| Overrides are auditable and visibly marked                      | Decided | High       |
| Removing override restores canonical projection                 | Decided | High       |
| Schema-scoped permissions (Concepts, fields, operations)        | Decided | High       |
| Proposing may require fewer permissions than applying           | Decided | High       |
| Local-only plugins excluded from canonical sync                 | Decided | High       |
| Local-only plugins may reference canonical entities             | Decided | High       |
| Imports produce explicit operations                             | Decided | High       |
| Imports must support dry-run/preview                            | Decided | High       |
| Bulk operations represented as bundles                          | Decided | High       |
| Bulk operations must be previewable                             | Decided | High       |
| UI indicators reflect underlying semantic state                 | Decided | High       |
| Transform Bindings are explicit user-declared rules             | Decided | High       |
| Transform Bindings must be deterministic                        | Decided | High       |
| Transform Bindings execute on source field change               | Decided | High       |
| Transform Bindings must not self-trigger or cycle               | Decided | High       |
| Transform Binding outputs are explicit operations               | Decided | High       |
| Transform Bindings subject to conflict detection                | Decided | High       |
| Transform Bindings must not auto-resolve conflicts              | Decided | High       |
| Transform Bindings require write permission                     | Decided | High       |
| Transform Bindings visible in history/audit                     | Decided | High       |
| Transform Bindings must not infer intent or merge entities      | Decided | High       |
| Transform Bindings do not suppress concurrent outputs           | Decided | High       |
| Concurrent differing transform outputs create conflicts         | Decided | High       |
| Transform Binding disable does not revert prior operations      | Decided | High       |
| Transform Binding lifecycle changes are auditable               | Decided | High       |
| Proposals may be created from overlay state                     | Decided | High       |
| Accepting proposal emits canonical ops, not overlay ops         | Decided | High       |
| Discarding overlay does not discard proposals from it           | Decided | High       |
| Concept entity deletion does not rewrite history                | Decided | High       |
| Historical refs to deleted Concepts remain resolvable           | Decided | High       |
| Queries evaluated against consistent state snapshots            | Decided | High       |
| Queries must not observe partial canonical operations           | Decided | High       |
| Sync application atomic from query perspective                  | Decided | High       |
| P2P replication specifics                                       | Open    | Medium     |
| Plugin sandboxing                                               | Open    | Low        |
| Overlay persistence across sessions                             | Open    | Medium     |
| Overlay sharing between peers                                   | Open    | Low        |
| Proposal expiration policies                                    | Open    | Medium     |
| Transform Binding dependency ordering                           | Open    | Medium     |
| Transform Binding versioning and migration                      | Open    | Medium     |
