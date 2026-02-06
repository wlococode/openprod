# Data Model Specification

This document defines the data model for Openprod. It covers two layers: the **user-facing table model** (what module developers declare and users interact with) and the **internal entity/facet model** (the engine that powers cross-module identity and data sharing under the hood).

---

## Overview: Two Layers

| Layer | Audience | Concepts | Purpose |
|-------|----------|----------|---------|
| **Table model** | Module developers, users | Tables, records, fields, field mappings | Familiar spreadsheet-like interface for declaring and interacting with data |
| **Entity/facet model** | Core implementers | Entities, facets, namespaced fields, edges | Internal engine for cross-module identity, multi-table membership, and data unification |

Users see tables, records, and fields. Under the hood, each table is backed by a facet. A record in a table is an entity with that facet attached. An entity appearing in multiple tables has multiple facets. This mapping is transparent to users but essential for implementers.

**Layer mapping:**

| User/developer sees | System does internally |
|---|---|
| "Create a contact" | Create entity, attach Contact facet |
| "My Contacts table" | Query: all entities with Contact facet |
| "Link Attendees to Contacts" | Map fields, attach Attendee facet to matching entities |
| "Jane is in Contacts and Attendees" | One entity, two facets |
| "Unlink Attendees from Contacts" | Detach facets, copy data to new standalone entities |
| "Delete a record from Contacts" | Detach Contact facet (or delete entity if no other facets) |

---

## Tables (User-Facing Model)

Tables are the primary data model that module developers declare and users interact with. Every module exposes one or more tables. Users see tables as collections of records with typed columns.

### Table Declaration

Modules declare tables in their manifest (TOML):

```toml
[module]
name = "contacts"
version = "0.1.0"

[table.Contacts]
description = "People and organizations"

[table.Contacts.fields.name]
type = "string"
required = true
shared_key = "name"

[table.Contacts.fields.email]
type = "string"
shared_key = "email"

[table.Contacts.fields.phone]
type = "string"
shared_key = "phone"

[table.Contacts.fields.status]
type = "string"
# No shared_key: namespaced as "contacts.status"

[table.Contacts.fields.internal_notes]
type = "string"
private = true
# Explicitly namespaced: "contacts.internal_notes"
```

```toml
[module]
name = "lighting"
version = "0.1.0"

[table.LightingCues]
description = "Lighting cue list"

[table.LightingCues.fields.cue_number]
type = "number"
required = true

[table.LightingCues.fields.intensity]
type = "number"

[table.LightingCues.fields.color]
type = "string"

[table.LightingCues.fields.is_called]
type = "boolean"
default = true
```

### Table Semantics

- A table is a named, schema'd collection of records.
- Each table is backed by a facet internally. The facet name is derived from the module and table name (e.g., module `lighting`, table `LightingCues` produces facet `lighting.LightingCues`).
- Creating a record in a table creates an entity and attaches the corresponding facet.
- Deleting a record from a table detaches the facet. If the entity has no remaining facets, the entity itself is deleted.
- A record's columns are the table's declared fields, resolved through the field mapping layer.

### Per-Entity Table Membership

Table membership operates at the entity level, not at the table level. Individual records can appear in multiple tables independently.

**Example: Cue tables in a theatrical production**

```
LX 11   (lighting cue, called by SM)
  -> in LightingCues table
  -> in SMCues table

LX 11.1 (lighting cue, auto-follow, not called)
  -> in LightingCues table ONLY

SM Cue 15 (sound cue, called by SM)
  -> in SoundCues table
  -> in SMCues table
  -> NOT in LightingCues table
```

Internally:
- LX 11 is one entity with facets `lighting.LightingCues` and `sm.SMCues`.
- LX 11.1 is one entity with facet `lighting.LightingCues` only.
- SM Cue 15 is one entity with facets `sound.SoundCues` and `sm.SMCues`.

**Mechanisms for table membership:**

| Mechanism | Description |
|-----------|-------------|
| **Manual** | User adds or removes an individual record from a table |
| **Bulk link** | User links two tables ("all contacts are also attendees") as a convenience shortcut |
| **Rule-based** | Automated membership rule: "Cues in LightingCues where `is_called == true` also appear in SMCues" |

Rules can automate per-entity membership decisions. Users can always override by manually adding or removing individual records.

---

## Entity Model (Internal Layer)

Entities are the fundamental unit of identity in the system. Users never interact with entities directly; they interact with records in tables. Implementers must understand the entity layer.

**Anchor invariant:** An entity is pure identity. All data lives in fields attached to the entity.

### Core Semantics

- Entities have stable IDs (within workspace)
- An entity is a UUID with whatever fields have been attached
- Entities exist once created; fields come into existence when something writes them
- Entity type is derived from table membership (which facets are attached), not from any type field
- An entity in both the Contacts table and the Attendees table is both a contact and an attendee -- no single canonical type required
- Edges are explicit and typed relationships between entities
- Redirect resolution (for merged entities) is transparent to queries

### Entity Lifecycle

- Entity creation is an explicit `CreateEntity` operation (see [operations.md](operations.md))
- Entity deletion is an explicit `DeleteEntity` operation
- An entity exists once a `CreateEntity` operation has been applied
- An entity is deleted once a `DeleteEntity` operation has been applied
- Writing to a non-existent entity ID is an error
- Entity operations are grouped into bundles for atomicity
- Entity creation metadata provides audit trail (created_by, created_at)

### Entity ID Strategy

**Anchor invariant:** Entity IDs are UUIDv7, generated locally without coordination. IDs are globally unique, time-sortable, and do not require a central authority.

#### Format

Entity IDs use UUIDv7 (RFC 9562):

```
xxxxxxxx-xxxx-7xxx-yxxx-xxxxxxxxxxxx
         |    |
         |    +-- version nibble (7)
         +-- 48-bit Unix timestamp (milliseconds)
```

- **Canonical representation:** lowercase hyphenated string (36 characters)
- **Example:** `018f6b1c-2e4a-7d00-8abc-1234567890ab`
- **Storage:** May be stored as 128-bit binary internally; always serialized as canonical string in oplogs and APIs

#### Why UUIDv7

| Requirement | UUIDv7 | UUIDv4 | Verdict |
|-------------|--------|--------|---------|
| Offline generation | Yes | Yes | Tie |
| No coordination | Yes | Yes | Tie |
| Collision resistance | 2^74 random bits | 2^122 random bits | Both sufficient |
| Time-sortable | Yes (48-bit timestamp) | No | UUIDv7 wins |
| Index-friendly | Yes (monotonic prefix) | No (random scatter) | UUIDv7 wins |
| Privacy (creation time) | Leaks millisecond precision | No | UUIDv4 wins |

**Decision:** UUIDv7 for entities. The indexing and sorting benefits outweigh the timestamp privacy concern because:

1. Entity creation timestamps are already recorded in bundle metadata (`created_at`)
2. Time-ordered IDs improve B-tree locality for range queries
3. Debugging is easier when IDs roughly correspond to creation order
4. Privacy-sensitive contexts can use the `created_at` field's access controls

#### Generation Rules

- Each actor generates entity IDs locally using UUIDv7
- The timestamp component uses the local clock (not HLC)
- The random component uses a cryptographically secure random source
- Actors must never generate IDs on behalf of other actors

#### Collision Handling

Collisions are astronomically improbable (same millisecond + same 74 random bits). If detected:

1. **Detection:** Bundle application fails if `creates` lists an already-existing entity ID
2. **Resolution:** The creating actor must regenerate with a new ID and retry
3. **No silent merge:** Two entities with the same ID from different actors is a hard error, not a merge

In practice, collision probability is ~10^-23 per ID pair. A workspace generating 1 billion entities has ~10^-5 probability of any collision occurring.

#### Deterministic IDs

Deterministic (content-addressed) entity IDs are **not used** for entities because:

- Entities are mutable containers; their identity is not their content
- Two entities with identical initial fields are still distinct entities
- Content-addressing is used for blobs (see Assets & Blobs), not entities

However, specific **derived identifiers** may be deterministic:

```yaml
# Deterministic: blob hash (content-addressed)
blob_id: blake3(content)

# NOT deterministic: entity ID (identity-addressed)
entity_id: uuidv7()
```

### Entity Deletion Semantics

**Anchor invariant:** Deleting an entity never deletes other entities implicitly; only relationships (edges) and facets are affected.

- Deleting an entity removes that entity and all its attached facets
- **Cascade deletion:** All edges connected to the deleted entity (as source or target) are deleted atomically in the same bundle
- Deleting an entity does NOT cascade to delete related entities (only edges)
- All deletion behavior is explicit, auditable, and reversible via undo
- **Undo behavior:** Restoring a deleted entity also restores all edges that were cascade-deleted, unless the other entity has also been deleted or data has changed (edge restoration is conditional)

**Edge cascade computation:**
- The system computes cascade-deleted edges at deletion time (finds all edges where entity is source or target)
- Computed edges are stored explicitly in the `DeleteEntity` operation for auditability
- This hybrid approach ensures: (1) no manual edge enumeration required, (2) full audit trail of what was deleted

---

## Fields

Fields are key-value data attached to entities. Fields are the mechanism for all data storage.

**Anchor invariant:** All entity data is stored as fields. Fields are either shared (visible/writable across tables via confirmed mappings) or namespaced (private to a module).

### Field Structure

```yaml
Field:
  entity_id: UUID
  key: string           # the field identifier
  value: any
  source: Source        # who wrote this
  timestamp: HLC
```

### Two Types of Field Keys

| Type | Format | Example | Behavior |
|------|--------|---------|----------|
| **Shared key** | `name` | `name`, `email`, `phone` | Multiple modules can read/write after confirmed mapping |
| **Namespaced key** | `module.field` | `contacts.status` | Private to that module |

### Shared Keys: Suggested-Confirmed Model

Modules declare `shared_key` annotations in their table schemas as **suggestions** of cross-module field equivalence. These suggestions do not auto-activate.

**Lifecycle:**

1. **Declaration:** Module manifest declares `shared_key` on a field (developer intent).
2. **Suggestion:** On module adoption or first table-linking, the system presents suggested field mappings based on shared key overlap.
3. **Confirmation:** User reviews and confirms (or rejects) each suggested mapping.
4. **Activation:** Confirmed mappings behave identically to direct shared keys -- writes to any mapped field write to the shared key at entity level.

```
Module A declares:  name  -> shared_key "name"
Module B declares:  display_name -> shared_key "name"

On adoption, system prompts:
  "Contacts and Scheduler both have a 'name' field.
   Should these be the same data?"

User confirms -> fields are unified.
User rejects  -> fields remain independent.
```

**Rules:**

- No auto-binding of shared keys on module install
- Templates (e.g., "Stage Management" starter) can pre-confirm mappings for zero-friction onboarding
- Users can create custom field mappings beyond what modules suggest
- Confirmed mappings are stored in workspace configuration and are auditable
- Users can revoke a confirmed mapping at any time (fields become independent again; existing values are copied)

### Standard Shared Keys

The following shared keys are recommended for common data types. Modules should prefer these over custom keys when applicable:

| Shared Key | Type | Description |
|------------|------|-------------|
| `name` | string | Display name |
| `email` | string | Email address |
| `phone` | string | Phone number |
| `description` | string | Long-form description |
| `notes` | string | General notes |
| `created_at` | timestamp | Creation timestamp |
| `updated_at` | timestamp | Last modification timestamp |

Custom shared keys can be defined by users in workspace configuration.

### Shared Key Type Validation

**Anchor invariant:** If two modules declare conflicting types for the same shared key, an error is surfaced at module adoption time.

- Modules declare the type of each shared key they use
- On module adoption, the system validates type compatibility for any shared keys the user has confirmed
- Conflicting types (e.g., `name: string` vs `name: number`) prevent confirmation of that mapping
- Users must resolve conflicts by adjusting module configuration or choosing one module's type

### Namespaced Key Semantics

- Namespaced keys are private to a module
- Format: `module.field` (e.g., `contacts.internal_notes`)
- Writes to namespaced keys do not affect other fields
- Provides module isolation for module-specific data

### Sources

A source is anything that writes data:

- Module
- User (direct edit)
- Script (includes imports and automation)
- Rule (automated)

All writes are attributed to their source for auditability.

---

## Facets (Internal Layer)

Facets are the internal mechanism that backs tables. Each table declared by a module corresponds to a facet. Facets group fields and enable multi-table entity membership. Users do not interact with facets directly.

**Anchor invariant:** Facets are module-owned. Each table declaration produces a facet. Facets enable an entity to participate in multiple tables simultaneously.

### Facet-Table Correspondence

Every table declaration in a module manifest produces a facet:

| Module | Table Declaration | Internal Facet |
|--------|-------------------|----------------|
| `contacts` | `Contacts` | `contacts.Contacts` |
| `lighting` | `LightingCues` | `lighting.LightingCues` |
| `sm` | `SMCues` | `sm.SMCues` |
| `sound` | `SoundCues` | `sound.SoundCues` |

An entity's "type" is the set of facets attached to it. There is no single canonical type field. An entity with facets `contacts.Contacts` and `scheduling.Attendees` is simultaneously a contact and an attendee.

### Table-Linking Compatibility

When a user links two tables (e.g., "Contacts records should also appear in Attendees"), the system checks whether the table schemas are compatible. Compatibility is determined by field overlap and type agreement, not by a static kind constraint.

**Enforcement behavior by source:**

| Source | Default Behavior | Rationale |
|--------|------------------|-----------|
| **User action** | Warn on unlikely combinations, then allow | Respects user agency; warning surfaces potential issues |
| **Rule action** | Skip unlikely combinations, log for review | Automation should be conservative |
| **Script action** | Same as user | Script runs with user authority |

**Workspace-level configuration:**

| Setting | Effect |
|---------|--------|
| `block` | Strict enforcement -- incompatible table combinations fail |
| `warn_allow` (default for user) | Warning shown, operation proceeds |
| `skip_log` (default for rule) | Operation skipped, logged for review |
| `allow_silent` | Permissive -- no warning or logging |

### Facet Operations

**Attach (add record to table):**

```yaml
operation: attach_facet
  entity: <id>
  facet: sm.SMCues
  source:
    type: user | rule
    actor: <actor_id>
    rule_id: <if rule-triggered>
```

**Detach (remove record from table, soft delete):**

```yaml
operation: detach_facet
  entity: <id>
  facet: sm.SMCues
  preserve: true    # stash field values for potential restore
```

- `preserve: true` stores field values in operation metadata
- Later restoration can recover the values
- **CRDT fields:** Full CRDT state (Yjs document) is preserved, not just rendered text. On restore, the field can continue receiving `ApplyCRDT` deltas.

**Restore (re-add record to table):**

```yaml
operation: restore_facet
  entity: <id>
  facet: sm.SMCues
  from_operation: <detach_op_id>   # reference the detach operation
```

### CRDT Field Declaration

Fields can enable CRDT merge semantics for collaborative editing. See [crdt.md](crdt.md) for details.

```toml
[table.LightingCues.fields.description]
type = "string"
shared_key = "description"
crdt = "text"

[table.LightingCues.fields.notes]
type = "string"
crdt = "text"
crdt_granularity = "character"   # character (default) or paragraph

[table.LightingCues.fields.tags]
type = "list"
crdt = "list"
item_type = "string"             # string, number, or boolean
```

**CRDT field rules:**
- `crdt = "text"` -- Text CRDT for string fields
- `crdt = "list"` -- List CRDT for array fields (primitive items only)
- For ordered lists of entity references, use [ordered edges](ordered-edges.md) instead
- CRDT declaration is immutable after module adoption

---

## Edges

Edges are first-class relationships between entities with their own properties and lifecycle.

**Anchor invariant:** Edges represent relationships, not data. The relationship itself can have properties. Edge constraints across modules are user-configured, not hardcoded.

### Edge Structure

```yaml
Edge:
  id: unique_edge_id
  type: "casting.assigned_to"     # Module-namespaced edge type
  source: actor_entity_id         # From entity
  target: scene_entity_id         # To entity
  properties:                     # Edge-specific data
    character: "Juliet"
    entrance: "Enter from SR"
    has_quick_change: true
```

### Edge Type Declaration

Modules declare edge types in their schema:

```toml
[edge.assigned_to]
# Constraints on own tables are allowed
source_tables = ["casting.Actors"]

# Cross-module constraints are HINTS, not requirements
target_tables_hint = ["scenes.Scenes", "scheduling.Segments"]
target_description = "Scene, segment, or schedulable unit"

# Edge properties
[edge.assigned_to.properties]
character = "string"
entrance = "string"
has_quick_change = "boolean"
```

### Ordered Edge Types

Edge types can enable CRDT-based ordering for ordered lists of entities. See [ordered-edges.md](ordered-edges.md) for details.

```toml
[edge.in_cue_list]
source_tables = ["lighting.LightingCues"]
target_tables = ["lighting.CueLists"]
ordered = true                # Enable CRDT-based ordering

[edge.in_cue_list.properties]
call_text = "string"
timing_override = "number"
```

**Ordered edge rules:**
- `ordered = true` -- Enable position tracking
- Edges are ordered within `(target, edge_type)` scope
- Use `CreateOrderedEdge` and `MoveOrderedEdge` operations
- Concurrent insertions at the same position merge deterministically
- The `_position` property is managed by the system (not user-specified)

### Edge Bindings (User-Configured)

Cross-module edge constraints are configured by users, not hardcoded:

```yaml
EdgeBinding:
  edge_type: "casting.assigned_to"

  source_constraint:
    table: "casting.Actors"           # Module's own table (fixed)

  target_constraint:
    mode: tables                      # tables | any
    tables: ["scenes.Scenes", "scheduling.Segments"]  # User-configured
```

**Module Independence:**
- Modules can constrain edges to their OWN tables
- Modules cannot hardcode constraints to OTHER modules' tables
- Modules can suggest via `*_hint` fields
- Users configure cross-module bindings

### Edge Directionality

- Edges are always directed (source -> target)
- Queries can traverse in either direction
- If bidirectional relationship needed, create two edges or query both directions

### Edge Properties

- Edges can have typed properties declared in schema
- Properties are specific to the relationship, not to either entity
- Example: `assigned_to` edge has `character` property (what role in this scene?)
- Example: `mounted_at` edge has `channel`, `dmx_address` (patch data for this mounting)

### Multiple Edges

- Multiple edges of the same type between same entities are allowed
- Differentiated by properties or context
- Example: Same cue placed on two pages with different call types (standby, go)

### Edge and Entity Deletion

**Anchor invariant:** Deleting an entity never deletes other entities. Edges to/from deleted entities are cascade-deleted atomically.

- When an entity is deleted, all edges where it appears as source or target are cascade-deleted
- Entity deletion and edge deletions are bundled atomically (all succeed or all fail)
- No cascade delete of entities through edges (only the edges themselves are deleted)
- Undoing entity deletion restores the entity and conditionally restores edges (skips restoration if other entity is gone or data has changed)
- Edges can be explicitly deleted independently (separate operation)

### Edge Conflicts

- Edge creation/deletion follows normal operation conflict rules
- Edge property edits follow field-level conflict rules
- Same edge created by two peers offline: deduplicated (same source, target, type)
- Conflicting property values: surfaced as conflict

### Edge Permissions

- Creating an edge requires edit permission on source entity
- Deleting an edge requires edit permission on source OR target entity
- Edge properties follow same permission model as entity fields

### Edge Queries

Common query patterns:
- "All edges of type X from entity Y" (outgoing)
- "All edges of type X to entity Y" (incoming)
- "All edges of type X" (global)
- Edges can be filtered by property values

### Use Cases

| Relationship | Edge Type | Properties |
|--------------|-----------|------------|
| Actor in Scene | `assigned_to` | character, entrance, blocking notes |
| Cue on Page | `placed_on` | position, call_type, call_text |
| Fixture at Position | `mounted_at` | unit_number, channel, dmx_address |
| Person in Department | `member_of` | title, start_date, is_head |
| Costume worn by Actor | `worn_by` | scenes, quick_change_location |

---

## Identity Repair (Merge / Split)

Identity Repair handles the exceptional case where separate entities represent the same real-world thing, or where a single entity should be separated into distinct records.

**Anchor invariant:** Identity repair is corrective maintenance, not primary modeling. The preferred model is multi-table entities from the start (via table-linking rules and confirmed field mappings).

### When Identity Repair Applies

- Legacy imports that created duplicate records across tables
- External data sources with inconsistent identity
- User error that created separate records for the same thing
- Dedup/matching rules did not catch a duplicate (e.g., "Jane Doe" vs "J. Doe")
- Records created before matching rules were configured

### Merge

Combine two entities that represent the same thing:

```yaml
operation: merge_entities
  sources: [<id_1>, <id_2>]
  into: <surviving_id>
  # conflicting field values surface as conflicts
```

**Anchor invariant:** Merge uses deterministic survivor selection. The entity with the lexicographically smaller UUID survives. This ensures independent merge operations across partitions choose the same survivor.

- Survivor chosen deterministically: `survivor = min(entity_a, entity_b)` by UUID
- Surviving entity receives all facets from both entities (appears in all tables both records belonged to)
- If both entities have the same facet type, field conflicts are created
- Conflicts are resolved the same way as other conflicts
- The absorbed entity's ID is recorded in the MergeResolution table

```yaml
MergeResolution:
  absorbed_id: <merged_away_id>
  survivor_id: <surviving_id>
  merge_operation: <merge_op_id>
```

**Resolution semantics:**
- Queries using an absorbed ID automatically resolve to the survivor
- Historical operations targeting the absorbed ID are interpreted as targeting the survivor during state replay
- Future operations targeting an absorbed ID are rejected ("entity was merged into X")
- Resolution chains are supported: if A->B and B->C, then A resolves to C

**Partition behavior:**
- Independent merge operations across partitions choose the same survivor (deterministic)
- No conflict when partitions merge the same entities
- Chained merges (A+B in partition 1, B+C in partition 2) resolve transitively after sync

### Split

When an entity should be separated (undo a prior merge, or decompose a record):

```yaml
operation: split_entity
  source: <original_id>
  into: [<new_id_1>, <new_id_2>]
  facet_distribution:
    new_id_1: [lighting.LightingCues]
    new_id_2: [sm.SMCues]
  field_distribution:
    new_id_1: [cue_number, intensity]
    new_id_2: [page_number, call_text]
    both: [name]    # copied to both
```

- Split is an explicit user action
- Split does not revert field values to pre-merge state
- Split creates two entities; facets (table memberships) are distributed by user choice

### Merge Exceptions

Prevent rules from re-merging split entities:

```yaml
merge_exceptions:
  - [<entity_a>, <entity_b>]    # never auto-merge these
```

Rules check the exception list before proposing merges.

### Dedup/Matching Rules

Matching rules are scoped to tables, not global:

```yaml
rule: match_contacts
  table: "contacts.Contacts"
  match_keys: ["name", "email"]
  action: propose_merge          # never auto-merge without user consent
```

When entities match by rule, the user is prompted to merge. Auto-merge can be enabled by power users via `action: auto_merge`.

### Audit Trail

- Erroneous identity assertions and their correction are preserved in history
- History must reflect both mistakes and recoveries truthfully
- No historical operation is erased due to user error
- Mistakes are auditable for forensic review and learning

---

## Cross-Module Interoperability

The preferred model is **one entity per real-world thing, appearing in multiple tables**.

**Anchor invariant:** Modules declare shared key suggestions for common data. Users confirm field mappings. Users can configure rules for automatic table membership and entity matching. Nothing fuses data without explicit user consent.

### Multi-Table Entities (Preferred)

- A single entity can appear in tables from multiple modules
- This is the primary mechanism for cross-module data sharing
- Confirmed shared key mappings ensure the same field name = same data across tables
- Modules interoperate by attaching their facets to shared entities

### How Cross-Module Identity Works

1. **Modules declare shared keys** -- Contacts module maps `name` to shared key `name`. Scheduler module maps `display_name` to shared key `name`. These are suggestions.
2. **User confirms mappings** -- On module adoption, the system presents: "Contacts and Scheduler both have a 'name' field. Should these be the same data?" User confirms.
3. **User configures matching rules** -- "Match records in Contacts and Attendees by `name` and `email`."
4. **System proposes merges** -- When records match by rule, user is prompted to merge.
5. **Merged entity appears in both tables** -- One entity, many facets, each module sees its own table's columns.

### Shared Key Conflict Semantics

- Writes to any field mapped to the same confirmed shared key are writes to the same semantic field
- Conflict detection treats shared-key-mapped fields as a single field
- Sharing requires user confirmation of field mappings at module adoption time

### Module Independence

- Modules never assume other modules exist
- Modules declare shared keys they read/write as suggestions
- Cross-module coordination happens via confirmed shared key mappings
- Users can override or revoke mappings in workspace config

### Defaults Are Safe

- All matching and table-linking defaults to "propose and wait for confirmation"
- Power users can configure auto-merge, auto-attach, etc.
- The system never mutates data without explicit user consent
- Templates can pre-confirm mappings for streamlined onboarding

### Source Attribution

All operations include source metadata:

```yaml
operation: attach_facet
  entity: <id>
  facet: sm.SMCues
  source:
    type: rule
    rule_id: "lighting_is_called_rule"
    triggered_by: <op_id>
```

```yaml
operation: attach_facet
  entity: <id>
  facet: sm.SMCues
  source:
    type: user
    actor: <user_id>
```

### Module Behavior Based on Source

Modules can handle explicit vs implicit table membership differently:

```typescript
onRecordAdded(entity, table, source) {
  if (source.type === "user") {
    // User added explicitly
    placeAtMousePosition(entity);
  } else if (source.type === "rule") {
    // Rule added automatically
    addToUnmappedBin(entity);
    notify("New cue from lighting needs page placement");
  }
}
```

---

## Assets & Blobs

- Blobs are immutable and content-addressed (hash = identity)
- Modifying content creates a new blob with a new hash
- Identical content = identical hash = automatic deduplication
- Asset deletions are recorded as operations
- Deleting an asset does not delete the blob immediately (GC handles cleanup)

### Garbage Collection

- Blobs may be GC'd after retention window if unreferenced by active ops
- GC never deletes blobs referenced by ops within retention window
- Ops referencing GC'd blobs remain valid; blob retrieval returns "unavailable"

### Replay & Storage

- Oplog replay reconstructs entity state without requiring blob data
- Ops that reference assets store metadata (hash, filename, size) inline
- Blob absence is a retrieval failure, not a state corruption
- Blobs are stored and synced compressed (transparent to modules)

---

## Open Questions

- Optimized graph traversal
- Indexing strategies for table queries and cross-table membership lookups
- Edge cardinality constraints (enforce "exactly one"?)
- Edge validation rules
- Cross-module query semantics (query by shared key values across confirmed mappings)
- Retention window duration (90 days? configurable?)
- Cold storage integration for archived blobs?
- Module-declared asset types with different retention rules?
- Compression algorithm (zstd?)
- Table membership rule evaluation order and conflict resolution
- UI for confirming/revoking shared key mappings at scale
