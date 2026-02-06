# Openprod Documentation

Openprod is a collaborative workspace for live entertainment production teams. This documentation explains what the system does, why it's designed this way, and how to implement it.

---

## Documentation Structure

| Document | Audience | Purpose |
|----------|----------|---------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Contributors | Vision, principles, *why* decisions were made |
| [FUNCTIONALITY.md](FUNCTIONALITY.md) | End users | *What* users experience, how features work |
| [INVARIANTS.md](INVARIANTS.md) | Implementers | Constraints that must always hold (checklist) |
| [spec/](spec/README.md) | Implementers | *How* to build each subsystem |

---

## Quick Start

**For end users:** Start with [FUNCTIONALITY.md](FUNCTIONALITY.md) to understand what Openprod does and how to use it.

**For contributors:** Read [ARCHITECTURE.md](ARCHITECTURE.md) first to understand the vision and design principles.

**For implementers:** Use [INVARIANTS.md](INVARIANTS.md) as a checklist of rules that must hold, then dive into [spec/](spec/README.md) for detailed subsystem specifications.

---

## Specification Index

The [spec/](spec/README.md) directory contains modular implementation specifications:

| Spec | Subsystem |
|------|-----------|
| [data-model.md](spec/data-model.md) | Entities, fields, facets, edges |
| [operations.md](spec/operations.md) | Oplog, bundles, operation types |
| [identity.md](spec/identity.md) | Actors, Ed25519 signing, permissions |
| [hlc.md](spec/hlc.md) | Hybrid Logical Clock timestamps |
| [sqlite-schema.md](spec/sqlite-schema.md) | SQLite table definitions, indexes |
| [conflicts.md](spec/conflicts.md) | Detection, resolution, convergence |
| [crdt.md](spec/crdt.md) | CRDT field semantics |
| [ordered-edges.md](spec/ordered-edges.md) | Ordered edge positioning |
| [overlays.md](spec/overlays.md) | Staging, transport router, canonical drift |
| [sync.md](spec/sync.md) | Replication, leader election, partitions |
| [rules.md](spec/rules.md) | Facet attachment, matching, derived fields |
| [scripts.md](spec/scripts.md) | Lua API, sessions, triggers, capabilities |
| [modules.md](spec/modules.md) | Schema, capabilities, views, scripts |
| [query-language.md](spec/query-language.md) | Query syntax and semantics |
| [workspace.md](spec/workspace.md) | Lifecycle, join modes, forks, recovery |

---

## Project Status

**Phase 1 complete** (Feb 2026): core, storage, and harness crates implemented with 37+ passing tests. Core operations, oplog, bundles, HLC, Ed25519 signing, SQLite materialization, and vector clocks are working.

**Phase 2 in progress:** Engine crate with command/query separation, undo/redo, and state rebuild.

**What's next:** Conflict detection, overlays, sync, rules engine, scripting.

---

## Open Questions

See [plans/open-topics.md](plans/open-topics.md) for deferred design topics and [ARCHITECTURE.md](ARCHITECTURE.md) for full context.
