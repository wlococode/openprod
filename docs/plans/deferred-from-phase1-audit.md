# Deferred Items from Phase 1 Audit

Items identified during Phase 1 code audit that were deferred for later phases.

---

## Signature/Hlc Serde Optimization

`Signature` and `Hlc` serde deserialize through an intermediate `Vec<u8>` allocation. For hot paths (bulk oplog reads), this could be optimized with a visitor pattern that reads directly into fixed-size arrays. Not urgent — only matters at scale.

**Files:** `crates/core/src/hlc.rs`, `crates/core/src/identity.rs`

## ~~Edge Query Deduplication~~ (Resolved)

~~`get_edges_from` and `get_edges_to` in `SqliteStorage` share ~60 lines of identical row-parsing logic, differing only in the WHERE clause.~~

**Resolved:** Extracted `extract_edge_row` and `parse_edge_row` helpers during Phase 2 Batch 1 audit. All three edge query methods (`get_edges_from`, `get_edges_to`, `get_edge`) now use shared helpers.

## query_and_then Migration

Several `query_map` closures in `SqliteStorage` tunnel `StorageError` through `OpaqueStorageError` because `query_map` requires `rusqlite::Error`. Rusqlite's `query_and_then` accepts `Result<T, E>` directly and would eliminate this wrapper. Migrate when touching these query methods.

**Files:** `crates/storage/src/sqlite.rs`

## get_bundle Method

`read_bundle` exists as a private helper in `sqlite.rs` but isn't exposed through the `Storage` trait. Add a `get_bundle(bundle_id) -> Result<Option<Bundle>>` method to the trait when needed (likely for undo/redo or sync).

**Files:** `crates/storage/src/traits.rs`, `crates/storage/src/sqlite.rs`

## HlcClock::new() Initial State

`HlcClock::new()` initializes with `wall_ms: 0, counter: 0`. This means the first `tick()` always jumps to physical time (correct behavior), but `receive()` called before `tick()` accepts any non-future remote timestamp. Consider initializing from `physical_now()` if this becomes a practical concern.

**Files:** `crates/core/src/hlc.rs`

## bundle_type Persistence Roundtrip Test

No test verifies that `BundleType` survives the `as i32` / `match bundle_type_int` roundtrip in SQLite. Add a test that stores and retrieves each `BundleType` variant to catch mapping drift.

**Files:** `crates/harness/tests/` or `crates/storage/src/sqlite.rs` (unit test)

---

## Phase 1 Schema Simplifications

Known deviations between `docs/spec/sqlite-schema.md` and `crates/storage/src/schema.rs`, intentionally deferred:

### actors table

Implementation has only `actor_id`, `display_name`, `first_seen_at`. Spec includes additional columns: `device_name`, `first_seen_in_bundle` (with FK to bundles), `revoked_at`, `revoked_by`, `revoked_in_bundle`. Add when implementing actor revocation and identity management.

**Files:** `crates/storage/src/schema.rs`

### facets table — missing source_id

Implementation has `source_type TEXT NOT NULL DEFAULT 'user'` but no `source_id TEXT` column. Spec includes `source_id` for tracking which rule or link triggered the facet attachment. Add when implementing rule impact analysis.

**Files:** `crates/storage/src/schema.rs`
