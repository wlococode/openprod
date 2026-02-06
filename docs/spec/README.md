# Openprod Specification

This directory contains detailed implementation specifications for each subsystem of Openprod. These documents are for implementers—they describe *how* to build each component.

---

## Document Map

| Document | Subsystem | Key Topics |
|----------|-----------|------------|
| [data-model.md](data-model.md) | Data Model | Tables, entities, fields, facets, edges, identity repair |
| [operations.md](operations.md) | Operations | Oplog, bundles, HLC timestamps, state derivation |
| [crdt.md](crdt.md) | CRDTs | Text CRDTs, list CRDTs, auto-merge semantics |
| [ordered-edges.md](ordered-edges.md) | Ordered Edges | Position identifiers, ordered entity lists |
| [hlc.md](hlc.md) | Time | Hybrid Logical Clock format, algorithm, drift handling |
| [sqlite-schema.md](sqlite-schema.md) | Storage | Table definitions, indexes, migrations |
| [sync.md](sync.md) | Replication | Leader election, partitions, vector clocks, catch-up |
| [overlays.md](overlays.md) | Overlays | Staging, transport router, canonical drift |
| [conflicts.md](conflicts.md) | Conflicts | Detection, resolution, GC, late-arriving edits |
| [rules.md](rules.md) | Rules | Table membership, matching, automation, cycle detection |
| [query-language.md](query-language.md) | Query Language | Grammar, operators, field access, edge traversal |
| [scripts.md](scripts.md) | Scripts | Lua API, async coroutines, sessions, triggers, capabilities |
| [modules.md](modules.md) | Modules | Schema, capabilities, views, scripts |
| [identity.md](identity.md) | Identity | Actor ID, oplog attribution, display names |
| [workspace.md](workspace.md) | Workspace | Lifecycle, join modes, forks, recovery |

---

## Reading Order

For a complete understanding, read in this order:

### Core Concepts (read first)
1. **[data-model.md](data-model.md)** — Understand what entities, fields, and facets are
2. **[operations.md](operations.md)** — Understand how changes are recorded
3. **[crdt.md](crdt.md)** — Understand collaborative text and list editing
4. **[ordered-edges.md](ordered-edges.md)** — Understand ordered entity lists
5. **[hlc.md](hlc.md)** — Understand deterministic time ordering
6. **[sqlite-schema.md](sqlite-schema.md)** — Understand local storage structure
7. **[sync.md](sync.md)** — Understand how peers collaborate

### User-Facing Features
8. **[overlays.md](overlays.md)** — Safe experimentation before commit
9. **[conflicts.md](conflicts.md)** — What happens when edits collide (CRDTs auto-merge)

### Automation & Modules
10. **[rules.md](rules.md)** — Automatic facet attachment and matching
11. **[scripts.md](scripts.md)** — User and module automation (includes CRDT APIs)
12. **[modules.md](modules.md)** — Module architecture and capabilities

### Advanced Topics
13. **[identity.md](identity.md)** — Authentication and authorization
14. **[workspace.md](workspace.md)** — Workspace lifecycle and recovery

---

## Archived / Deferred to Post-v1

The following specs have been moved to `docs/archive/`. They are deferred to post-v1 and not part of the current implementation scope.

| Document | Subsystem | Notes |
|----------|-----------|-------|
| wire-format.md | Wire Format | Binary encoding, compression, framing, bundle types |
| snapshots.md | Snapshots | Snapshots, segments, archiving, garbage collection |
| proposals.md | Proposals | Lifecycle, acceptance, bundled proposals |
| approval-workflows.md | Approval Workflows | Required proposals, role-based acceptance, multi-party approval |
| schema-evolution.md | Schema | Versioning, migrations, backward compatibility |

---

## Related Documents

| Document | Purpose | Audience |
|----------|---------|----------|
| [../ARCHITECTURE.md](../ARCHITECTURE.md) | Vision, principles, *why* decisions were made | Contributors |
| [../FUNCTIONALITY.md](../FUNCTIONALITY.md) | *What* users experience, how features work | End users |
| [../INVARIANTS.md](../INVARIANTS.md) | Constraints that must always hold | Implementers |

---

## Conventions

### Anchor Invariants

Each major section includes an **anchor invariant**—a one-sentence rule that must always hold:

> **Anchor invariant:** An entity is pure identity. All data lives in fields attached to the entity.

Anchor invariants are the non-negotiable constraints. Implementation details may evolve, but anchor invariants must not be violated.

### Open Questions

Sections marked **Open Questions** list unresolved design decisions. These require further investigation or stakeholder input before implementation.

### YAML/TOML Examples

Configuration examples use YAML or TOML to illustrate structure. These are illustrative, not prescriptive—actual serialization format may differ.

---

## Contributing

When updating specifications:

1. **Keep anchor invariants sacred** — If you need to change an anchor invariant, update [INVARIANTS.md](../INVARIANTS.md) first and ensure all specs align
2. **Cross-reference related specs** — Link to other documents when concepts overlap
3. **Mark open questions** — Don't leave ambiguity unmarked
4. **Update this index** — If you add a new spec file, add it to the document map
