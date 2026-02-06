# Staging Overlays Specification

This document defines staging overlays, the transport router, and canonical drift handling.

---

## Staging Overlays

Staging overlays are temporary, non-canonical layers of operations that enable safe experimentation and preview.

**Anchor invariant:** Overlays answer "Show me what this will do before it becomes real." All overlay operations are local-only until explicitly committed.

### Core Semantics

- Overlays are isolated from canonical state
- Overlay operations do not affect canonical history until explicitly committed
- Queries and conflicts operate identically within overlays
- Overlays may be discarded without affecting canonical state or history
- Committing an overlay produces explicit operations added to canonical history
- Commit is atomic: overlay either fully commits or fully fails

---

## Transport Router

All write operations pass through a transport router that determines destination:

```
Write Operation
      |
      v
+-----------------+
| Transport Router|
+--------+--------+
         |
    +----+----+
    | Active  |
    |Overlay? |
    +----+----+
         |
   Yes --+-- No
    |        |
    v        v
+--------+ +------------------+
|Overlay | |Oplog + Broadcast |
|Storage | |   (Canonical)    |
+--------+ +------------------+
```

- Default: operations go to canonical oplog and broadcast to peers
- When overlay active: operations route to the active overlay's local storage
- Scripts write to their own overlay (not the active user overlay)

---

## Overlay Structure

| Property | Description |
|----------|-------------|
| `id` | Unique identifier (UUID) |
| `displayName` | Human-readable name (auto-generated or user-provided) |
| `source` | Origin type: `user` or `script` |
| `sourceId` | For scripts: script execution ID; for user: actor ID |
| `createdAt` | Timestamp of creation |
| `operations` | Ordered list of operations in this overlay |
| `status` | `active` \| `stashed` \| `committed` \| `discarded` |

---

## Overlay Registry

Each client maintains an overlay registry:

```
 OverlayRegistry
+-- active: Overlay | null     (at most one)
+-- stashed: Overlay[]         (user overlays not currently active)
+-- pending: Overlay[]         (script overlays awaiting review)
```

- Only one overlay may be active at a time
- Multiple user overlays may exist; inactive ones are stashed
- Script overlays are independent and stored in pending until reviewed

---

## User Overlay Semantics

- Users may create multiple overlays
- Only one user overlay is active at a time
- Switching overlays auto-stashes the current overlay
- Stashed overlays can be recalled and reactivated
- User explicitly commits or discards each overlay

---

## Script Overlay Semantics

- Each script execution creates its own isolated overlay (unless using autoCommit mode)
- Script overlays are independent of user overlays
- Script overlays appear in the pending list when the script completes
- Scripts cannot write to the user's active overlay

---

## Multiple Overlays and Isolation

**Anchor invariant:** Overlays are mutually isolated. A user overlay cannot see script overlay changes, and vice versa. Interaction happens only through canonical state.

### Isolation Model

```
+-------------------------------------------------------------+
|  User Overlay (active)                                      |
|  - Sees: canonical + own uncommitted changes                |
|  - Cannot see: script overlay changes                       |
+-------------------------------------------------------------+

+-------------------------------------------------------------+
|  Script Overlay (pending)                                   |
|  - Sees: canonical at script start + own changes            |
|  - Cannot see: user overlay changes                         |
+-------------------------------------------------------------+

+-------------------------------------------------------------+
|  Canonical State                                            |
|  - Source of truth                                          |
|  - Updated when any overlay commits                         |
+-------------------------------------------------------------+
```

### When Overlays Modify the Same Field

No special coordination between overlays is needed. Existing systems handle interaction:

**Path A: User commits first**
1. User commits overlay -> field updated in canonical
2. User activates script overlay for review
3. Canonical drift detected: "Canonical changed while script was working"
4. User chooses: **Keep Mine** (keep script's value) or **Use Canonical** (discard script's delta for that field)

**Path B: Script reviewed first**
1. User stashes their overlay, activates script overlay
2. User commits script overlay -> field updated in canonical
3. User reactivates their overlay
4. Canonical drift detected on their overlay
5. User chooses: **Keep Mine** (keep their value) or **Use Canonical** (discard their delta for that field)

### No Cross-Overlay Awareness

- Overlays do not track what other overlays have modified
- Each overlay only knows about canonical state (at creation time) and its own changes
- Conflicts/drift are detected at commit time against current canonical state
- This keeps the model simple and predictable

---

## Script Completion Behavior

Script completion behavior is configurable (global default + per-script override):

| Tier | Behavior |
|------|----------|
| `notify` | Add to pending, send notification; user clicks to view |
| `surface` | Add to pending, send notification, auto-show overlay panel |
| `autoCommit` | Commit overlay immediately to canonical |

**Default:** `notify` (safest option)

When a script completes while user has an active overlay:
- Script overlay added to pending registry
- Notification shown with action: "Stash current and view"
- User's active overlay is not interrupted
- User can switch to script overlay when ready

---

## Overlay Activation

- Activating an overlay auto-stashes the currently active overlay (if any)
- No confirmation prompt required (stashing is non-destructive)
- UI clearly shows which overlay is active and which are stashed

---

## Display Priority

When an overlay is active, values are displayed in priority order:

```
Display Value = overlay.value ?? canonical.value
```

Overlay always wins visually. This creates a two-layer priority:

1. **Overlay** (highest) -- local staging changes
2. **Canonical** (lowest) -- committed truth

---

## Canonical Drift Handling

Overlays represent **deltas (changes only)**, not full snapshots of entity state. The overlay only tracks fields the user has explicitly changed. Canonical state may continue evolving underneath an active overlay; non-conflicting canonical changes are irrelevant to the overlay -- they do not affect overlay state and require no action.

When canonical state changes a field the overlay has **not** modified, nothing happens. The field simply shows the current canonical value as usual.

When canonical state changes a field the overlay **has** modified, the system warns the user:

**Detection:**
- System tracks canonical value at time of overlay operation
- When sync updates canonical state, drift is detected only for fields the user edited that also changed canonically
- Drift badges only appear on fields the user modified AND that changed canonically
- Fields the user didn't edit show current canonical value without badges

**Display:**
- Overlay value still displays (overlay wins)
- Badge indicates: "Canonical changed to X while you were editing"
- Badge appears on hover or as subtle indicator
- Multiple drifted fields: summary notification with granular control

**User Actions:**

| Action | Effect | Causal Result |
|--------|--------|---------------|
| **Keep Mine** | Acknowledge drift, keep overlay delta | User's op is causally after the drift (no conflict on commit) |
| **Use Canonical** (Knockout) | Discard overlay delta for that field, accept canonical value | No conflict, uses canonical value |

The user must choose one of these two options for each drifted field before committing.

**Why "Keep Mine" doesn't create conflicts:**
- User explicitly acknowledged seeing the canonical change
- Their overlay now has causal knowledge of that change (vector clock updated)
- Their commit is causally after the change, not concurrent

Knockout removes the overlay operation for that field without discarding the entire overlay.

---

## Commit Semantics

**Commit is atomic:**
1. All operations in overlay applied to canonical oplog atomically
2. Conflicts detected against current canonical state
3. If conflicts found, user resolves before commit completes
4. Committed operations broadcast to peers
5. Overlay is deleted after successful commit

To exclude specific operations, use Knockout before committing.

---

## Conflicts in Overlay Context

**Anchor invariant:** Committing an overlay that touches a conflicted field resolves the conflict to the overlay's value--same behavior as direct edits.

**Scenario:** While user is in overlay, a conflict is created in canonical (two peers edited same field while disconnected).

**Behavior:**
- Canonical drift detection surfaces the conflict: "This field now has a conflict in canonical"
- User can:
  - **Continue with overlay value** -- On commit, overlay value becomes the resolution
  - **Knockout** -- Accept canonical's conflicted state, remove overlay operation
- Committing with overlay value resolves the conflict as if user entered a new value
- Resolution is auditable: notes that overlay commit resolved the conflict

---

## Discard Semantics

- Discarding an overlay removes it and all its operations
- No effect on canonical state
- Discarding is immediate and permanent (no undo)

---

## Atomic Overlay Actions

**Anchor invariant:** Overlays are intentionally simple. You either commit all or discard all.

| Action | Effect |
|--------|--------|
| **Commit** | All overlay operations become canonical; overlay deleted |
| **Discard** | All overlay operations deleted; no canonical effect |

To exclude specific changes before commit, use Knockout (see below).

---

## Knockout Operations

Users can remove individual operations from an overlay before commit:

| Action | Effect |
|--------|--------|
| **Knockout field** | Remove operation affecting that field from overlay; field returns to canonical value |
| **Knockout (drift)** | Accept canonical value that changed during overlay; remove overlay operation |

Knockout enables selective exclusion before atomic commit. After knockout, the field shows canonical value and is no longer part of the overlay.

UI surfaces knockout via right-click or hover actions on overlay-modified fields.

---

## Storage

- Overlays stored in local SQLite (separate from canonical oplog)
- Overlays are local-only; they never sync to peers
- Overlays persist across app restarts
- No automatic expiration (user explicitly commits or discards)
- No auto-resume on startup; user manually reactivates overlays

---

## Startup Behavior

- On app startup, overlays are not automatically activated
- User can view and reactivate stashed overlays via overlay panel
- This prevents accidental continuation of old staging sessions

---

## Use Cases

| Trigger | Overlay Contains |
|---------|------------------|
| **Import** | Entities that would be created/merged |
| **Bulk edit** | Field changes that would apply |
| **Script** | Generated entities/changes |
| **What-if** | Hypothetical deletions/edits |
| **Rule preview** | Changes the rule would make |
| **Config change** | Merges/attachments that would occur |
| **Batch entry** | Accumulated edits before commit |

---

## Overlays and Bundles

- Overlay operations map naturally to operation bundles
- Committing an overlay produces a bundle in canonical history
- Bundle metadata includes overlay source and display name

---

## Undo/Redo Within Overlays

- Overlays support their own undo/redo stack for operations within the overlay
- Overlay undo stack is **non-persistent** (cleared on app restart, like canonical undo)
- Overlays themselves persist across restarts; their undo history does not
- Undo within overlay removes operations from the overlay (reverts to canonical value)
- Redo within overlay re-adds previously undone operations

---

## Open Questions

- Overlay merge (combine two overlays into one)
- Overlay branching (fork overlay to try alternatives)
