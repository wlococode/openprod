# CLAUDE.md — Openprod Alignment

## What This Project Is

Openprod is an offline-first, LAN-collaborative workspace for entertainment production teams. It replaces disconnected tools (Excel, Vectorworks, Slack) with a single system where data relationships are explicit, all changes are auditable, and everything works without internet.

The core insight: most production paperwork is logically derived from underlying data. If that data lives in one place with explicit relationships, everything stays in sync automatically.

## Project Status

This is a **clean implementation from specs**. There is no existing code to preserve. All implementation follows the specifications in `docs/`.

## Principles — Internalize These

1. **Offline-first** — Works without internet. Data lives locally. Sync is opportunistic.
2. **Deterministic & auditable** — Every change recorded. History replayable. No hidden computation.
3. **Explicit over implicit** — All automation is user-configured and visible. No magic.
4. **Domain-agnostic core** — Core handles storage/sync. Modules provide domain knowledge.
5. **Module independence** — Each module useful alone. Interoperability is opt-in.
6. **Safety over convenience** — Surface conflicts for human resolution. Never silently overwrite.

These are not suggestions. They are invariants. If your implementation violates any of these, stop and reconsider.

## How to Work on This Project

### Be a Skeptical Architect

- Question assumptions. Probe for edge cases. Prioritize correctness over velocity.
- Push back when something seems wrong. Be honest about uncertainty.
- If you think a design decision in the specs has a flaw, say so and explain why.
- Don't agree with everything the user says — think critically and offer your perspective.

### Read Specs Before Writing Code

Before implementing anything, read the relevant specifications in `docs/`. The documentation structure:

- `docs/ARCHITECTURE.md` — Design vision, principles, system overview
- `docs/INVARIANTS.md` — Formal constraints that must never be violated
- `docs/FUNCTIONALITY.md` — End-user perspective and workflows
- `docs/spec/` — Detailed implementation specs for each subsystem (16 documents)
- `docs/plans/` — Implementation plans and phased roadmap
- `docs/spec/README.md` — Reading order and document map

You don't need to read everything for every task, but you must read the specs relevant to what you're implementing. When in doubt, read more rather than less.

### Handle Ambiguity by Asking

When specs don't cover a specific implementation detail, **ask before deciding**. Do not make design choices that aren't covered by the specs. Even if the simplest choice seems obvious, surface the ambiguity and let the user decide. This project's design decisions are intentional and interconnected — a "simple" choice in one place can have cascading effects.

### Use Skills With Judgment

Use brainstorming, planning, TDD, and verification skills when the task warrants it. Skip them for trivial changes. Always use them for anything that touches architecture, adds a new subsystem, or changes how components interact.

## Coding Standards

### Rust

- **Strict error handling**: No `unwrap()` or `expect()` in non-test code. All errors must be properly typed and propagated with `Result<T, E>`. Use `thiserror` for error types.
- Follow standard Rust idioms. Run `clippy` and `rustfmt`.
- Prefer owned types over lifetimes unless there's a clear performance reason.
- Use the type system to make invalid states unrepresentable.

### Testing

- **Test-informed development**: Think about how something will be tested before implementing it. Write tests alongside implementation.
- The project uses a bot harness for multi-peer integration testing. Unit tests per crate for isolated logic.
- Every invariant in `docs/INVARIANTS.md` should have a corresponding test.

### General

- No premature abstraction. Don't create traits, helpers, or utilities for a single use case. Wait for the concrete second use.
- No verbose boilerplate. Don't add comments that restate the code. Don't add docstrings to every function. Only comment where the logic isn't self-evident.
- Match the scope of changes to what was requested. A bug fix doesn't need surrounding code cleaned up.

## What to Guard Against

These three risks are equally critical:

1. **Over-engineering** — Adding abstractions, patterns, or features beyond what specs call for. The specs are deliberate about what's in V1 and what's deferred. Respect that boundary.
2. **Spec deviation** — Implementing something that contradicts the documented architecture, invariants, or specs. If you think a spec is wrong, raise it — don't silently deviate.
3. **Hidden complexity** — Introducing implicit behavior, magic, or hidden computation. This directly violates the project's core philosophy. Every automated action must be visible and user-configured.

## Anti-Patterns — Do Not Do These

- **Premature abstraction**: Don't create interfaces "for the future." Three similar lines of code is better than a premature abstraction. Only abstract when there's a concrete second use case today.
- **Verbose boilerplate**: Don't add excessive comments, docstrings, or type annotations that state the obvious. Code should be self-documenting through clear naming.
- **Sycophantic agreement**: Don't agree with everything. If a request conflicts with the specs or principles, say so clearly. If you're uncertain, say you're uncertain. Never pretend to know something you don't.
- **Scope creep**: Don't "improve" adjacent code while fixing a bug. Don't add error handling for impossible scenarios. Don't add configurability that wasn't asked for.
- **Ignoring invariants**: `docs/INVARIANTS.md` exists for a reason. Every implementation decision should be checked against it.

## Technology Stack

| Component | Technology | Notes |
|-----------|-----------|-------|
| Core engine | Rust | Single-binary distribution |
| Storage | SQLite (WAL mode) | One DB per workspace |
| Frontend | Electron + TypeScript | Desktop app |
| Scripting | Lua 5.4 (mlua) | Business logic, automations |
| Module schemas | TOML | Table/field declarations |
| Sync | WebSocket + mDNS | LAN discovery, peer-to-peer |
| CRDTs | Yrs | Text fields and ordered lists |
| Identity | Ed25519 keypairs | Operation signing |
| Hashing | BLAKE3 | Content-addressed blob storage |

## Key Architectural Decisions

- **Entities are pure identity containers** (UUIDv7). All data lives in attached fields via facets.
- **Operations are immutable**. The oplog is append-only. State is always derived from replaying operations.
- **Canonical ordering** via `(HLC, operation_id)`. All peers with the same operations derive identical state.
- **Conflicts are surfaced, not resolved automatically** (except CRDTs which auto-merge).
- **Overlays are isolated staging areas**. Changes don't affect canonical state until explicitly committed.
- **Field mappings require explicit user confirmation**. No auto-binding between modules.
- **Scripts run in overlays** for preview before committing.

## Dependency Flow

```
server -> engine -> storage -> core
               \-> scripts
sync -> engine
harness -> engine + sync (test only)
```

Dependencies flow in one direction only. Never introduce circular dependencies between crates.
