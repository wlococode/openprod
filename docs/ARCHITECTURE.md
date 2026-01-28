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

Openprod is a collaboration system for production teams. The core handles storage, syncing, and conflict resolution. Plugins provide the actual functionality—contacts, schedules, cue sheets, paperwork layouts.

The key insight: most production paperwork is _derived_ from the same underlying information in a pretty deterministic way. If the data lives in one place and relationships are explicit, everything can stay in sync automatically.

A stage manager shouldn't have to think "what time is this person's first call?" for every company member. The system knows who's in which scenes/segments. It knows the schedule. The call time is a query, not a calculation someone does by hand.

**Think:** Obsidian's plugin model meets Git's offline-first collaboration, built for production workflows.

---

## Core Principles

These guide every design decision:

1. **Offline and local-first.** No subscription required. No cloud dependency. Your data lives on your machine and works without internet.

2. **Deterministic and auditable.** Every change is recorded. History is replayable. You can always see what changed, when, and by whom.

3. **Explicit over implicit.** The system never mutates data on its own. All changes come from user actions or explicitly-triggered jobs. No surprise behavior.

4. **Domain-agnostic core.** The core knows about entities, relationships, and sync—not about lighting cues or call times. All domain knowledge lives in plugins.

5. **Plugins are independent.** Each plugin must be useful on its own. Interoperability between plugins is opt-in, not assumed.

6. **Safety over convenience.** When there's a conflict, surface it for human resolution. Never silently overwrite someone's work.

---

## System Overview

The core handles:

- **Storage** — Entity/facet/edge graph in SQLite (WAL mode), asset blobs on disk
- **Sync** — Peer-to-peer replication with leader election, network partition tolerance
- **History** — Append-only operation log with hybrid logical clocks
- **Identity** — Entity merging, deduplication, cross-plugin references
- **Conflicts** — Detection, human-readable presentation, reversible resolution
- **Plugins** — Schema registration, capability enforcement, cross-plugin bindings
- **Queries** — Declarative queries that respect bindings and permissions

```
┌─────────────────────────────────────────────────────────────────┐
│                         User Interface                          │
│                   (Plugin-provided views)                       │
└─────────────────────────────────────────────────────────────────┘
                                │
┌─────────────────────────────────────────────────────────────────┐
│                        Plugin Runtime                           │
│             Schema · Views · Jobs · Capabilities                │
└─────────────────────────────────────────────────────────────────┘
                                │
┌─────────────────────────────────────────────────────────────────┐
│                            Core                                 │
│   Storage · Sync · Oplog · Identity · Conflicts · Queries       │
└─────────────────────────────────────────────────────────────────┘
                                │
┌─────────────────────────────────────────────────────────────────┐
│                   SQLite + Blob Storage                         │
└─────────────────────────────────────────────────────────────────┘
```

---

## Data Model

### Entity–Facet–Edge

The core uses a graph-based data model:

| Term       | Definition                                                                   |
| ---------- | ---------------------------------------------------------------------------- |
| **Entity** | A thing: a person, a cue, a prop, a document. Entities have stable IDs.      |
| **Facet**  | A set of fields attached to an entity by a plugin. Plugins own their facets. |
| **Edge**   | A relationship between two entities, with optional properties.               |

No plugin owns an entity exclusively. Multiple plugins can attach facets to the same entity.

### Example: A Person

```
Entity: person_jane_doe
├── Facet: contacts.person
│   ├── name: "Jane Doe"
│   ├── email: "jane@example.com"
│   └── phone: "555-1234"
├── Facet: crew.member
│   ├── role: "Stage Manager"
│   ├── department: "Production"
│   └── call_time: "18:00"
└── Edges:
    ├── assigned_to → show_hamlet
    └── member_of → department_production
```

The Contacts plugin sees Jane as a contact. The Crew plugin sees her as a crew member. Both are views of the same underlying entity.

---

## Concepts & Bindings

**This is the part of the architecture I'm least certain about. I have a working model, but I'd love input on whether there's a better approach.**

### The Problem

Plugins are independent, and they should be. No plugin should be dependent on another plugin. They don't know about each other. But, users need cross-plugin identity: the "Jane Doe" in my contacts should be the same "Jane Doe" in my schedule.

The tension:

- If we force plugins to use standardized types (Person, Cue, Prop), we lock users into a rigid structure
- If plugins define whatever they want, users have to wire everything manually

### Current Approach: Concepts and Bindings

A **Concept** is a user-defined semantic anchor. It says "this is what a Person means in this workspace."

A **Binding** maps plugin fields to Concept fields. When a user draws a binding from `contacts.person.name` and `schedule.attendee.name` to a concept field `Person.name`, they're saying "these represent the same thing."

The core then handles:

- Recognizing when two entities refer to the same real-world thing
- Merging/deduplicating as needed
- Keeping references valid across plugins

### Open Questions

Where should the "canonical" data live when two plugins are bound?

- Option A: One plugin retains the data, the other holds references
- Option B: A concept-level entity exists outside both plugins; both reference it

How do we make this intuitive for non-technical users? Schema changes are dangerous and confusing.

Is there prior art I should study? (RDF, Notion relations, Airtable linked records, contact merging in email clients?)

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

### Determinism Guarantee

Given the same operations in the same order, any peer will arrive at the same state. This is critical for trust: users need to know that what they see is what everyone else sees (after sync).

---

## Collaboration Model

### Local / Offline Mode

Single user, no network. Everything works. This is the default state.

### LAN Collaboration

Peers discover each other on the local network. The first to host becomes the **leader**. The leader sequences operations to ensure consistency.

Leadership is a transport role, not a permission escalation. The leader doesn't have special powers over the data.

If the leader disconnects, a new leader is elected. Everyone else keeps working.

### Network Partitions

Teams can work independently and merge later.

Example: The sound team works Saturday. The lighting team works Sunday. Monday, everyone reconnects. The system merges both timelines, surfacing conflicts where they edited the same things.

This is intentional. We want to support "everyone goes home, works on their own, comes back and syncs" without explicitly requiring cloud infrastructure.

### Session Lifecycle

- **Host** — Start a collaborative session
- **Join** — Connect to an existing session
- **Leave** — Disconnect (your data stays local)
- **End** — Close the session entirely

No surprise syncs. Users control when merging happens.

---

## Conflict Resolution

### When Conflicts Occur

A conflict happens when two peers edited the same semantic field while disconnected. "Semantic" means we respect bindings—if `contacts.person.name` and `schedule.attendee.name` are bound, editing either creates a potential conflict.

### How Conflicts Are Presented

N-way conflicts are described in domain language, not database terms:

> "Jane Doe's call time was edited by Alex (18:00) and Jordan (18:30) since last sync. Choose a value."

Users see what changed, who changed it, and what the options are.

Conflicts are non-blocking by nature, but interface should encourage users to resolve them. To ensure replica parity, users could see LWW value by default until conflict is resolved.

### How Conflicts Are Resolved

- User explicitly picks a resolution
- The resolution is recorded as an operation (auditable)
- Resolutions can be revisited and changed later
- Nothing is ever silently lost

### Example

1. Alex (offline) changes Cue 42's start time to 10:30
2. Jordan (offline) changes Cue 42's start time to 10:45
3. They reconnect
4. System shows: "Cue 42 start time: Alex set 10:30, Jordan set 10:45. Which is correct?"
5. User picks Jordan's version
6. Operation recorded: "Conflict resolved: Cue 42 start time = 10:45 (chose Jordan's edit over Alex's)"

---

## Plugin System

### Philosophy

- **Independent**: Every plugin must be useful on its own
- **No hard dependencies**: Plugins never assume other plugins exist
- **Opt-in interoperability**: Cross-plugin features emerge through Concepts and Bindings, not code coupling

### Installation vs. Adoption

| Action      | Scope     | Effect                                      |
| ----------- | --------- | ------------------------------------------- |
| **Install** | Per-user  | Plugin UI available locally                 |
| **Adopt**   | Workspace | Plugin schema shared with all collaborators |

A lighting designer can install plugins the stage manager doesn't need. But if the lighting plugin's schema should be shared workspace-wide, it needs to be adopted.

### Plugin Anatomy

Plugins can provide:

- **Schema** (TOML) — Declares facet types, edge types, and optional Concept bindings
- **Views** (TypeScript) — UI components for viewing and editing data
- **Jobs** (Rust) — Compute-intensive tasks like PDF generation

### Capabilities

Plugins request host capabilities:

- Filesystem access
- Network access
- MIDI/OSC output
- etc.

Capabilities are granted per-user and enforced by the core. A plugin can't access the filesystem unless you've allowed it.

---

## Jobs

Jobs handle compute-intensive tasks: generating PDFs, bulk transformations, complex calculations.

### Safety Model

1. **Jobs are planners, not executors** — A job reads data and produces a bundle of operations. It never writes directly.
2. **Preview before apply** — Users see what a job will do before it happens.
3. **Deterministic** — Same inputs, same outputs. Job results are replayable.

This means jobs are safe to experiment with. You can run a job, see what it would do, and cancel if it's wrong.

---

## Query & View System

### Structured Queries

Queries are declarative and binding-aware. If you query for "all people," the system knows to include both `contacts.person` and `schedule.attendee` entities (if they're bound via a Concept).

### Derived Views

Read-only views computed from the graph. Used for:

- Reports and paperwork
- Dashboards
- Cross-plugin summaries

Derived views are never stored—they're always computed fresh from source data.

---

## Practical Examples

These are the workflows I want to enable:

### Stage Manager: Rehearsal Scheduling

Today: SM manually calculates call times for 50 people by checking which scenes they're in, when those scenes rehearse, and adding buffer time. Takes hours.

With Openprod:

1. Import script, mark scene boundaries
2. Import contacts, assign people to scenes
3. Create schedule, drop in events like "Fight Call" with attendees = "people in Scene 2 where fight occurs"
4. System automatically derives each person's call time, break windows, and departure time
5. Changes to the schedule automatically update all derived times

### Stage Manager: Prompt Book Cue Integration

Today: SM manually copies cues from each department into their prompt book. When something changes, they update by hand. Errors happen.

With Openprod:

1. Each department enters their cues in their own plugin
2. SM's prompt book view queries all cues, displays them on the relevant script pages
3. When lighting updates Cue 42, the prompt book updates automatically
4. Standby calls are derived: "get all cues on this page and the next N pages"

### Lighting Designer: Patch and Focus

Today: LD exports CSV from Vectorworks, manually enters DMX addresses, patches console by hand.

With Openprod:

1. Import fixture data from Vectorworks/Lightwright
2. System calculates DMX universes and addresses based on position, type, rules
3. Send patch to console via OSC
4. Query console: "what lights aren't used in Cue 101?" "What color are my down pools in Scene 3?"

### Cross-Department: Notes

Today: Each department takes notes in their own format, exports PDFs, emails them out. Everyone has to open multiple files to see if anything affects them.

With Openprod:

1. Anyone can attach notes to any entity
2. Notes can have images, URLs, threads, status (open/resolved/on hold)
3. Personalized views: "show me notes tagged with my department"
4. Personalized emails: "Your notes for today: [relevant subset]"

---

## Non-Goals

Things we will **not** do:

| We won't do this                        | Why                                                     |
| --------------------------------------- | ------------------------------------------------------- |
| Background auto-mutations               | Implicit behavior erodes trust                          |
| Formula fields / live calculation rules | All transformations must be explicit and user-approved  |
| Hidden coupling between plugins         | All interoperability must be visible and explainable    |
| Require a central server                | Peer-to-peer by default; cloud is optional              |
| Replace every production tool           | We're a collaboration substrate, not an opinionated app |
| Silently merge conflicting edits        | Users must see and resolve conflicts explicitly         |

---

## Technology Choices

| Component     | Technology      | Rationale                                       |
| ------------- | --------------- | ----------------------------------------------- |
| Core          | Rust            | Performance, safety, single-binary distribution |
| Storage       | SQLite (WAL)    | Battle-tested, embeddable, excellent tooling    |
| Plugin UI     | TypeScript      | Familiar to web developers, good ecosystem      |
| Plugin Schema | TOML            | Human-readable, simple for non-programmers      |
| Sync Protocol | Custom over TCP | LAN-optimized, no cloud dependency              |

---

## Open Questions

These are areas where I need input:

1. **Concepts and Bindings** — Is my model for cross-plugin identity the right approach? Are there better patterns from RDF, linked data, or other systems I should study?

2. **Where does canonical data live?** — When two plugins bind to the same Concept, which one "owns" the data? Or should there be a concept-level entity outside both?

3. **Peer-to-peer replication** — What's the industry standard here? I've designed around leader election and HLC, but I don't know if there are better approaches.

4. **Plugin sandboxing** — How strict should isolation be? WASM? Process isolation? What's the right tradeoff between safety and capability?

5. **Schema evolution** — How do plugins handle breaking changes to their facet definitions?

6. **Peer discovery** — How do peers find each other on LAN? mDNS? Something else?

---

## Project Status

**Where we are:** Design phase with a rough prototype. The prototype proves the core ideas work, but it's not anywhere near production-ready.

**What exists:** A working-ish implementation of HLC/sync/plugin loading, relationships, queries. It's messy and needs rewriting, but it demonstrates feasibility.

**What's needed:** Help finalizing the architecture, especially around the Concept/Binding system and peer-to-peer replication. Then: a clean implementation of the core.

---

## Glossary

| Term         | Definition                                                          |
| ------------ | ------------------------------------------------------------------- |
| Entity       | A thing with a stable ID (person, cue, prop, etc.)                  |
| Facet        | A set of fields attached to an entity by a plugin                   |
| Edge         | A relationship between two entities                                 |
| Concept      | A user-defined semantic anchor for shared identity across plugins   |
| Binding      | A mapping from plugin fields to Concept fields                      |
| Oplog        | Append-only log of all operations; the source of truth              |
| HLC          | Hybrid Logical Clock; provides deterministic ordering across peers  |
| Redirect     | A pointer from a merged entity to its canonical version             |
| Canonical    | The authoritative version of a merged entity                        |
| Adoption     | Making a plugin's schema available workspace-wide                   |
| Capability   | A host feature (filesystem, network, etc.) that plugins can request |
| Job          | A compute task that produces operations without direct mutation     |
| Derived View | A read-only view computed from graph queries                        |

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

### Scenario B: User merges duplicate entities

1. Contacts plugin has "John Smith"
2. Schedule plugin has "J. Smith" as an attendee
3. User recognizes these are the same person
4. User merges them, choosing "John Smith" as canonical
5. System creates redirect from J. Smith → John Smith
6. All references to "J. Smith" now resolve to "John Smith"
7. Both plugins see the merged entity with all facets

### Scenario C: Conflict resolution flow

1. Alex changes Cue 42 timing to 10:30 (offline)
2. Jordan changes Cue 42 timing to 10:45 (offline)
3. Both reconnect
4. System detects conflict on Cue 42's timing field
5. UI shows: "Cue 42 timing edited by Alex (10:30) and Jordan (10:45)"
6. User picks Jordan's version
7. Resolution recorded as operation: chose 10:45
8. Later, user can view conflict history and change resolution if needed
