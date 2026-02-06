# Open Topics

These topics are intentionally deferred for future versions. They were identified during the documentation review but do not block v1 implementation.

## Script Module System

**Question:** How do scripts import shared libraries?

**Considerations:**
- Sandboxing implications
- Version management for shared code
- Dependency resolution

## Proposal Dependencies

**Question:** Can proposals have dependencies (accept A requires B)?

**Considerations:**
- DAG of proposals
- Circular dependency detection
- UX for chained approvals

## Overlay Merge/Branch Operations

**Question:** Can overlays be merged (combine two) or branched (fork to try alternatives)?

**Considerations:**
- Conflict resolution between overlays
- UX for managing overlay trees
- Performance implications

## Complex Derived Fields

**Question:** Should derived fields support conditionals, fallbacks, or complex expressions?

**Considerations:**
- Expression language design
- Determinism requirements
- Performance of complex derivations

## Approval Delegation

**Question:** Can an approver delegate their authority temporarily?

**Considerations:**
- Time-bounded delegation
- Audit trail for delegated approvals
- Revocation of delegation

## Cloud Archive Authentication

**Question:** What authentication model for cloud archive storage?

**Considerations:**
- Identity federation
- Key management
- Multi-tenant isolation
