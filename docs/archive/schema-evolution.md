# Schema Evolution Specification

This document defines schema versioning, migrations, and backward compatibility.

---

## Schema Evolution

Schema evolution handles changes to plugin facet definitions over time.

**Anchor invariant:** Schema changes are operations in the oplog. Breaking changes require explicit migrations. Old operations remain valid through migration interpretation.

---

## Schema Versioning

- Plugins use semantic versioning (MAJOR.MINOR.PATCH)
- **PATCH**: Bug fixes, no schema change
- **MINOR**: Backwards-compatible changes (add optional field)
- **MAJOR**: Breaking changes (remove, rename, type change)
- Schema version is workspace-scoped, not peer-scoped
- All peers in a workspace share the same schema version for each plugin

---

## Operations and Schema Versions

- Operations include the plugin versions they were created with
- This allows replay to interpret operations against the correct schema
- Operation structure includes:

```yaml
Operation:
  id: unique_op_id
  actor_id: ...
  hlc: ...
  plugin_versions:
    contacts: "1.1.0"
    scheduler: "2.0.0"
  payload: { ... }
  signature: ...
```

---

## Change Classification

| Change Type | Breaking? | Requires Migration? |
| ----------- | --------- | ------------------- |
| Add optional field (default null) | No | No (auto-applied) |
| Add required field | Yes | Yes |
| Remove field | Yes | Yes |
| Rename field | Yes | Yes |
| Change field type | Yes | Yes |
| Add validation constraint | Maybe | Yes (if existing data may violate) |
| Change shared_key mapping | Yes | Yes |
| Remove shared_key mapping | Yes | Yes |

---

## Non-Breaking Changes

- Adding an optional field with default `null` is always safe
- Auto-applied when plugin is updated
- Emits a `schema_update` operation for auditability
- Does not require Admin approval

---

## Breaking Changes and Migrations

Breaking changes require explicit migration operations:

```yaml
Operation:
  type: schema_migration
  actor: admin_actor_id
  plugin: contacts
  from_version: "1.0.0"
  to_version: "2.0.0"

  migrations:
    - action: rename_field
      facet: contact
      from: phone
      to: phone_primary

    - action: add_field
      facet: contact
      name: phone_secondary
      type: string
      required: false
      default: null

    - action: remove_field
      facet: contact
      name: fax
```

---

## Migration Permissions

- New permission: `can_apply_migrations`
- Granted to Admin role by default
- Can be assigned to custom roles
- Migration operations are rejected if actor lacks this permission

---

## Adding Required Fields

When adding a required field to a facet with existing entities:

- Migration MUST specify a default value
- Existing entities receive the default value
- If default is semantically incomplete (e.g., "Unassigned"), entities can be flagged for review
- This aligns with "incomplete data is valid state" invariant

```yaml
- action: add_field
  facet: contact
  name: department
  type: string
  required: true
  default: "Unassigned"
```

---

## Shared Key Migration

When changing shared_key mappings:

```yaml
- action: change_shared_key
  facet: contact
  field: display_name
  from_shared_key: name
  to_shared_key: legal_name
```

**Behavior:**
- Existing data remains in the old shared key
- New writes go to the new shared key
- Migration can optionally copy data: `copy_data: true`
- Rules referencing the field are flagged for review

---

## Migration Execution

When a migration operation is applied:

1. **Validation** — Migration is checked for correctness
2. **Schema update** — Workspace schema registry is updated
3. **Data migration** — All existing entities with affected facet are updated
4. **Rule flagging** — Rules referencing changed fields are flagged for review
5. **Audit** — Migration is recorded in oplog

---

## Backward Compatibility for Oplog Replay

When replaying an old operation against a newer schema:

1. System reads operation's `plugin_versions`
2. Determines migration path from operation version to current version
3. Interprets field names through migration chain
4. Example: `set contact.phone = "555"` with schema 1.0.0 is interpreted as `set contact.phone_primary = "555"` under schema 2.0.0

**Old operations remain valid.** They are interpreted, not modified.

---

## Rule Updates on Migration

When a migration affects fields referenced by rules:

| Change Type | Proposal Confidence | Behavior |
| ----------- | ------------------- | -------- |
| Field renamed | High | Propose update with one-click accept |
| Field removed | Medium | Flag for review, rule may need deletion |
| Type changed | Low | Require manual review |

- Migration surfaces all affected rules for review
- High-confidence updates can be batch-accepted
- Low-confidence updates require individual review
- No rule is auto-updated without user confirmation

---

## Plugin Adoption

- When a new peer adopts a plugin, they adopt at the current workspace schema version
- No version choice — all peers share the same schema version
- If you need an older version, fork the workspace

---

## Rollback

- There is no "undo" for migrations
- Rollback is expressed as a new migration forward
- Example: 2.0.0 → 2.1.0 that reverses the changes from 1.0.0 → 2.0.0
- History shows the full journey: upgrade, then reversal
- This maintains immutable history

---

## Migration Conflicts

If two Admins offline create different migrations for the same plugin:

- Treated as a conflict on the schema version
- Must be resolved before workspace can proceed
- Resolution picks one migration path
- Losing migration is discarded (or merged manually)

---

## CRDT Field Schema Evolution

CRDT fields have special migration requirements due to their internal state format. See [crdt.md](crdt.md) for CRDT field specification.

### Adding CRDT to Existing Field

Converting a plain field to CRDT:

```yaml
- action: convert_to_crdt
  facet: notes
  field: description
  crdt_type: text
```

**Behavior:**
- Existing string value is converted to Yjs document with that content
- Conversion is automatic and lossless for text fields
- Field history (prior SetField ops) is preserved in oplog but not in CRDT state

**Constraints:**
- Source field must be compatible type (string → text CRDT, array → list CRDT)
- Incompatible types require remove + add (data loss acknowledged)

### Removing CRDT from Existing Field

Converting a CRDT field back to plain:

```yaml
- action: convert_from_crdt
  facet: notes
  field: description
```

**Behavior:**
- CRDT state is rendered to plain value (text string or array)
- CRDT edit history within field is lost (only final state preserved)
- Subsequent edits use plain field semantics (LWW conflicts)

**Warning:** This is a lossy operation. Concurrent edits during migration may conflict.

### Changing CRDT Options

Changing CRDT configuration (e.g., granularity):

```yaml
- action: update_crdt_options
  facet: notes
  field: description
  crdt_options:
    granularity: paragraph    # Was: character
```

**Behavior:**
- Yjs state is preserved (granularity is a merge hint, not stored in state)
- Future operations use new granularity setting
- No data migration required

### Changing CRDT Type

Changing between CRDT types (e.g., text → list) is **not supported**:

```yaml
# NOT ALLOWED
- action: change_crdt_type
  field: tags
  from_crdt_type: text
  to_crdt_type: list
```

**Rationale:** Text and list CRDTs have incompatible internal structures. Conversion requires:
1. Remove field (loses data)
2. Add new field with new CRDT type
3. Optionally migrate data via script

### CRDT and Oplog Replay

When replaying oplog across schema versions:

1. **ApplyCRDT ops before CRDT enabled:** Treated as errors, logged and skipped
2. **ApplyCRDT ops after CRDT removed:** Treated as errors, logged and skipped
3. **SetField ops on CRDT fields:** Always valid (full state replacement)

The `plugin_versions` field in operations enables correct interpretation during replay.

### Breaking Changes Summary

| Change | Breaking? | Migration Action |
|--------|-----------|------------------|
| Add CRDT to plain field | No | `convert_to_crdt` |
| Remove CRDT from field | Yes (lossy) | `convert_from_crdt` |
| Change CRDT options | No | `update_crdt_options` |
| Change CRDT type | Yes | Remove + Add |
| Change Yrs version | Depends | See compatibility guarantees |

---

## Open Questions

- Complex migration expressions (conditional transforms)
- Migration testing/simulation before apply
- Bulk per-entity value specification in migrations
- Kind migration and renaming semantics
- CRDT state compaction during migration
