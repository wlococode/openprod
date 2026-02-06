# Proposals Specification

This document defines the proposal lifecycle, acceptance semantics, and bundled proposals.

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

---

## Proposal Lifecycle

- Proposals are created by explicit user or plugin action
- Proposals may be reviewed by any user with read access
- Proposals may be accepted only by users authorized to write to affected fields
- Proposals may be rejected by the proposer or by authorized users
- Proposals may expire or be withdrawn without affecting canonical state

---

## Proposal Visibility

- Proposals are visible to all peers within the workspace
- Proposals sync like other operations but do not affect canonical state
- Proposals may be filtered or grouped in UI for review
- Multiple proposals may exist for the same field simultaneously

---

## Multiple Proposals for Same Field

**Anchor invariant:** When multiple proposals exist for the same field, accepting one marks the others as `superseded`.

**Resolution flow:**
1. User views field with multiple pending proposals
2. UI shows all proposed values with proposer context
3. User selects one proposal to accept (or enters a new value, or accepts none)
4. Selected proposal is accepted → becomes canonical
5. Other proposals for that field are marked `superseded` (not `rejected`)

**Proposal statuses:**

| Status | Meaning |
|--------|---------|
| `pending` | Awaiting review |
| `accepted` | Proposal was accepted, canonical operations emitted |
| `rejected` | Proposal was explicitly rejected by reviewer |
| `superseded` | Another proposal or direct edit updated the field |
| `withdrawn` | Proposer withdrew the proposal |
| `expired` | Proposal exceeded expiration policy (if configured) |

---

## Relationship to Other Features

- Proposals may be created from overlay changes (commit as proposal instead of direct commit)
- Proposals may be created from transform previews
- Proposals do not create conflicts; conflicts arise only from committed canonical operations

---

## Proposals and Overlays

**Anchor invariant:** Proposing an overlay is atomic. All overlay operations become the proposal, and the overlay is discarded.

- Proposals may be created from overlay state
- When proposed, overlay operations move to proposal state; overlay is deleted
- Proposals are atomic bundles (cannot propose individual operations from overlay)
- To exclude operations, use Knockout before proposing
- Accepting a proposal emits canonical operations, not overlay operations
- Proposals created from overlays become independent once created

---

## Proposals and Conflicts Independence

- Proposals and conflicts are independent derived states
- Proposals do not suppress, replace, or affect conflict derivation
- Proposals never affect conflict detection or resolution until accepted
- A field may have both an open conflict and pending proposals simultaneously
- Conflict resolution and proposal acceptance are orthogonal operations

---

## Proposal Acceptance Semantics

**v1 Model: Single Approver**

In v1, proposals use a simple single-approver model:

- Any user with valid permission can accept or reject a proposal
- A single accept immediately applies the proposed changes to canonical state
- A single reject immediately discards the proposal
- No multi-party approval, quorum, or pooling in v1

**Approval permission:**
- Checked at the time of the approval action
- Based on current permission state (not a snapshot from proposal creation)
- Workspace owners and admins can always approve/reject any proposal

**Acceptance mechanics:**

- Accepting a proposal is semantically equivalent to performing the proposed operation directly
- Proposal acceptance must not bypass conflict detection or resolution rules
- If the proposed field is not conflicted, proposal acceptance emits a normal edit operation
- If the proposed field is conflicted, proposal acceptance emits a resolve_conflict operation
- Proposal acceptance respects the same authorization rules as direct edits
- Accepting a proposal for a conflicted field resolves the conflict to the proposed value

---

## Use Cases

- Collaborative review workflows (designer proposes → SM approves)
- Safe cross-plugin suggestions
- Bulk change review before commit
- Non-destructive experimentation shared with team

---

## Bundled Proposals

When creating a proposal from overlay state:
- Proposal contains all overlay operations as a bundle
- Collaborators can accept/reject the whole bundle atomically
- Collaborators can also review and accept/reject item-by-item
- Partial acceptance creates a new proposal with remaining items (or discards them)

---

## Related Specifications

- **[approval-workflows.md](approval-workflows.md)** — Required proposals, role-based acceptance, multi-party approval, expiration policies

---

## Open Questions

- Proposal dependencies (accept A requires accepting B)
