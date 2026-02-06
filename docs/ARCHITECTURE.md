# Architecture Overview

## About This Document

I'm a lighting and video designer for theatre, opera, and concerts. I've spent about a year thinking through this architecture, and I have a rough prototype that proves the core ideas work—but it's not production-ready, and some of the harder problems are beyond my current skills.

This document describes what I want to build and why. Some parts are firm convictions; others are open questions where I need input. I've tried to be clear about which is which.

If you're reading this and something seems wrong, or you know a better way, I want to hear it.

---

## The Problem

In live entertainment production, departments rely on domain-specific tools, communication platforms, and frequent meetings to stay on the same page. Lighting uses Vectorworks and Lightwright. Stage management uses Word and Excel. Sound has their own apps. Everyone digs through Slack, email, and SharePoint trying to find what's "current."

The daily reality:

- Stage managers calculate call times for 50+ people by hand, cross-referencing schedules and scene breakdowns
- Lighting designers export Excel files with work notes every night and email them to their team
- Technical directors build from outdated scenic drafts because someone forgot to send the revision
- When something changes, every related document needs manual updates—and something always gets missed

Every program and venue has their own process. There's no standard. When paperwork is wrong, it's "oh, I made a typo" and another revision goes out. People just remember which version is current.

Live entertainment is a fascinating intersection of art and technology, but the workflows have not evolved to keep up with growing budgets, project scopes, and hungry audiences.

---

## The Vision

Openprod is a collaboration system for production teams. The core handles storage, syncing, and conflict resolution. Modules provide the actual functionality—contacts, schedules, cue sheets, paperwork layouts.

The key insight: most production paperwork is _derived_ from the same underlying information in a pretty deterministic way. If the data lives in one place and relationships are explicit, everything can stay in sync automatically.

A stage manager shouldn't have to think "what time is this person's first call?" for every company member. The system knows who's in which scenes/segments. It knows the schedule. The call time is a query, not a calculation someone does by hand.

**Think:** Obsidian's module model meets Git's offline-first collaboration, built for production workflows.

---

## Core Principles

These guide every design decision:

1. **Offline and local-first.** No subscription required. No cloud dependency. Your data lives on your machine and works without internet.

2. **Deterministic and auditable.** Every change is recorded. History is replayable. You can always see what changed, when, and by whom.

3. **Explicit over implicit.** All automated mutations are user-configured, visible, and auditable. Every automated action can be traced to a rule or trigger that the user explicitly created or approved. No automation runs without user consent.

4. **Domain-agnostic core.** The core knows about entities, relationships, and sync—not about lighting cues or call times. All domain knowledge lives in modules.

5. **Modules are independent.** Each module must be useful on its own. Interoperability between modules is opt-in, not assumed.

6. **Safety over convenience.** When there's a conflict, surface it for human resolution. Never silently overwrite someone's work.

---

## System Overview

The core handles:

- **Storage** — Entity/facet/edge graph in SQLite (WAL mode), asset blobs on disk
- **Sync** — LAN discovery and offline modes — all oplog-based (cloud sync is post-v1)
- **History** — Append-only operation log with hybrid logical clocks
- **Tables & Field Mappings** — User-facing data model backed by entity/facet internals
- **Conflicts** — Detection, human-readable presentation, reversible resolution
- **Overlays** — Staging areas for safe experimentation and preview
- **Scripts** — User and module automation that emits auditable operations
- **Rules** — Query-to-action automation scoped to tables
- **Modules** — Schema registration, capability enforcement, cross-module bindings
- **Queries** — Declarative queries that respect mappings and overlays

```
+-----------------------------------------------------------------+
|                         User Interface                          |
|                   (Module-provided views)                       |
+-----------------------------------------------------------------+
                                |
+-----------------------------------------------------------------+
|                        Module Runtime                           |
|         Tables · Views · Scripts · Smart Fields                 |
+-----------------------------------------------------------------+
                                |
+-----------------------------------------------------------------+
|                            Core                                 |
|   Entity/Facet · Sync · Oplog · Conflicts · Rules              |
+-----------------------------------------------------------------+
                                |
+-----------------------------------------------------------------+
|                   SQLite + Blob Storage                         |
+-----------------------------------------------------------------+
```

---

## Data Model

### What Users See: Tables, Records, and Fields

Users interact with **tables**. Modules declare tables with schemas. A table is a named collection of records with typed fields — concepts people already know from Excel, FileMaker, and Lightwright.

| User/developer sees | System does internally |
|---|---|
| "Create a contact" | Create entity, attach Contact facet |
| "My contacts table" | Query: all entities with Contact facet |
| "Link attendees to contacts" | Map fields, attach Attendee facet to matching entities |
| "Jane is in Contacts and Attendees" | One entity, two facets |
| "Unlink attendees from contacts" | Detach facets, copy data to new standalone entities |

An entity in both the Contacts table and the Attendees table is both a contact and an attendee. There's no single canonical type. Entity "type" is derived from table membership — which facets are attached.

### What the Core Uses: Entities, Facets, and Edges

Under the hood, the system uses an entity/facet model that makes multi-module identity work. This is internal architecture — module developers need to understand it, but end users never see it.

| Term       | Definition                                                                   |
| ---------- | ---------------------------------------------------------------------------- |
| **Entity** | A thing with a stable ID. Just an identifier — all data lives in fields.     |
| **Facet**  | A module-defined grouping of fields. Attaching a facet adds a record to the corresponding table. |
| **Field**  | Key-value data attached to an entity. Either shared or namespaced.           |
| **Edge**   | A relationship between two entities, with optional properties.               |

No module owns an entity exclusively. Multiple modules can attach facets to the same entity.

### Fields: Shared and Namespaced

Fields live at the entity level. Two types of field keys:

| Type | Format | Example | Behavior |
|------|--------|---------|----------|
| **Shared key** | `name` | `name`, `email`, `phone` | Multiple modules can read/write |
| **Namespaced key** | `module.field` | `casting.agent_contact` | Private to that module |

### Per-Entity Table Membership

Table membership operates at the individual entity level, not just at the table level. This is important for real production scenarios.

**The cue scenario:**
- LX 11 (lighting cue, called by stage manager) — in both the Lighting Cues table AND the SM Cues table
- LX 11.1 (lighting cue, auto-follow) — in the Lighting Cues table ONLY
- SM Cue 15 (sound cue) — in the SM Cues table AND the Sound Cues table, NOT in Lighting Cues

What this means:

- Table-level linking ("all contacts are attendees") is a convenience shortcut
- Per-entity table membership is the fundamental mechanism
- Rules can automate per-entity membership: "Cues in Lighting table where `is_called==true` also appear in SM Cues table"
- Users can manually add/remove individual records from tables

### Separation of Responsibilities

Each layer has a single, clear responsibility:

| Layer               | Responsibility                                                          |
| ------------------- | ----------------------------------------------------------------------- |
| **Entities**        | Identity (this is a unique thing)                                       |
| **Fields**          | Data (shared keys for cross-module, namespaced for module-private)      |
| **Facets**          | Module-defined field groupings (internal — maps to table membership)    |
| **Tables**          | User-facing data model (collections of records with schemas)            |
| **Edges**           | Relationships with properties (how things relate)                       |
| **Rules**           | Automation (matching, table membership, computed values) scoped to tables |

---

## Cross-Module Identity

### The Problem

Modules are independent, and they should be. No module should depend on another module. They don't know about each other. But users need cross-module identity: the "Jane Doe" in my contacts should be the same "Jane Doe" in my schedule.

The tension:

- If we force modules to use standardized types (Person, Cue, Prop), we lock users into a rigid structure
- If modules define whatever they want, users have to wire everything manually

### The Solution: Suggested-Confirmed Field Mapping

Modules declare shared key *suggestions* in their manifests (developer intent), but mappings do not auto-activate. On module adoption or first table-linking, the system presents suggested field mappings based on shared key overlap. The user reviews and confirms each mapping. Confirmed mappings then behave identically to classic shared key semantics.

**How it works:**

1. Contacts module declares a `name` field. Scheduler module also declares a `name` field.
2. On adoption or first link, the system says: "Contacts and Scheduler both have a `name` field. Should these be the same data?"
3. User confirms or rejects each suggested mapping.
4. Confirmed mappings share data going forward. Rejected mappings stay independent.
5. Users can also create custom field mappings beyond what modules suggest.

**Templates** (e.g., a "Stage Management" starter workspace) can pre-confirm mappings for zero-friction onboarding.

**Why this approach:** The system is welcome to suggest potential matches, but it should always be up to the user to say "these tables/types are the same thing." Auto-binding risks false collisions and violates the principle of explicit user control.

**Table-linking compatibility** replaces kind-compatibility. The system warns on unlikely table combinations; the user decides. Dedup/matching rules are scoped to tables, not to a global type.

---

## Operations & History

### The Oplog

All mutations are recorded as operations in an append-only log. The oplog is the source of truth; current state is derived by replaying operations.

Operations are:

- Immutable once written
- Idempotent (safe to replay)
- Attributed to a user and timestamp

### Hybrid Logical Clocks (HLC)

We use HLCs for deterministic ordering across peers. HLCs combine wall-clock time with a logical counter, giving us:

- Causally consistent ordering
- No reliance on synchronized clocks
- Deterministic merge behavior

### Operations and Bundles

Mutations are field-level operations grouped into atomic bundles. A bundle is an all-or-nothing unit — every operation in the bundle succeeds together or fails together.

### Determinism Guarantee

Given the same operations in the same order, any peer will arrive at the same state. This is critical for trust: users need to know that what they see is what everyone else sees (after sync).

---

## Collaboration Model

### Sync Modes (V1)

V1 supports two sync modes, both using the same underlying oplog-based protocol:

| Mode | How it works | When to use |
|------|-------------|-------------|
| **LAN session** | Devices discover each other via mDNS and sync directly. No internet required. | On-site production networks, isolated subnets |
| **Offline** | No sync. Changes accumulate locally. Merge on reconnect. | Default state — everything works without a network |

**Post-v1:** Cloud server sync (a central Rust server that clients sync to via WebSocket) and automated leader election are deferred to post-v1.

**Key constraint:** Users on isolated networks (lighting ETCNet, sound network) without WAN access must still be able to sync on their local subnet. Network topology cannot be assumed.

**What stays the same across all modes:**
- Local-first: all data lives on device, always works offline
- Oplog-based sync: "send me all ops I don't have"
- HLC for deterministic ordering
- Conflict detection and resolution model

### LAN Sessions

In LAN mode, peers discover each other on the local network via mDNS and sync directly. The first to host becomes the **leader**. The leader sequences operations to ensure consistency.

Leadership is a transport role, not a permission escalation. The leader doesn't have special powers over the data. If the leader disconnects, a new leader is elected. Everyone else keeps working.

### Network Partitions

Teams can work independently and merge later.

Example: The sound team works Saturday. The lighting team works Sunday. Monday, everyone reconnects. The system merges both timelines, surfacing conflicts where they edited the same things.

This is intentional. We want to support "everyone goes home, works on their own, comes back and syncs" without requiring cloud infrastructure.

---

## Conflict Resolution

### When Conflicts Occur

A conflict happens when two peers edited the same field while disconnected. Fields with confirmed mappings across modules are the same field — if `contacts.name` and `scheduler.display_name` are mapped, editing either creates a potential conflict.

### How Conflicts Are Presented

N-way conflicts are described in domain language, not database terms:

> "Jane Doe's call time was edited by Alex (18:00) and Jordan (18:30) since last sync. Choose a value."

Users see what changed, who changed it, and what the options are.

### How Conflicts Are Resolved

- User explicitly picks a resolution
- The resolution is recorded as an operation (auditable)
- Resolutions can be revisited and changed later
- Nothing is ever silently lost
- CRDTs handle text fields and ordered lists automatically (no user intervention needed)

---

## Smart Fields

### Smart Fields (UI Concept)

Smart Fields are a UI concept, not core architecture. Every field in the system supports modes, switchable through a popover:

- **Discrete** — Plain value (default)
- **Reference** — Points to a field on another entity (edit-through)
- **Query** — Dynamic expression producing a value or set (post-v1; requires expression language)

For V1, Smart Fields support Discrete and Reference modes. The Query mode depends on the expression language, which is deferred to post-v1.

The core must support field references for V1. "Smart Fields" as a unified concept lives in the UI/view layer. Module developers get Smart Field rendering for free from the core UI components.

---

## Staging Overlays

Staging overlays are temporary, non-canonical layers of operations that enable safe experimentation and preview. Overlays answer: **"Show me what this will do before it becomes real."**

### Core Concept

An overlay is an isolated workspace where users can:

- Make changes without affecting canonical state
- Preview bulk operations, imports, or transforms
- Experiment with "what-if" scenarios
- Stage data entry before committing

Overlays behave like canonical state for all read operations, but nothing syncs or becomes permanent until explicitly committed.

### Key Semantics

- **Isolation** — Overlay operations do not affect canonical history until committed
- **Atomic actions** — Commit all or discard all (no partial commits)
- **Knockout** — Remove specific operations before commit to exclude them
- **Safe discard** — Discarding an overlay affects no canonical state
- **Persistence** — Overlays persist across app restarts (no auto-expiry)

Scripts, imports, and manual experimentation all use overlays as their staging mechanism.

---

## Scripts and Automation

Scripts are the automation layer for Openprod. They enable both module developers and end users to create powerful, flexible workflows that emit auditable operations.

**Philosophy:** Modules provide schema, UI, and scripts. Users create scripts to automate tasks, trigger behaviors, and build complex functionality without needing to create modules.

All scripts are written in **Lua 5.4**, chosen for maturity (30+ years), native async via coroutines, lightweight runtime (~300KB), cross-platform support (desktop, mobile, web via WASM), and approachable syntax.

Scripts execute in two modes for V1:

- **Manual** — User-triggered, runs in an overlay for preview before commit
- **On-change** — Triggered by data changes, runs automatically based on user-configured triggers

**Post-v1:** Background mode (long-running scripts for OSC listeners, file watchers, etc.) is deferred to post-v1.

All script output is subject to normal conflict detection.

---

## Module System

### Philosophy

- **Independent**: Every module must be useful on its own
- **No hard dependencies**: Modules never assume other modules exist
- **Opt-in interoperability**: Cross-module features emerge through field mappings and user-defined rules, not code coupling

### Installation vs. Adoption

| Action      | Scope     | Effect                                       |
| ----------- | --------- | -------------------------------------------- |
| **Install** | Per-user  | Module UI available locally                  |
| **Adopt**   | Workspace | Module schema shared with all collaborators  |

### Module Anatomy

Modules can provide:

- **Schema** (TOML) — Declares tables, field types, and field mapping suggestions
- **Views** (TypeScript) — UI components for viewing and editing data
- **Scripts** (Lua) — Automation, imports, exports, and compute tasks

### Configuration Hierarchy

Configuration cascades from module defaults to workspace overrides:

```
Module defaults
    | (overridden by)
Workspace config
    | (overridden by)
Per-entity overrides (rare)
```

---

## V1 Foundation Primitives

These 13 primitives form the complete v1 architecture. Everything else is built on top or deferred:

1. **Entity/facet model** — Internal architecture. Entities with facets, field namespacing.
2. **Tables + field mappings** — User-facing data model. Modules declare tables, users link and map fields.
3. **Oplog + HLC** — Append-only operation log with hybrid logical clocks. Source of truth.
4. **Operations and bundles** — Field-level mutations grouped into atomic bundles.
5. **Conflict detection and resolution** — Scalar conflicts surfaced for user resolution. CRDTs for text and ordered lists.
6. **Overlays** — Staging areas for preview before commit. Used by scripts, imports, and manual experimentation.
7. **Lua scripting** — Business logic, automations, external I/O. Manual and on-change modes. (Background mode is post-v1.)
8. **Rules engine** — Query-to-action automation scoped to tables. Record matching, table membership, and computed values.
9. **Module system** — Modules as packages: table schema (TOML) + views (TypeScript) + scripts (Lua).
10. **Sync** — LAN discovery + offline. Oplog-based, CRDT-enhanced. (Cloud sync is post-v1.)
11. **Edges/relationships** — First-class directed relationships with properties. Ordered edges for lists.
12. **Blobs/assets** — Content-addressed immutable file storage (BLAKE3 hashing) for PDFs, images, CSVs.
13. **Actor identity** — Ed25519 keypair identity for oplog attribution, operation signing, and conflict context.

---

## Non-Goals

Things we will **not** do:

| We won't do this                        | Why                                                                |
| --------------------------------------- | ------------------------------------------------------------------ |
| Hidden implicit computation             | All computed values are user-configured, visible, and auditable. The system does not compute values unless the user has explicitly set up an expression, reference, or rule. There are no hidden formulas or implicit calculations. |
| Hidden automated mutations              | All automated mutations are user-configured, visible, and auditable. Every automated action can be traced to a rule or trigger that the user explicitly created or approved. No automation runs without user consent. |
| Hidden coupling between modules         | All interoperability must be visible and explainable               |
| Require a central server                | Local-first by default; cloud is optional                          |
| Replace every production tool           | We're a collaboration substrate, not an opinionated app            |
| Silently merge conflicting edits        | Users must see and resolve conflicts explicitly                    |
| Auto-resolve scalar conflicts           | All scalar resolutions require explicit user action (CRDTs handle text and lists) |

The distinction in the first two rows is **explicit vs. implicit**, not "no computation" or "no automation." Expressions, Smart Fields, rules, and on-change scripts all compute and mutate data — but only because the user set them up, and the user can always see what's happening.

---

## Future Considerations

These features are architecturally compatible but deferred to post-v1:

- **Expression language** — Lightweight formula language for field-level data transforms and queries. Enables Smart Field query mode and computed values. Deferred until the core is stable.
- **Cloud sync** — A central Rust server (self-hosted or provider-hosted) that clients sync to via WebSocket. V1 supports LAN and offline sync only.
- **Background scripts** — Long-running Lua scripts (OSC listeners, file watchers) using coroutines with Rust's tokio runtime. V1 supports manual and on-change script modes only.
- **Permissions & roles** — Role-based access control (Viewer/Editor/Admin). V1 has no role enforcement; everyone can edit. Permissions are deferred to post-v1.
- **Proposals** — Non-authoritative suggested changes visible to collaborators. Can be layered on top of overlays later.
- **Approval workflows** — Multi-party approval with quorum, expiration, delegation. Enterprise-level complexity that theatrical productions rarely need in software.
- **Schema evolution** — Formal migration system with semantic versioning. For v1, schema changes are handled manually. The oplog records module versions on operations, so the data for future migration support exists.
- **Snapshots & garbage collection** — Oplog segmentation, hash chains, and GC are performance optimizations. Not needed until the oplog grows large enough to matter.
- **Wire format optimization** — Use a simple serialization format (MessagePack or JSON) for v1. Optimize later if performance requires it.

Specs for these features exist in the archive for future reference.

---

## Technology Choices

| Component      | Technology        | Rationale                                        |
| -------------- | ----------------- | ------------------------------------------------ |
| Core engine    | Rust              | Performance, safety, single-binary distribution  |
| Storage        | SQLite (WAL mode) | Battle-tested, embeddable, excellent tooling     |
| Frontend       | Electron + TypeScript | Desktop app, module views                    |
| Scripting      | Lua 5.4 (mlua)   | Business logic, automations                      |
| Module schemas | TOML              | Human-readable table/field declarations          |
| Module views   | TypeScript        | UI components registered with view system        |
| Sync transport | WebSocket         | LAN (cloud server post-v1)                       |
| LAN discovery  | mDNS              | Zero-config local network peer finding           |
| CRDTs          | Yrs (recommended) | Text fields, ordered lists                       |

---

## Open Questions

These areas still need input:

1. **Cloud server protocol** — Exact relay and persistence protocol for the cloud sync target (post-v1).
2. **Expression language design** — What syntax and capabilities should the expression language support? (Post-v1, but design work can begin early.)

---

## Project Status

**Where we are:** Design phase with a rough prototype. The prototype proves the core ideas work, but it's not anywhere near production-ready.

**What exists:** A working-ish implementation of HLC/sync/module loading, relationships, queries. It's messy and needs rewriting, but it demonstrates feasibility.

**What's needed:** Help finalizing the architecture, especially around LAN sync and the rules engine. Then: a clean implementation of the core.

---

## Glossary

| Term                | Definition                                                                 |
| ------------------- | -------------------------------------------------------------------------- |
| Entity              | A thing with a stable ID; just an identifier (internal concept)            |
| Facet               | A module-defined grouping of fields; maps to table membership (internal concept) |
| Table               | A user-facing named collection of records with a schema                    |
| Record              | A single entry in a table (corresponds to an entity with a facet)          |
| Field               | Key-value data on an entity; either shared key or namespaced key           |
| Shared Key          | A field key accessible by multiple modules (e.g., `name`, `email`)         |
| Namespaced Key      | A module-private field key (e.g., `contacts.status`)                       |
| Field Mapping       | A confirmed link between fields across tables/modules — same data          |
| Edge                | A relationship between two entities                                        |
| Rule                | A query-to-action automation, scoped to a table                            |
| Expression          | A formula that computes a field value from other data                       |
| Smart Field         | UI concept: a field that can be discrete, reference, or query mode         |
| Staging Overlay     | A temporary, non-canonical layer of operations                             |
| Oplog               | Append-only log of all operations; the source of truth                     |
| HLC                 | Hybrid Logical Clock; provides deterministic ordering across peers         |
| Bundle              | An atomic group of operations that commit together                         |
| Module              | A package providing table schema, views, and/or scripts                    |

---

## Appendix: Scenario Walkthroughs

### Scenario A: Two departments work offline, then sync

1. Lighting team works Saturday, makes 20 cue changes
2. Sound team works Sunday, makes 15 cue changes
3. Monday, both teams connect to the same session
4. System compares oplogs, identifies 3 conflicts (same cues edited by both)
5. Conflicts surface with clear descriptions
6. Stage manager resolves each conflict
7. All peers now have identical state

### Scenario B: User links records across modules

1. Contacts module has a Contact with `name: "John Smith"`
2. Schedule module has an Attendee with `name: "J. Smith"`
3. On first table-linking, system suggests: "Contacts and Schedule both have a `name` field. Should these be the same data?"
4. User confirms the field mapping
5. User recognizes these two records are the same person but names don't match exactly
6. User manually merges — one entity now has both Contact and Attendee facets
7. Conflict on `name` field: "John Smith" vs "J. Smith"
8. User resolves, choosing "John Smith"
9. Both modules now see "John Smith"

### Scenario C: Import with overlay preview

1. Stage manager imports a CSV of 50 cast members
2. Import runs in a staging overlay (default behavior)
3. UI shows: "This will create 47 new Contacts. 3 match existing records."
4. SM reviews, spots "John Smithh" typo, fixes it directly in overlay
5. SM clicks "Commit" — all 50 operations apply atomically
6. If SM had spotted a bigger problem, they could discard the entire import

### Scenario D: Per-entity table membership (the cue scenario)

1. Lighting designer creates cues LX 1 through LX 20 in the Lighting Cues table
2. A rule is configured: "Cues in Lighting table where `is_called==true` also appear in SM Cues table"
3. LX 11 is marked `is_called: true` — it automatically appears in the SM Cues table
4. LX 11.1 (auto-follow) stays in Lighting Cues only — stage manager doesn't need to see it
5. Sound designer creates SND 5, marks it as called — it appears in both Sound Cues and SM Cues
6. Stage manager now has a unified cue list of all called cues across departments

### Scenario E: Script overlay with canonical drift

1. User is in staging mode, editing cue timings
2. User runs "Generate rehearsal schedule" script
3. Script completes — notification: "Schedule generator finished — 15 events staged"
4. User clicks "Stash current and view" to review script output
5. While reviewing, a peer syncs — one event's room changed
6. Badge appears: "Room changed to 'Studio B' while you were reviewing"
7. User clicks "Use Canonical" to accept the peer's change
8. User commits the remaining 14 events
9. User recalls their stashed overlay to continue editing cue timings

---

## Related Documents

| Document | Purpose | Audience |
|----------|---------|----------|
| [FUNCTIONALITY.md](FUNCTIONALITY.md) | *What* users experience, how features work | End users |
| [INVARIANTS.md](INVARIANTS.md) | Constraints that must always hold | Implementers |
| [spec/](spec/README.md) | *How* to build each subsystem | Implementers |
