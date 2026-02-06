# Conflicts Specification

This document defines conflict detection, resolution, late-arriving edits, and conflict garbage collection.

---

## Conflicts & Resolution

- Conflict occurs when two or more peers make **causally concurrent** edits to the same field
- Causal concurrency means neither peer had seen the other's edit when they wrote
- Conflict detection operates at field level
- Conflict presentation groups by bundle for user clarity
- N-way detection, presentation, resolution supported
- Detected any time two peers sync
- When open but not resolved, canonical data is HLC LWW value, with interface flag
- Resolved conflicts are re-opened if new peer adds new conflicting state (last resolved state is favored over LWW)
- Resolution produces an operation
- Conflicts are auditable and reversible

### Causal Concurrency Model

**Anchor invariant:** Conflicts are determined by causal concurrency, not HLC ordering. Two edits conflict if neither actor had seen the other's edit when they wrote.

Vector clocks determine causal knowledge:
- If `A.vector_clock[B] < B_op.HLC`, then A didn't see B's operation when A wrote
- If both A and B didn't see each other's ops, they are causally concurrent -> conflict

### Branch Tips (Competing Values)

**Anchor invariant:** In an N-way conflict, competing values are the **latest value from each causal branch**, not all intermediate values.

| Scenario | Competing Values |
|----------|------------------|
| Alice offline: todo -> blocked -> wontfix -> blocked | Alice's branch tip: "blocked" |
| Bob online: todo -> in_progress -> done | Bob's branch tip: "done" |
| Conflict | "blocked" vs "done" (2 values, not 6) |

More edits or later edits don't give priority. Only the branch tips are presented for resolution.

---

## Field-Level Conflict Granularity

- Conflicts are defined at semantic field granularity, not entity granularity
- Concurrent edits to different semantic fields of the same entity do not constitute a conflict
- All non-conflicting field edits are preserved after sync; no edit is discarded due to sequencing or wall-clock timing alone
- Wall-clock ordering does not determine precedence when fields do not overlap
- After sync, canonical entity state may reflect contributions from multiple actors

---

## CRDT Fields: Automatic Merge

**Anchor invariant:** CRDT fields do not produce field-level conflicts. Concurrent edits are merged automatically by the CRDT algorithm.

See [crdt.md](crdt.md) and [ordered-edges.md](ordered-edges.md) for CRDT specifications.

### Text CRDT Merge Behavior

| Scenario | Result |
|----------|--------|
| Concurrent character insertions | Both appear (deterministic order by actor) |
| Concurrent deletions | Both deletions applied |
| Insert at deleted position | Insert preserved |
| Concurrent paragraph edits | Merged at character level |

No conflict is surfaced to the user. All edits are preserved and merged.

### List CRDT Merge Behavior

| Scenario | Result |
|----------|--------|
| Concurrent insertions at same index | Both appear (deterministic order) |
| Concurrent deletions | Both deletions applied |
| Insert + delete at same index | Insert preserved |

### Ordered Edge Merge Behavior

| Scenario | Result |
|----------|--------|
| Concurrent insertions at same position | Both appear (tiebreak by actor_id) |
| Concurrent moves of same edge | LWW by HLC (later move wins) |
| Move + delete | Delete wins |

### What Can Still Conflict on CRDT Entities

CRDT merge only applies to the CRDT content itself. Other aspects may still conflict:

| Aspect | Conflict Behavior |
|--------|-------------------|
| CRDT field content | Auto-merged (no conflict) |
| Concurrent `SetField` on CRDT field | LWW (full state replacement) |
| Non-CRDT fields on same entity | Normal conflict rules |
| Non-position edge properties | Normal conflict rules (LWW or surfaced) |
| Permissions/metadata | Normal conflict rules |

### SetField on CRDT Fields

**Anchor invariant:** `SetField` operations on CRDT fields are rejected at validation time. Use CRDT-specific operations for CRDT fields.

```yaml
# REJECTED at operation validation
SetField:
  entity_id: <uuid>
  field: "description"  # Declared as CRDT in schema
  value: "New text"
  # -> ValidationError: SetField not allowed on CRDT field

# ALLOWED
ApplyCRDT:
  entity_id: <uuid>
  field: "description"
  delta: <crdt_delta>
  # -> Applies delta to CRDT state
```

**Rationale:** Mixing `SetField` and `ApplyCRDT` creates ambiguous semantics. A `SetField` could silently discard concurrent CRDT edits from other users.

### ClearAndAdd Operation (Reset Use Case)

For the "reset to specific values" use case, use the `ClearAndAdd` CRDT operation:

```yaml
ClearAndAdd:
  entity_id: <uuid>
  field: "tags"           # CRDT set field
  values: ["low", "priority"]
  actor_id: <actor>
  hlc: <timestamp>

Semantics:
  - Removes all elements added before this op's HLC
  - Adds the specified elements
  - Concurrent adds AFTER this HLC still apply
```

**Example:**
```
Alice: AddToSet("urgent")           @ HLC 100
Bob:   ClearAndAdd(["low"])         @ HLC 150
Alice: AddToSet("important")        @ HLC 200

Result: ["low", "important"]
  - "urgent" cleared (HLC 100 < 150)
  - "low" added by ClearAndAdd
  - "important" survives (HLC 200 > 150)
```

### Schema Migration: Regular -> CRDT

When a schema change converts a field from regular to CRDT:

```yaml
ConvertToCRDT:
  entity_id: <uuid>
  field: "description"
  hlc: <timestamp>

Semantics:
  - Snapshot current value at this HLC
  - That becomes the CRDT's initial state
  - Historical SetField ops are "sealed" -- not replayed as CRDT ops
  - New CRDT ops apply on top
```

### Schema Migration: CRDT -> Regular

**Not recommended.** Converting CRDT back to regular field is lossy (concurrent ops become conflicts).

Options if needed:
- Disallow (once CRDT, always CRDT)
- Require explicit "resolve to single value" step before conversion

### Structural Conflicts (Rare)

Some CRDT implementations may detect structural conflicts that cannot be auto-merged (e.g., concurrent block-type changes in rich text). If the CRDT algorithm surfaces a structural conflict:

1. The algorithm chooses a deterministic resolution
2. The conflict may be logged for user awareness
3. No explicit resolution required

Structural conflicts are implementation-dependent and rare in practice.

---

## Unresolved Conflict Display Value

**Anchor invariant:** Unresolved conflicts use LWW (Last Writer Wins by HLC) as the display value. This ensures data remains usable while flagged as contested.

- When a conflict is open but not resolved, the field displays the LWW value
- The conflict flag indicates this value is contested and may be incorrect
- LWW provides a usable fallback rather than null or error state
- Queries, views, and exports use the LWW value (with conflict flag available)
- Users can resolve conflicts to confirm or change the displayed value

**Rationale:** Setting conflicted fields to null or an error value would break dependent queries, views, and exports. LWW ensures data is at least valid and legible, even if potentially incorrect.

---

## Awareness Events

**Anchor invariant:** Awareness events are short-lived courtesy notifications for near-simultaneous edits that don't technically conflict.

**What triggers an awareness event:**
- Two users edit different fields of the same entity within ~1 second
- Both users were technically "aware" of each other's edits (synced state)
- No conflict exists because different fields were edited
- But proximity suggests one or both may not have noticed the other's edit

**Awareness event behavior:**
- Non-blocking, minimally invasive notification
- Persists for 30 seconds to 5 minutes (configurable)
- Cleared on app restart
- Does not affect canonical state or history
- Pure UX feature to improve collaboration awareness

**Example:**
```
Alice edits entity.name at HLC 100
Bob edits entity.email at HLC 100.5

-> No conflict (different fields)
-> Awareness event: "Bob also edited this entity just now"
```

---

## Conflict Surfacing

- Conflicts may be detected during sync but are surfaced only after a stable merge point
- The set of surfaced conflicts represents the complete known conflict set at that point
- On reconnection, the author continues to see their optimistic value initially
- After reconciliation reaches a stable point, conflicts are detected and surfaced
- Both conflicting values are visible for resolution; neither is silently overwritten

---

## Resolution as History

- Conflict resolution is always recorded as a new operation in history
- Original conflicting operations are immutable and are never modified retroactively
- Resolution operations are part of canonical history and participate in deterministic replay
- The existence of the conflict itself is never erased by resolution

---

## Resolution Content

- Resolution operations explicitly reference the conflict they resolve
- Resolution operations declare the chosen outcome as an explicit decision
- Resolution operations record the identity of the resolving actor
- Resolution operations record the chosen value and/or a reference to the accepted conflict state
- Resolution operations record logical time, not wall-clock time as authority

---

## Resolution Auditability & Retention

- Original conflicting operations remain auditable until explicitly garbage collected
- Conflict records may be garbage collected under explicit retention policy
- Garbage collection does not alter historical meaning or invalidate prior resolution decisions
- Resolution operations must remain interpretable even if referenced conflict records are no longer present

---

## Resolution Immutability & Revision

- Conflict resolution operations are immutable and are never modified or deleted
- Revisiting or changing a prior resolution produces a new operation
- A later resolution may supersede the outcome of an earlier resolution without invalidating it
- Canonical state reflects the most recent valid resolution or edit in sequence order
- Reversing a resolution is semantically distinct from making a new edit
- Resolution revision operations represent a change in decision about a prior conflict
- Normal edit operations represent new intent applied to the current canonical state
- Resolution revisions explicitly reference the resolution they supersede
- Normal edits do not retroactively alter conflict history
- The full lineage is preserved: original conflict -> first resolution -> later override

---

## Late-Arriving Edits & Conflict Reopening

**Anchor invariant:** Conflicts are determined by causal concurrency at the time of the edit, not by arrival time. The divergence point is the last operation both peers had seen.

### Divergence Point

The **divergence point** is determined by vector clock comparison--the last operation both peers had seen before their edits diverged.

```
Timeline:
  Day 1: Alice sets task.status = "todo" (HLC 100) -- both see this
  Day 2: Alice goes offline
  Day 3: Bob sets task.status = "in_progress" (HLC 200)
  Day 4: Bob sets task.status = "done" (HLC 300)
  Day 5: Alice (offline) sets task.status = "blocked" (HLC 150)
  Day 6: Alice reconnects

Divergence point: HLC 100 ("todo") -- last op both saw
Alice's branch tip: "blocked"
Bob's branch tip: "done"
Conflict: "blocked" vs "done"
```

### Staleness vs Conflict

**Staleness is purely a UX warning, not a conflict mechanic.**

- An operation is "stale" if its wall-clock time is suspiciously old (e.g., >7 days)
- Staleness indicates "this came from an offline device with old data"
- Stale operations still follow normal conflict rules based on causal concurrency
- The staleness badge helps users notice "old changes just arrived" but doesn't affect ordering or conflicts

### Conflict Reopening

- A conflict represents competing edits made without knowledge of each other
- Conflict resolution records a decision that closes a divergence window
- A resolved conflict remains resolved until explicitly superseded by a new resolution operation
- A conflict may be reopened by edits from peers who were unaware of the resolution
- Late-arriving edits from peers unaware of a resolution extend the existing conflict
- Edits made after a resolution was known create new conflicts, not reopen old ones
- Canonical state remains determined by the most recent explicit resolution decision
- While a conflict is unresolved or reopened, canonical state is derived deterministically and remains stable
- The reconnecting peer sees the current canonical state (determined by prior resolution)
- The reconnecting peer's offline edit is preserved and not discarded
- The reconnecting peer is notified of the conflict their edit has reopened or extended

---

## Conflicts Across Mapped Fields

- When multiple module fields map to the same underlying data, they represent a single semantic field
- Conflicts are detected and resolved across all fields mapped to the same data
- Concurrent edits to mapped fields constitute a single conflict, not multiple independent conflicts
- Resolving a mapped field conflict updates the shared value (visible to all mapped fields)
- Fields that are not mapped are semantically independent (namespaced)
- Concurrent edits to namespaced fields do not conflict with each other, even if they belong to the same entity
- The system only infers conflicts across modules when they share the same field mapping

---

## Resolution Authority

- Conflict resolution is a write operation and is subject to normal authorization rules
- A conflict may be resolved only by a user authorized to write to the affected field(s)
- Resolution authority is evaluated at the field level
- Resolution requires the same permission as writing the field normally
- No special "resolve conflicts" permission exists separate from field write permission
- No special authority is granted solely by the presence of a conflict
- The system does not enforce neutrality or conflict-of-interest rules
- Users without write permission must not be allowed to resolve conflicts
- Unauthorized conflict resolution attempts are rejected and do not affect history

---

## Concurrent Resolution Attempts

- Conflict resolution operations target a specific unresolved conflict state
- Resolution validity is determined by whether the conflict was unresolved at the time the resolution is applied
- At most one resolution may be accepted for a given conflict state
- The first valid resolution sequenced against an unresolved conflict is accepted
- Subsequent resolution attempts targeting an already-resolved conflict are invalid
- Rejected resolution attempts do not affect canonical state
- Rejected resolution attempts may be recorded as invalid operations for audit purposes
- Resolution operations must not conflict with each other or create recursive conflicts

---

## Resolution Stability

- Canonical state must not oscillate due to concurrent resolution attempts
- Resolution acceptance is conditional on conflict state, not arrival order alone
- Once a conflict is resolved, canonical state remains stable until explicitly changed
- Changing a resolved outcome requires explicitly revisiting or reopening the conflict

---

## Conflict Garbage Collection

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

---

## Conflict Compaction

- Conflict compaction must not modify or delete existing operations
- Compaction produces new summary operations
- Summaries must preserve actors, decisions, and ordering
- Detailed history may be hidden or expired, but summary truth must remain
- Compaction is interpretive summarization, not rewrite

---

## Partial History & Reopening

- Conflicts may be reopened even if some historical conflict details have been garbage collected
- Partial conflict history does not invalidate resolution
- Resolution decisions are based on available competing intents and canonical state

---

## Conflict Audit Requirements

- Auditability requirements define a minimum retained conflict summary
- Conflict summaries may replace detailed payloads after retention
- Minimum audit record: conflict identifier, semantic field, resolver identity, resolution time, chosen outcome, authorization proof
- We may forget details, but we never forget decisions

---

## Retention Policy

- Retention policy is explicit, workspace-scoped, and auditable
- Changes to retention policy are recorded as operations
- Retention policy changes are subject to authorization

---

## Open Questions

- UI presentation patterns
- Auto-resolution heuristics (never for critical fields?)
- Batch conflict tooling

---

## Related Documents

- [crdt.md](crdt.md) -- CRDT field specification (auto-merge for text and lists)
- [ordered-edges.md](ordered-edges.md) -- Ordered edge specification (auto-merge for ordered entity lists)
