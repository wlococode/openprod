# IDEA-1: Unified Data Abstraction Layer

This document describes a simplified, maximally flexible data abstraction layer for openprod. The design prioritizes:

- **No hardcoded primitives** — Everything is configurable
- **Sensible defaults** — Works out of the box, power users can customize infinitely
- **One entity = one real-world thing** — Identity is unified, never fragmented
- **Plugin independence** — Plugins work in isolation, interop is opt-in
- **Full reversibility** — Every operation can be undone

---

## Core Principles

### Start from invariants, not abstractions

The system grows upward from rules that must be true:

1. **Identity is stable** — An entity ID never changes meaning
2. **All changes are operations** — Append-only log, auditable, replayable
3. **Plugins don't own entities** — Entities exist in the workspace; plugins attach data
4. **Configuration is optional** — Defaults work; customization is for exceptions
5. **Propose, don't act** — System suggests, user confirms (unless configured otherwise)

---

## Data Model

### Entity

An entity is **pure identity**. Nothing more.

```
Entity:
  id: UUID
```

- No predefined fields
- No inherent type or kind
- Just a stable reference to "this thing exists"

Fields come into existence when something writes them. An entity is just an ID with whatever fields have been attached.

### Fields

Fields live at the **entity level**, not inside plugin namespaces.

```
Field:
  entity_id: UUID
  key: string           # the field identifier
  value: any
  source: Source        # who wrote this
  timestamp: HLC
```

**Two types of field keys:**

| Type | Format | Example | Behavior |
|------|--------|---------|----------|
| **Shared key** | `name` | `name`, `email`, `phone` | Multiple plugins can read/write |
| **Namespaced key** | `plugin.field` | `casting.agent_contact` | Private to that plugin |

### Sources

A source is anything that writes data:

- Plugin
- User (direct edit)
- Script (includes imports and automation)
- Rule (automated)

All writes are attributed to their source for auditability.

---

## Plugin Field Declarations

Plugins declare which fields they use and how they map to shared keys.

### Plugin manifest

```yaml
# contacts plugin
fields:
  name:
    shared_key: name        # maps to shared key "name"
    type: string

  email:
    shared_key: email       # maps to shared key "email"
    type: string

  status:                   # no shared_key = namespaced as "contacts.status"
    type: string

  internal_notes:
    type: string
    private: true           # explicitly namespaced
```

### Behavior

- If `shared_key` is specified → plugin reads/writes that shared key
- If no `shared_key` → field is namespaced as `plugin.field`
- Multiple plugins mapping to the same shared key = same underlying data
- **No parity rules needed** — it's the same field, not copies

### User overrides

Users can override plugin mappings at the workspace level:

```yaml
# workspace config
field_overrides:
  contacts:
    name:
      shared_key: legal_name    # override: use different shared key
```

Users can also create custom shared keys:

```yaml
custom_shared_keys:
  - abbreviated_name
  - preferred_contact_method
```

---

## Entity Type (Kind)

Entity "type" is not special — it's just a shared key called `kind`.

```yaml
# When Contacts creates a person
fields:
  kind: "Person"
  name: "Jane Doe"

# When Lighting creates a cue
fields:
  kind: "Cue"
  cue_number: 42
```

### Kind is immutable

Once set, an entity's `kind` cannot be changed. This ensures:

- Facet compatibility remains stable
- Entity identity is predictable ("a person doesn't become a cue")
- Mental model stays clean

**If you need to change an entity's kind:**
- **Split** the entity and create a new one with the correct kind
- Or **delete + recreate** if it was simply wrong from the start

Kind immutability is enforced at the operation level — `set_field` operations targeting the `kind` field are rejected after initial creation.

### Facet compatibility

Facets declare which kinds they're compatible with:

```yaml
# contacts plugin - specific kind
facets:
  Contact:
    compatible_kinds: ["Person"]
    fields: [name, email, phone, status]

# lighting plugin - specific kind
facets:
  Cue:
    compatible_kinds: ["Cue"]
    fields: [cue_number, intensity, color, is_called]

# scheduling plugin - multiple kinds
facets:
  Schedulable:
    compatible_kinds: ["Person", "Event", "Scene"]
    fields: [start_time, end_time, location]

# notes plugin - wildcard (any entity)
facets:
  Note:
    compatible_kinds: "*"
    fields: [content, author, created_at, resolved]

# tags plugin - omitted means any (same as wildcard)
facets:
  Tag:
    fields: [tag_name, color]
```

**Compatibility declarations:**

| Declaration | Meaning |
|-------------|---------|
| `compatible_kinds: ["Person"]` | Only Person entities |
| `compatible_kinds: ["Person", "Cue"]` | Person or Cue entities |
| `compatible_kinds: "*"` | Any entity (wildcard) |
| *(omitted)* | Same as `"*"` — unrestricted by default |

This enables cross-cutting plugins (Notes, Todos, Tags, Comments) to attach to any entity while domain-specific plugins enforce constraints.

### Compatibility mismatch behavior

When attaching a facet to an entity with incompatible `kind`:

**Default behavior: Warn + require confirmation**

```
⚠️ Compatibility warning

Contact facet expects kind: "Person"
This entity has kind: "Cue"

[Attach anyway] [Cancel]
```

- Not a hard block (preserves flexibility for edge cases)
- Not silent (prevents accidents)
- Consistent with "propose, don't act" philosophy

**For rule-triggered attachments:** Incompatible entities are skipped by default and logged for review.

---

## Rules: Query → Action

All automation is expressed as **rules**. One unified concept replaces:
- Match rules
- Presence rules
- Parity rules (eliminated — shared keys handle this)
- Field mappings (eliminated — shared keys handle this)

### Rule structure

```yaml
rule:
  name: "Actors are Contacts"
  when: <query>
  propose: <action>
```

### Example rules

**Facet attachment (replaces presence rules):**

```yaml
rule:
  name: "Actors are Contacts"
  when: kind == "Person" AND casting.role == "actor"
  propose:
    action: attach_facet
    facet: casting.Actor
```

**Cross-plugin facet attachment:**

```yaml
rule:
  name: "Called cues appear in SM script"
  when: kind == "Cue" AND lighting.is_called == true
  propose:
    action: attach_facet
    facet: sm.Cue
    defaults:
      department: "lighting"
      cue_number: $source.lighting.cue_number
```

**Entity matching (replaces match rules):**

```yaml
rule:
  name: "Match people by name"
  when: kind == "Person"
  match_on: [name, email]
  propose:
    action: merge_entities
```

### Rule scoping via queries

The `when` clause IS the scope. No separate scoping concept needed.

**Scope by kind (simplest, recommended):**
```yaml
rule:
  when: kind == "Person"
  match_on: [name, email]
  propose: merge_entities
```

**Scope by facet presence:**
```yaml
rule:
  when: contacts.Contact exists
  match_on: [name]
  propose: merge_entities
```

**Scope by complex conditions:**
```yaml
rule:
  when: kind == "Person" AND department == "crew" AND NOT is_archived
  match_on: [name, email]
  propose: merge_entities
```

The query determines which entities the rule evaluates. All three patterns above are valid — use whatever fits your needs.

### Default values on attachment

Rules can specify initial values when attaching facets:

```yaml
defaults:
  department:
    value: "lighting"
    mode: fill_if_empty      # only set if null/missing

  cue_number:
    value: $source.lighting.cue_number
    mode: always             # always set, even if exists

  notes:
    value: $source.lighting.notes
    mode: preserve_on_restore # respect soft-delete values
```

**Modes:**

| Mode | Behavior |
|------|----------|
| `fill_if_empty` | Only set if field is null/missing |
| `always` | Overwrite any existing value |
| `preserve_on_restore` | Use soft-deleted value if available, else default |

### Dynamic references in rules

- `$source.<field>` — field from entity that triggered the rule
- `$now` — current timestamp
- `$actor` — user who triggered the change
- `$literal("value")` — explicit static value

---

## Derived Fields

Computed fields that auto-update when sources change.

```yaml
derived_field:
  name: abbreviated_name
  source: name
  transform: abbreviate()      # "Jane Doe" → "J. Doe"
  auto_update: true
```

Or as a rule:

```yaml
rule:
  name: "Compute abbreviated name"
  when: name changes
  action: set_field
    field: abbreviated_name
    value: abbreviate($source.name)
```

### Built-in transforms

| Transform | Example |
|-----------|---------|
| `abbreviate()` | "Jane Doe" → "J. Doe" |
| `initials()` | "Jane Doe" → "JD" |
| `uppercase()` | "jane" → "JANE" |
| `lowercase()` | "JANE" → "jane" |
| `format_date(fmt)` | ISO → "Jan 15, 2026" |

Complex transforms can reference Lua scripts.

---

## Plugin Interface Slots

Plugins declare **what data they need**, not **which fields they read**.

### Plugin manifest

```yaml
# scheduler plugin
interface_slots:
  display_name:
    description: "Name shown on schedule/export"
    default: name
    type: string

  contact_email:
    description: "Email for notifications"
    default: email
    type: string

  department:
    description: "Department grouping"
    default: department
    type: string
```

### Plugin code

```typescript
// Plugin reads from slots, never from specific fields
function renderAttendee(entity, slots) {
  return <div>{slots.display_name}</div>;
}

function exportSchedule(entities, slots) {
  return entities.map(e => ({
    name: slots.display_name,
    email: slots.contact_email
  }));
}
```

### User slot bindings

```yaml
# workspace config
slot_bindings:
  scheduler:
    display_name: abbreviated_name   # use derived field
    contact_email: personal_email    # different email field
```

### Data flow

```
[Entity Fields]        [Derived Fields]         [Plugin Slots]
     name        →     abbreviated_name    →    scheduler.display_name
     email       ───────────────────────────→   scheduler.contact_email
```

**Plugin knows nothing about where data comes from. User controls everything.**

---

## Facet Operations

### Attach

```yaml
operation: attach_facet
  entity: <id>
  facet: casting.Actor
  source:
    type: user | rule
    actor: <actor_id>
    rule_id: <if rule-triggered>
```

### Detach (soft delete)

```yaml
operation: detach_facet
  entity: <id>
  facet: casting.Actor
  preserve: true    # stash field values for potential restore
```

- `preserve: true` → field values stored in operation metadata
- Later restoration can recover the values

### Restore

```yaml
operation: restore_facet
  entity: <id>
  facet: casting.Actor
  from_operation: <detach_op_id>   # reference the detach operation
```

---

## Entity Operations

### Split

When users disagree about an entity, split into two:

```yaml
operation: split_entity
  source: <original_id>
  into: [<new_id_1>, <new_id_2>]
  facet_distribution:
    new_id_1: [lighting.Cue]
    new_id_2: [sm.Cue]
  field_distribution:
    new_id_1: [cue_number, intensity]
    new_id_2: [page_number, call_text]
    both: [name]    # copied to both
```

### Merge

Combine two entities that represent the same thing:

```yaml
operation: merge_entities
  sources: [<id_1>, <id_2>]
  into: <surviving_id>
  # conflicting field values surface as conflicts
```

### Merge exceptions

Prevent rules from re-merging split entities:

```yaml
merge_exceptions:
  - [<entity_a>, <entity_b>]    # never auto-merge these
```

Rules check the exception list before proposing merges.

---

## Source Attribution

All operations include source metadata:

```yaml
operation: attach_facet
  entity: <id>
  facet: sm.Cue
  source:
    type: rule
    rule_id: "lighting_is_called_rule"
    triggered_by: <op_id>
```

vs

```yaml
operation: attach_facet
  entity: <id>
  facet: sm.Cue
  source:
    type: user
    actor: <user_id>
```

### Plugin behavior based on source

Plugins can handle explicit vs implicit creation differently:

```typescript
onFacetAttached(entity, facet, source) {
  if (source.type === "user") {
    // User created explicitly
    placeAtMousePosition(entity);
  } else if (source.type === "rule") {
    // Rule attached automatically
    addToUnmappedBin(entity);
    notify("New cue from lighting needs page placement");
  }
}
```

---

## Safety Model

### Propose, don't act

By default, all automated actions are **proposals**:

- Match rule finds candidates → proposes merge → user confirms
- Presence rule triggers → proposes facet attachment → user confirms
- Everything surfaces for review

### Power user automation

Users can configure rules to auto-accept:

```yaml
rule:
  name: "Actors are Contacts"
  when: kind == "Person" AND casting.role == "actor"
  propose:
    action: attach_facet
    facet: casting.Actor
  auto_accept: true    # skip confirmation
```

### Full reversibility

- **Merge** → can split later
- **Facet attach** → can detach (with soft delete)
- **Field edit** → undo via oplog
- **Split** → can re-merge

All operations are in the append-only log. Nothing is truly destructive.

---

## Configuration Hierarchy

```
Plugin defaults
    ↓ (overridden by)
Workspace config
    ↓ (overridden by)
Per-entity overrides (rare)
```

### Plugin defaults

Plugins ship with sensible defaults:
- Field → shared key mappings
- Slot → field bindings
- Facet compatibility declarations

### Workspace config

Users customize at workspace level:
- Override field mappings
- Override slot bindings
- Add custom shared keys
- Define rules
- Define derived fields

### Per-entity overrides

Rare, but possible:
- Merge exceptions
- Entity-specific slot bindings

---

## Query Language

Queries select entities based on field values:

```
# Basic
kind == "Person"

# Multiple conditions
kind == "Person" AND contacts.role == "actor"

# Field existence
email exists

# Comparisons
lighting.intensity > 50

# Cross-field
lighting.is_called == true AND sm.page_number == null
```

Queries are used in:
- Rules (when clause)
- UI filtering
- Exports
- Scripts

---

## Summary: What's Eliminated

| Old Concept | Replaced By |
|-------------|-------------|
| Kind Groups | Shared key `kind` + facet compatibility |
| Field Mappings | Plugin → shared key declarations |
| Parity Rules | Same shared key = same data |
| Presence Rules | Query → action rules |
| Match Rules | Query → action rules with `match_on` |
| Field projections | Derived fields + slot bindings |

**One unified concept: query → action rules**

---

## Summary: Core Components

1. **Entity** — Pure identity (UUID only)
2. **Fields** — Key-value data on entities (shared or namespaced)
3. **Facets** — Plugin-defined field groupings with compatibility constraints
4. **Rules** — Query → action automation (attach, merge, set field)
5. **Derived fields** — Computed fields with transforms
6. **Slots** — Plugin interface points, user-bindable to any field

---

## Design Goals Achieved

| Goal | How |
|------|-----|
| No hardcoded primitives | Fields are arbitrary; kind is just a field |
| Sensible defaults | Plugins declare defaults; works without config |
| Infinite customization | Users can override everything |
| One entity = one thing | Shared keys unify data; merge resolves duplicates |
| Plugin independence | Plugins use shared keys; don't know about each other |
| Full reversibility | All operations auditable and undoable |
| Minimal configuration | Common case needs zero config; rules for exceptions |
