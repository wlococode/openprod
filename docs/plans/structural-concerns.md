# Structural Concerns — Deferred Items

Items identified during the edge property restructuring review that are out of scope for the current phase but should be addressed in future work.

## 1. Schema Enforcement (Phase 5)

Local-write-time validation of field types and required fields against module manifests. Sync-ingested operations bypass validation to preserve convergence (an invalid-but-signed op must still be stored and replayed). Requires module manifest loading infrastructure.

## 2. Oplog Streaming (Post-v1)

Change `get_ops_canonical()` and `rebuild_from_oplog()` to streaming iteration instead of collecting all operations into memory. Not urgent until the oplog exceeds ~100K operations.

## 3. State Hash Verification (Phase 4 — Sync)

Implement `state_hash` for convergence verification between peers. After sync, both peers compute a deterministic hash of their materialized state; divergence indicates a bug in LWW materialization or missed operations.

## 4. Entity Identity Modeling Guidance (Adopter Documentation)

The entity model handles contextual identity via edges + facets. Document recommended patterns for entertainment production data: people appearing in multiple roles across projects, equipment shared between departments, etc.

## 5. Concurrent Engine Access (Phase 5 — Scripts)

Overlays establish the concurrency boundary for script execution. Script overlays provide independent write paths that are later committed or discarded. The engine currently assumes single-writer access; overlay commit must be serialized.

## 6. Composite Field Types (Open Question)

Entertainment data frequently needs structured values (addresses, rate tables, multi-currency amounts). Current options:
- **Nested entities via edges** — works today but verbose for simple structured data
- **JSON FieldValue variant** — queryable with JSON path expressions but loses type safety
- **Structured field type** — new FieldValue variant with schema-validated sub-fields

No decision yet. Needs real-world usage data before committing to an approach.
