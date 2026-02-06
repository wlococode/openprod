# Approval Workflows Specification

This document defines approval policies, required proposals, role-based acceptance, and multi-party approval workflows.

---

## Overview

Approval workflows extend the proposal system to support required review gates. While proposals are normally voluntary (users can edit directly if they have permission), approval policies make proposals mandatory for specific scopes.

**Anchor invariant:** Approval policies route edits through proposals. They do not grant or deny permissions—they change the path an edit must take to reach canonical state.

---

## Relationship to Existing Systems

| System | Purpose | Approval Workflows Add |
|--------|---------|----------------------|
| **Proposals** | Optional suggested changes | Required proposal routing |
| **Permissions** | Who can perform actions | Who can accept proposals in scope |
| **Roles** | Permission bundles | Approval authority as role capability |
| **Rules** | Automated actions | N/A (approval is human-in-the-loop) |

Approval workflows layer on top of these systems without modifying their core semantics.

---

## Approval Policies

An approval policy defines when proposals are required and who can accept them.

### Policy Structure

```yaml
ApprovalPolicy:
  id: unique_policy_id
  name: "SM approves call time changes"
  description: "Stage manager must approve any changes to performer call times"

  # What this policy applies to
  scope:
    kind: "Person"                    # Optional: filter by kind
    facet: "scheduler"                # Optional: filter by facet
    fields: [call_time, call_location] # Optional: specific fields
    query: "department == 'cast'"     # Optional: additional filter

  # Routing behavior
  require_proposal: true              # Edits become proposals

  # Who can accept
  accept_roles: [stage_manager, production_manager]

  # Optional: multi-party approval
  approval_requirements:
    mode: any                         # any | all | quorum
    count: 1                          # For quorum mode

  # Optional: expiration
  expires_after_hours: 48
  on_expire: notify                   # notify | escalate | auto_reject
  escalate_to: [production_manager]

  # Policy metadata
  priority: 100                       # Higher priority = evaluated first
  enabled: true
  created_by: actor_id
  created_at: hlc_timestamp
```

### Scope Resolution

Policies are matched against operations using scope filters:

```
function policy_applies(policy, operation) -> bool:
    # All specified filters must match (AND logic)

    if policy.scope.kind and entity.kind != policy.scope.kind:
        return false

    if policy.scope.facet and field.facet != policy.scope.facet:
        return false

    if policy.scope.fields and field.key not in policy.scope.fields:
        return false

    if policy.scope.query and not evaluate_query(policy.scope.query, entity):
        return false

    return true
```

### Policy Priority

When multiple policies could apply, the highest priority policy wins:

```yaml
# Policy A: priority 100
scope: { kind: "Person", fields: [call_time] }
accept_roles: [stage_manager]

# Policy B: priority 50
scope: { kind: "Person" }
accept_roles: [production_manager]

# Edit to Person.call_time → Policy A applies (higher priority)
# Edit to Person.email → Policy B applies (only match)
```

**Anchor invariant:** At most one approval policy applies to any given operation. The highest-priority matching policy wins.

---

## Edit Routing

When an actor attempts an edit that matches an approval policy:

```
┌─────────────────┐
│  Edit Operation │
└────────┬────────┘
         │
         ▼
┌─────────────────────────┐
│ Check approval policies │
└────────────┬────────────┘
             │
      ┌──────┴──────┐
      │   Match?    │
      └──────┬──────┘
             │
    Yes ─────┴───── No
     │               │
     ▼               ▼
┌──────────┐   ┌──────────────┐
│ Route to │   │ Normal edit  │
│ Proposal │   │ (if permitted)│
└──────────┘   └──────────────┘
```

### Routing Behavior

| Actor Permission | Policy Applies | Result |
|-----------------|----------------|--------|
| Has `can_edit` | No | Direct edit |
| Has `can_edit` | Yes | Edit becomes proposal |
| Has `can_propose` only | No | Must propose (existing behavior) |
| Has `can_propose` only | Yes | Must propose (unchanged) |
| No permission | Either | Rejected |

**Key insight:** Approval policies don't reduce permissions—they change routing. An Editor who normally edits directly is routed through proposals when a policy applies.

### Routing in Overlays

When editing in an overlay with an active approval policy:

1. Edit is stored in overlay normally
2. On commit, routing is evaluated
3. If policy applies: overlay becomes a proposal (not canonical)
4. If no policy: overlay commits to canonical

**Anchor invariant:** Approval policy routing is evaluated at commit time, not during overlay editing. Users can experiment freely; policies gate the path to canonical.

---

## Acceptance Authority

### The `can_accept_proposal` Permission

A new permission category for proposal acceptance within policy scope:

```yaml
Permission: can_accept_proposal
Scopes: Global, Kind, Facet, Field
```

This permission is distinct from `can_edit`:

| Permission | Allows |
|------------|--------|
| `can_edit` | Direct edits (when no policy applies) |
| `can_accept_proposal` | Accepting proposals in scope |

### Role-Based Acceptance

Approval policies specify `accept_roles`. Actors with those roles gain implicit `can_accept_proposal` for the policy's scope:

```yaml
ApprovalPolicy:
  scope: { kind: "Person", fields: [call_time] }
  accept_roles: [stage_manager]

# Equivalent to granting:
Role: stage_manager
  permissions:
    - can_accept_proposal at Field=call_time (for kind=Person)
```

### Acceptance Algorithm

```
function can_accept(actor, proposal) -> bool:
    policy = get_policy_for_proposal(proposal)

    if policy is null:
        # No policy = standard acceptance rules
        return check_permission(actor, "can_edit", proposal.context)

    # Policy-governed acceptance
    actor_roles = get_actor_roles(actor)
    return any(role in policy.accept_roles for role in actor_roles)
```

### Self-Acceptance

**Anchor invariant:** An actor cannot accept their own proposal when an approval policy applies, unless they hold an accept_role.

| Scenario | Can Self-Accept? |
|----------|-----------------|
| No policy, has `can_edit` | Yes (proposal is optional anyway) |
| Policy applies, not in `accept_roles` | No |
| Policy applies, is in `accept_roles` | Yes |

This prevents circumventing approval workflows while allowing authorized users to fast-track their own proposals.

---

## Multi-Party Approval

For workflows requiring multiple approvals before acceptance.

### Approval Requirements

```yaml
approval_requirements:
  mode: any | all | quorum
  count: N                    # For quorum mode
  roles: [role_a, role_b]     # Optional: override accept_roles
```

| Mode | Behavior |
|------|----------|
| `any` | First approval from any `accept_role` accepts the proposal |
| `all` | Every role in `accept_roles` (or `roles`) must approve |
| `quorum` | At least `count` distinct approvals required |

### Approval State

Multi-party proposals track approval progress:

```yaml
ProposalApprovalState:
  proposal_id: <uuid>
  required_mode: all
  required_roles: [stage_manager, director]
  approvals:
    - actor: alice_actor_id
      role: stage_manager
      timestamp: hlc
    - actor: bob_actor_id
      role: director
      timestamp: hlc
  status: pending | approved | rejected
```

### Partial Approval Operations

```yaml
Operation: ApproveProposal
  proposal_id: <uuid>
  actor: <actor_id>
  role: <role used for approval>
  hlc: <timestamp>

# Distinct from AcceptProposal, which finalizes
```

When all requirements are met, the proposal auto-transitions to accepted and emits canonical operations.

### Rejection in Multi-Party

Any authorized actor can reject. Rejection is immediate and final:

```yaml
Operation: RejectProposal
  proposal_id: <uuid>
  actor: <actor_id>
  reason: "Timing conflicts with scene change"
  hlc: <timestamp>
```

Rejection clears all partial approvals. The proposer can create a new proposal if desired.

---

## Expiration and Escalation

### Expiration

Proposals under approval policies can expire:

```yaml
expires_after_hours: 48
on_expire: notify | escalate | auto_reject
```

| Action | Behavior |
|--------|----------|
| `notify` | Proposal remains pending; notification sent to proposer and accept_roles |
| `escalate` | Proposal reassigned to escalation roles; original accept_roles can still approve |
| `auto_reject` | Proposal auto-rejected; notification sent |

### Escalation

```yaml
escalate_to: [production_manager, admin]
escalate_after_hours: 24
```

Escalation adds roles to the acceptance pool without removing original roles:

```
Original accept_roles: [stage_manager]
After escalation: [stage_manager, production_manager, admin]
```

### Expiration Timing

**Anchor invariant:** Expiration is calculated from proposal creation HLC, not wall-clock time. Offline peers process expirations on reconnect using canonical HLC ordering.

```
function check_expiration(proposal, current_hlc):
    if proposal.policy.expires_after_hours is null:
        return  # No expiration

    age_hours = hlc_to_hours(current_hlc - proposal.created_hlc)

    if age_hours >= proposal.policy.expires_after_hours:
        apply_expiration_action(proposal, proposal.policy.on_expire)
```

---

## Notifications

Approval workflows generate notifications for relevant actors.

### Notification Events

| Event | Recipients |
|-------|------------|
| Proposal created (policy applies) | `accept_roles` |
| Proposal approved (partial) | Proposer, remaining `accept_roles` |
| Proposal accepted (complete) | Proposer |
| Proposal rejected | Proposer |
| Proposal expiring soon | Proposer, `accept_roles` |
| Proposal expired | Proposer, `accept_roles` |
| Proposal escalated | Proposer, `escalate_to` roles |

### Notification Structure

```yaml
Notification:
  type: proposal_pending | approval_added | proposal_accepted | ...
  proposal_id: <uuid>
  entity_context:
    kind: "Person"
    name: "Jane Doe"         # For human-readable context
    field: "call_time"
  actor_context:
    proposer: alice_actor_id
    approver: bob_actor_id   # If applicable
  message: "Call time change for Jane Doe requires your approval"
  created_at: hlc
```

### Notification Delivery

Notifications are operations in the oplog and sync to all peers:

```yaml
Operation: CreateNotification
  notification: { ... }
  recipients: [actor_id, ...]
  hlc: <timestamp>
```

UI surfaces notifications; external delivery (email, SMS) is a plugin capability.

---

## Interaction with Conflicts

### Proposal on Conflicted Field

If a proposal targets a field that becomes conflicted:

1. Proposal remains valid
2. Accepting the proposal resolves the conflict to the proposed value
3. Same behavior as direct edit on conflicted field

### Conflict During Approval

If a conflict arises on a field with a pending proposal:

```
Field: call_time
  - Canonical: 10:00 (conflicted: Alex=10:00, Jordan=10:30)
  - Pending proposal: 11:00 (from Carol)

Accepting Carol's proposal:
  - Resolves conflict to 11:00
  - Conflict marked resolved
  - Proposal marked accepted
```

---

## Interaction with Overlays

### Editing in Overlay

Users can edit policy-governed fields in overlays freely. Policy routing applies at commit:

```
User in overlay:
  1. Edits call_time (policy applies)
  2. Edits email (no policy)
  3. Commits overlay

Result:
  - email edit → canonical (direct)
  - call_time edit → proposal (routed)
```

**Anchor invariant:** Overlay commits may split into direct operations and proposals when approval policies apply to some but not all operations.

### Mixed Commit Behavior

When an overlay contains both policy-governed and non-governed edits:

```yaml
CommitResult:
  direct_operations: [email_edit]     # Applied to canonical
  proposals_created: [call_time_edit] # Routed to proposal
  message: "1 change applied, 1 change requires approval"
```

The user is informed which changes went through directly and which require approval.

---

## Interaction with Rules

### Rule-Triggered Edits

When a rule triggers an edit on a policy-governed field:

**Anchor invariant:** Rule-triggered edits are subject to approval policies. The proposal is attributed to the user whose action triggered the rule.

```
User edits: status = "confirmed"
Rule triggers: set call_time = calculate_call()

If call_time has approval policy:
  - Rule output becomes a proposal
  - Proposal attributed to original user
  - accept_roles must approve
```

This ensures approval gates cannot be bypassed via rules.

### Rule-Generated Proposals

Rule-generated proposals behave identically to user proposals:
- Same acceptance requirements
- Same expiration behavior
- Same notification flow

The only difference is the `source` metadata indicating rule origin.

---

## Policy Operations

Approval policies are workspace configuration stored as operations.

### Creating a Policy

```yaml
Operation: CreateApprovalPolicy
  policy: { ... }
  actor: admin_actor_id
  hlc: <timestamp>
```

Requires `can_manage_rules` permission (approval policies are rule-like configuration).

### Modifying a Policy

```yaml
Operation: UpdateApprovalPolicy
  policy_id: <uuid>
  changes:
    accept_roles: [stage_manager, assistant_stage_manager]
  actor: admin_actor_id
  hlc: <timestamp>
```

Changes apply to new proposals. Existing pending proposals retain their original policy snapshot.

### Disabling a Policy

```yaml
Operation: UpdateApprovalPolicy
  policy_id: <uuid>
  changes:
    enabled: false
  actor: admin_actor_id
  hlc: <timestamp>
```

Disabled policies don't route new edits. Existing proposals remain pending until resolved.

---

## Policy Snapshots

**Anchor invariant:** Proposals capture a snapshot of the applicable policy at creation time. Policy changes don't retroactively affect pending proposals.

```yaml
Proposal:
  id: <uuid>
  operations: [...]
  policy_snapshot:
    policy_id: <uuid>
    accept_roles: [stage_manager]      # At proposal creation
    approval_requirements: { mode: any }
    expires_after_hours: 48
```

This ensures:
- Proposals are evaluated consistently
- Policy changes don't invalidate pending work
- Audit trail shows what rules applied

---

## Default Policies

Workspaces may define default policies that apply broadly:

```yaml
# Workspace default: all Cue edits require SM approval
ApprovalPolicy:
  name: "Default cue approval"
  scope: { kind: "Cue" }
  accept_roles: [stage_manager]
  priority: 10                    # Low priority, easily overridden
```

Plugin-specific policies can override with higher priority:

```yaml
# Lighting plugin: lighting cues can be approved by LD
ApprovalPolicy:
  name: "Lighting cue approval"
  scope: { kind: "Cue", facet: "lighting" }
  accept_roles: [lighting_designer, stage_manager]
  priority: 100                   # Higher priority, takes precedence
```

---

## Bypass Conditions

Some scenarios may allow policy bypass:

### Emergency Bypass

```yaml
ApprovalPolicy:
  # ...
  allow_bypass:
    roles: [admin, production_manager]
    requires_reason: true
```

When bypassing:

```yaml
Operation: BypassApprovalPolicy
  policy_id: <uuid>
  operation: { ... }              # The edit being made
  reason: "Emergency timing fix during performance"
  actor: admin_actor_id
  hlc: <timestamp>
```

**Anchor invariant:** Bypasses are auditable operations. The bypass, reason, and actor are permanently recorded.

### Auto-Accept Conditions

```yaml
ApprovalPolicy:
  # ...
  auto_accept_when:
    query: "change_magnitude < 5"   # e.g., small timing adjustments
```

When auto-accept conditions are met, the proposal is created and immediately accepted (with audit trail showing auto-acceptance).

---

## UI Considerations

### Proposal List Enhancements

Proposals under approval policies show:
- Policy name and requirements
- Current approval state (for multi-party)
- Time until expiration
- Escalation status

### Edit Experience

When editing a policy-governed field:
- Indicator showing approval will be required
- Preview of who can approve
- Option to add context/justification

### Acceptance Experience

Approvers see:
- What's being changed (before/after)
- Who proposed and why
- Other pending approvals (for multi-party)
- Accept / Reject / Request Changes actions

---

## Audit Trail

All approval workflow actions are operations in the oplog:

| Operation | Records |
|-----------|---------|
| `CreateApprovalPolicy` | Policy definition |
| `UpdateApprovalPolicy` | Policy changes |
| `CreateProposal` | Proposal with policy snapshot |
| `ApproveProposal` | Partial approval (multi-party) |
| `AcceptProposal` | Final acceptance |
| `RejectProposal` | Rejection with reason |
| `ExpireProposal` | Automatic expiration |
| `EscalateProposal` | Escalation event |
| `BypassApprovalPolicy` | Emergency bypass with reason |

History queries can answer:
- "Who approved this change?"
- "How long did approval take?"
- "What proposals were rejected and why?"
- "How often is this policy bypassed?"

---

## CRDT Fields and Approval

**Anchor invariant:** CRDT fields are exempt from approval policies. `ApplyCRDT` operations always apply immediately without routing through proposals.

### Rationale

CRDT fields enable real-time collaborative editing where concurrent changes merge automatically. Routing CRDT operations through approval would:

1. Defeat the purpose of CRDTs (no real-time collaboration)
2. Create complex merge scenarios (approval order vs. CRDT merge order)
3. Block collaboration until approver reviews every keystroke

### What This Means

| Operation | Approval Policy Applies? |
|-----------|-------------------------|
| `SetField` on plain field | Yes |
| `SetField` on CRDT field (full replacement) | Yes |
| `ApplyCRDT` (delta update) | **No** |
| `CreateOrderedEdge` | Yes (edge creation is discrete) |
| `MoveOrderedEdge` | Yes |

### Design Guidance

If a field requires approval before changes are visible:
- **Don't make it a CRDT field** — use a plain field with normal approval
- CRDT fields are for collaborative content where all contributions are welcome

### Future: Suggestions Mode

A Google Docs-style "Suggestions" mode is planned for CRDT fields:

- Edits marked as suggestions (visible but not canonical)
- Inline accept/reject per suggestion
- Preserves real-time collaboration while enabling review

See [crdt.md](crdt.md) open questions for status.

---

## Open Questions

1. **Delegation:** Can an approver delegate their approval authority temporarily? (e.g., SM on break, ASM covers)

2. **Batch Approval:** Can multiple proposals be approved in one action? (e.g., "Approve all pending call time changes")

3. **Conditional Approval:** Can approvers approve with conditions? (e.g., "Approved if lighting confirms")

4. **Approval Comments:** Should approvers be able to add comments without approving/rejecting?

5. **Revision Requests:** Formal "request changes" flow that returns proposal to proposer for modification?

6. **Policy Templates:** Pre-built policies for common workflows (SM approval, department head approval)?

7. **Policy Inheritance:** Should policies cascade from workspace to entity? (e.g., workspace default + entity-specific override)

8. **Offline Approval:** How do approvals work when approver is offline? (Proposal waits; approver approves on reconnect; sync propagates)

9. **Proposal Versioning:** Can proposers update a pending proposal, or must they create a new one?

---

## Summary

Approval workflows extend proposals from voluntary suggestions to required gates. The key additions are:

1. **Approval policies** — Configuration that routes edits through proposals
2. **Scoped acceptance** — `accept_roles` determine who can accept, separate from `can_edit`
3. **Multi-party approval** — Optional requirement for multiple approvals
4. **Expiration and escalation** — Time-based workflow management
5. **Policy snapshots** — Proposals capture policy state at creation

These features enable production workflows like:
- "SM approves all call time changes"
- "Both SM and Director must approve blocking changes"
- "Lighting can edit their cues freely; SM cues require SM approval"
- "Emergency bypass for admins with mandatory reason"

All while maintaining the system's core properties: offline-first, deterministic, auditable, and conflict-aware.
