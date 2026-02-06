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
| [operations.md](spec/operations.md) | Oplog, bundles, HLC timestamps |
| [snapshots.md](spec/snapshots.md) | Snapshots, segments, archiving, GC |
| [sync.md](spec/sync.md) | Replication, leader election, partitions |
| [overlays.md](spec/overlays.md) | Staging, transport router, canonical drift |
| [proposals.md](spec/proposals.md) | Lifecycle, acceptance, bundled proposals |
| [conflicts.md](spec/conflicts.md) | Detection, resolution, GC |
| [rules.md](spec/rules.md) | Facet attachment, matching, derived fields |
| [scripts.md](spec/scripts.md) | Lua API, sessions, triggers, capabilities |
| [modules.md](spec/modules.md) | Schema, capabilities, views, scripts |
| [schema-evolution.md](spec/schema-evolution.md) | Versioning, migrations |
| [identity.md](spec/identity.md) | Actors, signing, permissions, roles |
| [workspace.md](spec/workspace.md) | Lifecycle, join modes, forks, recovery |

---

## Project Status

**Current phase:** Design with rough prototype. Core ideas are proven but implementation needs work.

**What exists:** Working HLC/sync/module loading, relationships, queries. Messy but demonstrates feasibility.

**What's needed:** Clean implementation of the core, especially peer-to-peer replication.

---

## Open Questions

These areas need further design work:

1. **Proposal workflows** — Expiration policies, notification model, dependencies

See [ARCHITECTURE.md](ARCHITECTURE.md) for full context on open questions.
