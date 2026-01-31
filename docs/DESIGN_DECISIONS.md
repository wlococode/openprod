# Design Decisions: Open Questions

This document explores unresolved architectural questions that need decisions before implementation.

---

## 1. Operation Granularity & Scope

**Question:** What is the atomic unit of change?

### The Tension

- **Fine-grained** (one field = one op): Precise conflicts, but more storage, slower replay
- **Coarse-grained** (one save = one op): Fewer ops, but conflicts are all-or-nothing

### Options

#### Option A: Pure Field-Level Operations

Every field mutation is a separate operation.

```
User edits contact:
  → Op 1: SET contacts.person.abc123.name = "Jane Doe"
  → Op 2: SET contacts.person.abc123.email = "jane@example.com"
  → Op 3: SET contacts.person.abc123.phone = "555-1234"
```

**Pros:**
- Maximum conflict precision (Alex edits name, Jordan edits email = no conflict)
- Simple mental model: one field, one op
- Replay is straightforward

**Cons:**
- High op volume (editing a contact with 10 fields = 10 ops)
- Jobs generating call times for 50 people = hundreds of ops
- UI needs to batch for undo/redo (user expects "undo" to undo the whole edit, not one field)

#### Option B: Logical Operation Bundles

User actions become atomic bundles. Fields are still tracked individually, but the bundle is the conflict unit.

```
User edits contact:
  → Bundle "update_contact_abc123":
      - SET name = "Jane Doe"
      - SET email = "jane@example.com"
      - SET phone = "555-1234"
```

**Pros:**
- Undo/redo works naturally (undo the bundle)
- Conflict detection can be bundle-level OR field-level (configurable)
- Jobs produce one bundle, not thousands of loose ops

**Cons:**
- More complex op structure
- Need to decide: does bundle conflict if ANY field overlaps, or only if SAME field overlaps?

#### Option C: Entity-Level Operations

Each entity mutation is one op, regardless of how many fields changed.

```
User edits contact:
  → Op: UPDATE contacts.person.abc123 { name: "Jane Doe", email: "...", phone: "..." }
```

**Pros:**
- Simple
- Fewer ops
- Natural undo unit

**Cons:**
- Coarse conflicts: Alex changes name, Jordan changes email on same contact = conflict (even though they edited different fields)
- Loses precision for partially-overlapping edits

#### Option D: Hybrid - Field Storage, Bundle Presentation

Store ops at field level. Group into bundles for conflict presentation and undo.

```
Storage:
  → Op 1: SET name (bundle: edit_abc123_t1234)
  → Op 2: SET email (bundle: edit_abc123_t1234)
  → Op 3: SET phone (bundle: edit_abc123_t1234)

Conflict detection: field-level
Conflict presentation: "Contact 'Jane Doe' was edited by Alex and Jordan"
Undo: reverts entire bundle
```

**Pros:**
- Best of both worlds: precise detection, intuitive presentation
- Flexible for different UI needs

**Cons:**
- More implementation complexity
- Need bundle metadata in every op

### Recommendation

**Option D (Hybrid)** seems right for your use cases:

- Stage manager edits schedule event with 5 fields → one undo action, but if only one field conflicts, show that
- Job calculates 50 call times → one bundle, atomic commit, single undo
- Lighting designer changes cue timing → small bundle, precise conflict if another LD touched same cue

### Suggested Invariants

```
- Operations are stored at field granularity
- Operations are grouped into bundles for atomicity
- A bundle either fully commits or fully fails
- Conflict detection operates at field level (after binding resolution)
- Conflict presentation groups by bundle for user clarity
- Undo/redo operates on bundles, not individual ops
- Jobs produce exactly one bundle per execution
- Bundle size is unbounded but should be "reasonable" (warn if > 1000 ops?)
```

### Open Sub-Questions

- Is a bundle the smallest revert unit, or can users cherry-pick fields?
- Should bundles have types (user_edit, job_output, import, merge_resolution)?
- Max bundle size? Or just advisory warnings?

---

## 2. Asset/Blob Lifecycle

**Question:** How do binary assets (logos, PDFs, scripts) behave?

### Context from Your Use Cases

Assets include:
- Show logos (small, rarely change)
- Script versions (medium, versioned over time)
- Exported paperwork (generated, possibly regenerated)
- NOT large CAD files or video

### Core Questions & Proposed Answers

#### Are blobs immutable once written?

**Recommendation: Yes.**

Content-addressed storage (hash-based) means the hash IS the identity. If you change the content, you get a new hash, which is a new blob. The old blob still exists.

```
Invariant: Blobs are immutable. Modifying content creates a new blob with a new hash.
```

#### Can blobs be garbage collected?

**Recommendation: Yes, with caution.**

If history is never deleted, old ops may reference old blobs. Options:

A) **Never GC**: All blobs live forever. Simple but storage grows unbounded.

B) **GC unreferenced blobs**: If no op references a blob, delete it. But what about checkpoints/snapshots that skip old ops?

C) **Tiered retention**: Keep blobs referenced by recent ops (e.g., last 90 days). Archive or delete older ones. Ops remain but blob retrieval fails gracefully.

D) **Explicit archive**: Blobs can be marked "archived" (removed from local, retrievable from cold storage or peers).

**Recommendation: Option C or D.** Production shows have finite lifespans. A show from 5 years ago probably doesn't need instant blob access, but the oplog history is still valuable.

```
Invariant: Blobs may be garbage collected if unreferenced by "active" ops.
Invariant: GC never deletes blobs referenced by ops within retention window.
Invariant: Ops referencing GC'd blobs remain valid; blob retrieval returns "unavailable."
```

#### Are blob deletes operations?

**Recommendation: Yes.**

Deleting a logo or removing a script version should be auditable.

```
Op: DELETE_ASSET { hash: "abc123", reason: "replaced with updated version" }
```

The blob data may be GC'd later, but the DELETE_ASSET op stays in history.

```
Invariant: Asset deletions are recorded as operations.
Invariant: Deleting an asset does not delete the blob immediately (GC handles cleanup).
```

#### What if two peers add different blobs with the same hash?

**This cannot happen with content-addressed storage.** Same hash = same content (assuming no hash collision, which is astronomically unlikely with SHA-256).

If two peers independently add the same logo, they produce the same hash, and only one copy is stored. No conflict.

```
Invariant: Blobs with identical content produce identical hashes.
Invariant: Identical hashes are deduplicated automatically (no conflict).
```

#### Are assets required for oplog replay?

**Recommendation: No, but with caveats.**

State reconstruction (entity fields, relationships) should not require blob data. But full audit/display may need blobs.

```
Invariant: Oplog replay reconstructs entity state without requiring blob data.
Invariant: Ops that reference assets store metadata (hash, filename, size) inline.
Invariant: Blob absence is a retrieval failure, not a state corruption.
```

#### Compression?

**Recommendation: Yes, for storage and sync.**

- Store blobs compressed on disk (zstd or similar)
- Sync transfers compressed
- Decompress on read

```
Invariant: Blobs are stored compressed. Compression is transparent to plugins.
```

### Suggested Invariants (Summary)

```
- Blobs are immutable and content-addressed (hash = identity)
- Modifying content creates a new blob
- Blob deletions are recorded as operations
- Blobs may be GC'd after retention window if unreferenced
- Ops referencing GC'd blobs remain valid; retrieval returns "unavailable"
- Identical content = identical hash = automatic deduplication
- Oplog replay does not require blob data for state reconstruction
- Blobs are stored and synced compressed
```

### Open Sub-Questions

- Retention window: 90 days? Configurable per workspace?
- Cold storage integration for archived blobs?
- Should plugins declare asset types (logo, script, export) with different retention rules?

---

## 3. Workspace Model

**Question:** What is a workspace, and how do workspaces relate to each other?

### Current Understanding

A workspace = one show/project:
- One oplog namespace
- One set of entities
- One collaborative scope

### Proposed Core Invariants

```
- Workspace = isolated oplog namespace with unique ID
- Entities belong to exactly one workspace
- Entity IDs are unique within a workspace (not globally)
- Sync only occurs between peers with the same workspace ID
- Different workspaces never leak or mutate each other's data
- Actor IDs (user/device identities) may operate across multiple workspaces
```

### The Hard Questions

#### Forking: Same tour, multiple venues

**Scenario:** A tour plays 20 cities. Each venue has different rigging, different local crew, but the same core show (cues, principal cast, design).

**Options:**

**A) Separate workspaces, manual duplication**
- Each venue is its own workspace
- Copy contacts, cue sheets manually (or via export/import)
- No automatic sync between venues

*Pros:* Simple, clear isolation
*Cons:* Tedious, drift between venues is invisible

**B) Workspace templates**
- Create a "tour master" template
- Instantiate new workspaces from template
- No ongoing link; changes don't propagate

*Pros:* Good starting point, still simple
*Cons:* Still no sync after creation

**C) Linked workspaces (read-only references)**
- Venue workspace can reference entities from "tour master" workspace
- References are read-only; edits create local copies
- Changes to master can be "pulled" into venue workspaces

*Pros:* Updates propagate (when user chooses)
*Cons:* Complex. What if master is offline? What if reference breaks?

**D) Workspace hierarchy (parent/child)**
- Tour master is parent
- Venue workspaces are children
- Children inherit from parent but can override

*Pros:* Elegant for this use case
*Cons:* Significant complexity. Inheritance semantics are hard.

**Recommendation: Option B (Templates) for v1.**

Templates solve 80% of the use case with 20% of the complexity. Users can:
1. Build a "tour master" workspace
2. Export as template
3. Create venue workspace from template
4. Customize venue-specific data

Cross-workspace sync can be a future feature if there's demand.

```
Invariant: Workspaces are fully isolated by default.
Invariant: Templates are point-in-time snapshots used to initialize new workspaces.
Invariant: No automatic cross-workspace sync in v1.
```

#### Cross-workspace references (contacts, fixtures)

**Scenario:** A lighting designer works on multiple shows and wants to reuse their fixture library or personal contacts.

**Options:**

**A) No cross-workspace data**
- Copy what you need into each workspace
- Contacts are duplicated, not linked

**B) User-level data separate from workspace data**
- "My Contacts" and "My Fixtures" live in a personal space
- Workspaces can import from personal space (copy, not link)

**C) Explicit linking**
- Entity in Workspace A can reference entity in Workspace B
- Complex sync and offline implications

**Recommendation: Option B for v1.**

Personal data (your contacts, your fixture library) is separate from show data. When you start a new show, you import from your personal library. The import creates copies, not live links.

```
Invariant: Workspace data is isolated from personal/library data.
Invariant: Imports from personal library create copies, not references.
Invariant: Personal library syncs independently from workspace sync.
```

#### Cloning vs. Forking

**Cloning:** Exact copy of workspace at point in time. New workspace ID, no history, just current state.

**Forking:** Copy with history. New workspace ID, but oplog is duplicated.

**Recommendation:** Support cloning (current state only). Forking adds complexity and bloats storage.

```
Invariant: Clone creates new workspace with current state, no history.
Invariant: Forking (with history) is not supported in v1.
```

### Suggested Invariants (Summary)

```
- Workspace = isolated namespace with unique ID
- Entities belong to exactly one workspace
- Entity IDs are workspace-scoped, not global
- Sync only occurs within same workspace ID
- Workspaces never leak data to each other
- Actor IDs may span multiple workspaces
- Templates are snapshots for initializing new workspaces
- Cloning creates new workspace from current state (no history)
- Personal libraries are separate from workspace data
- Imports from libraries create copies, not references
- Cross-workspace references not supported in v1
```

### Open Sub-Questions

- Template format? Full oplog snapshot or just entity state?
- Personal library sync: same mechanism as workspace sync, or different?
- Can a workspace be "archived" (read-only, no more edits)?

---

## 4. Failure & Recovery

**Question:** What happens when things go wrong?

### Failure Scenarios

#### Crash during write

SQLite WAL mode helps here. Writes are atomic at the database level.

```
Invariant: Partial writes never corrupt the oplog.
Invariant: On crash recovery, database is consistent (WAL replay).
Invariant: Incomplete bundles are discarded on recovery (never partially committed).
```

#### Partial sync

Sync transfers ops in batches. What if connection drops mid-batch?

**Options:**

**A) All-or-nothing per sync session**
- Sync completes fully or rolls back
- Simple but may waste bandwidth on retry

**B) Checkpoint-based**
- Sync commits in checkpoints (e.g., every 100 ops)
- Resume from last checkpoint on retry

**C) Op-level acknowledgment**
- Each op is acked individually
- Resume from last acked op

**Recommendation: Option B (Checkpoints).**

All-or-nothing is wasteful for large syncs. Op-level is complex. Checkpoints balance reliability and efficiency.

```
Invariant: Sync progress is checkpointed periodically.
Invariant: Interrupted sync resumes from last checkpoint.
Invariant: No ops are applied until checkpoint is complete.
Invariant: Partial sync never leaves peer in inconsistent state.
```

#### Corrupt oplog entry

Ops should be checksummed. What if checksum fails?

```
Invariant: Every op has a checksum (hash of content).
Invariant: Corrupt ops are detected on read/sync.
Invariant: Corrupt ops are quarantined, not applied.
Invariant: Quarantined ops are logged for manual review.
Invariant: Sync rejects corrupt ops from peers.
```

**Recovery options:**

1. **Request op from another peer** — If multiple peers have the op, fetch a valid copy
2. **Skip and log** — If op is unrecoverable, skip it but log the gap
3. **Manual repair** — User reviews quarantine and decides

```
Invariant: System attempts to recover corrupt ops from other peers before quarantining.
```

#### Plugin crash during job

A plugin job runs, produces 500 ops, then crashes before completing.

```
Invariant: Job output is staged, not committed, until job completes successfully.
Invariant: Failed jobs produce no ops.
Invariant: Job failure surfaces error to user with context.
Invariant: Partial job output is discarded entirely (no half-applied batches).
```

#### Plugin crash during view render

Less critical—view is just display. But should be graceful.

```
Invariant: Plugin view crashes do not corrupt state.
Invariant: Crashed views show error boundary with retry option.
```

### Recovery Tooling

Users need ways to recover from problems:

```
Invariant: System provides oplog inspection tools (view history, search ops).
Invariant: System provides conflict history review (see past resolutions).
Invariant: System provides quarantine review (see rejected/corrupt ops).
Invariant: System provides "export current state" for emergency backup.
```

### User-Facing Recovery UX

When things go wrong, users should see:
- **What happened** (in plain language)
- **What was affected** (which entities, which ops)
- **What they can do** (retry, skip, contact support)

```
Invariant: Failures surface human-readable explanations.
Invariant: Recovery actions are explicit user choices, not automatic.
```

### Suggested Invariants (Summary)

```
Writes:
- Partial writes never corrupt oplog (SQLite WAL guarantees)
- Incomplete bundles are discarded on crash recovery

Sync:
- Sync progress is checkpointed
- Interrupted sync resumes from checkpoint
- No ops applied until checkpoint complete
- Partial sync never corrupts local state

Corruption:
- Every op has a checksum
- Corrupt ops detected and quarantined
- System attempts recovery from peers before quarantine
- Quarantined ops logged for review

Jobs:
- Job output staged until completion
- Failed jobs produce no ops
- Partial job output discarded entirely

General:
- Plugin crashes don't corrupt state
- Failures surface human-readable messages
- Recovery is explicit user choice
- Inspection tools available (oplog, conflicts, quarantine)
- Emergency export always available
```

---

## Summary: Decisions Needed

| Area | Recommendation | Confidence |
|------|----------------|------------|
| Op granularity | Hybrid: field-level storage, bundle presentation | High |
| Blob immutability | Yes, content-addressed | High |
| Blob GC | Tiered retention with "unavailable" fallback | Medium |
| Blob deletes as ops | Yes | High |
| Blobs required for replay | No (metadata inline, blob optional) | High |
| Workspace isolation | Full isolation, no cross-workspace refs v1 | High |
| Templates | Point-in-time snapshots for new workspaces | High |
| Personal libraries | Separate from workspace, import copies | Medium |
| Crash recovery | SQLite WAL, discard incomplete bundles | High |
| Sync interruption | Checkpoint-based resume | Medium |
| Corrupt ops | Checksum, quarantine, attempt peer recovery | High |
| Job failures | Stage until complete, discard on fail | High |

---

## Next Steps

1. Review these recommendations
2. Flag disagreements or alternative preferences
3. Decide on open sub-questions
4. Move finalized invariants to INVARIANTS.md
