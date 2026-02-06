# Module System Specification

This document defines module structure, table declarations, field schemas, shared key suggestions, capabilities, views, scripts, installation lifecycle, and templates.

---

## Philosophy

- **Independent**: Every module must be useful on its own.
- **No inter-module dependencies**: Modules never assume other modules exist. A module cannot import code from, declare a dependency on, or require the presence of another module.
- **Opt-in interoperability**: Cross-module data sharing emerges through confirmed field mappings, not code coupling. Modules suggest shared keys; users confirm them.
- **Tables, not facets**: Module developers declare tables with field schemas. Users see tables, records, and fields. The entity/facet layer is internal architecture, not a module author concern.

---

## Installation and Adoption

### Installation

Installation is adding a module folder to the workspace modules directory:

```
workspace/
  modules/
    contacts/        <-- drop folder here
    lighting/
    scheduling/
```

No registry, no package manager, no build step. The core detects module folders on startup and watches for additions at runtime.

### Adoption

| Action      | Scope     | Effect                                                    |
| ----------- | --------- | --------------------------------------------------------- |
| **Install** | Per-user  | Module UI and scripts available locally                   |
| **Adopt**   | Workspace | Module schema shared with all collaborators via sync      |

A lighting designer can install modules the stage manager does not need. If the lighting module's schema should be shared workspace-wide, it must be adopted. Adoption triggers the field mapping confirmation flow (see Shared Key Suggestions below).

### Uninstallation

Uninstalling a module means removing its folder from the workspace modules directory. The core handles orphaned data gracefully:

- Records created by the module remain in the oplog and materialized state.
- Fields namespaced to the removed module are preserved but hidden from active views.
- Edges referencing removed module schemas remain valid; queries still traverse them.
- Reinstalling the module re-surfaces all previously hidden data.
- No data is deleted on uninstall. The user can explicitly clean up orphaned data if desired.

---

## Module Package Structure

```
contacts/
  manifest.toml           # Module identity, table declarations, capabilities
  schema/
    tables.toml           # Table and field definitions
  views/
    ContactList.tsx        # TypeScript UI components
    ContactDetail.tsx
  scripts/
    import_csv.lua        # Lua automation scripts
    export_vcards.lua
  assets/
    icon.png              # Static assets (icons, templates, etc.)
```

### Required Files

Only `manifest.toml` is required. All other directories and files are optional. A module that declares only schema (no views, no scripts) is valid. A module that provides only views (no schema, no scripts) is valid.

### Reserved Module Names

The name `core` is reserved and cannot be used as a module name (it refers to built-in system tables and fields).

---

## Manifest

The manifest declares module identity, version, capabilities, and table schemas. All module metadata lives in `manifest.toml`.

### Minimal Manifest

```toml
[module]
name = "contacts"
version = "1.0.0"
display_name = "Contacts"
description = "Contact management for production teams"
author = "Openprod Community"
```

### Full Manifest Example

```toml
[module]
name = "lighting"
version = "2.1.0"
display_name = "Lighting"
description = "Lighting cue management, patch, and console integration"
author = "Openprod Community"

[capabilities]
network = ["osc:8000-8010", "http.client"]
filesystem = ["read:imports/*", "write:exports/*", "watch:imports/*"]
data = ["read:lighting.*", "write:lighting.*"]
```

Capability declarations are covered in full in the Capabilities section below. Table schemas can live inline in the manifest or in a separate `schema/tables.toml` file. The separate file is recommended for modules with multiple tables.

---

## Table Declarations

Modules declare tables with typed field schemas. Each table maps to one facet internally, but module authors think in terms of tables and fields.

### Schema File: `schema/tables.toml`

```toml
# ─────────────────────────────────────────────
# Lighting Cues table
# ─────────────────────────────────────────────
[table.cues]
display_name = "Lighting Cues"
description = "All lighting cue data: numbers, levels, timing"

# Compatibility hints: which other tables make sense to link with this one.
# These are UI hints for the table-linking flow — not enforcement.
compatible_tables = ["sm.cues", "scheduling.events"]

[table.cues.fields.cue_number]
type = "number"
required = true
display_name = "Cue #"
description = "The cue number as it appears on the console"

[table.cues.fields.label]
type = "string"
display_name = "Label"

[table.cues.fields.name]
type = "string"
display_name = "Cue Name"
shared_key = "name"                # SUGGESTION — see Shared Key Suggestions

[table.cues.fields.intensity]
type = "number"
display_name = "Intensity"
description = "Master intensity level (0-100)"

[table.cues.fields.fade_time]
type = "number"
display_name = "Fade Time"
description = "Upfade time in seconds"

[table.cues.fields.is_called]
type = "boolean"
display_name = "Called"
description = "Whether this cue is called by the stage manager"
default = true

[table.cues.fields.notes]
type = "string"
display_name = "Notes"
shared_key = "notes"
crdt = "text"

# ─────────────────────────────────────────────
# Fixtures table
# ─────────────────────────────────────────────
[table.fixtures]
display_name = "Fixtures"
description = "Lighting instruments: type, position, patch"

[table.fixtures.fields.unit_number]
type = "string"
required = true
display_name = "Unit #"

[table.fixtures.fields.name]
type = "string"
display_name = "Fixture Name"
shared_key = "name"

[table.fixtures.fields.channel]
type = "number"
display_name = "Channel"

[table.fixtures.fields.dmx_address]
type = "number"
display_name = "DMX Address"

[table.fixtures.fields.fixture_type]
type = "string"
display_name = "Instrument Type"

[table.fixtures.fields.position]
type = "string"
display_name = "Position"

[table.fixtures.fields.color]
type = "string"
display_name = "Color/Gel"

[table.fixtures.fields.wattage]
type = "number"
display_name = "Wattage"
```

### Field Types

| Type        | Description                            | Example values                |
| ----------- | -------------------------------------- | ----------------------------- |
| `string`    | UTF-8 text                             | `"Jane Doe"`, `"FOH"`        |
| `number`    | 64-bit float                           | `42`, `3.14`, `0`            |
| `boolean`   | True or false                          | `true`, `false`              |
| `timestamp` | ISO 8601 datetime                      | `"2026-02-05T14:30:00Z"`    |
| `blob`      | Reference to content-addressed asset   | `"blake3:abc123..."`         |

CRDT-enabled fields add a `crdt` property:

```toml
[table.cues.fields.notes]
type = "string"
crdt = "text"               # Text CRDT for collaborative editing

[table.cues.fields.tags]
type = "list"
crdt = "list"                # List CRDT for ordered primitives
item_type = "string"
```

### Field Properties

| Property       | Type    | Required | Description                                         |
| -------------- | ------- | -------- | --------------------------------------------------- |
| `type`         | string  | Yes      | One of the field types above                        |
| `display_name` | string  | No       | Human-readable label for UI                         |
| `description`  | string  | No       | Tooltip/help text                                   |
| `required`     | boolean | No       | Whether the field must have a value (default false) |
| `default`      | any     | No       | Default value for new records                       |
| `shared_key`   | string  | No       | Suggested shared key mapping (see below)            |
| `crdt`         | string  | No       | CRDT mode: `"text"` or `"list"`                     |
| `item_type`    | string  | No       | Element type for list fields                        |

### Inline Schema (Alternative)

For simple modules with one or two tables, the schema can live directly in `manifest.toml` instead of a separate file:

```toml
[module]
name = "tags"
version = "1.0.0"
display_name = "Tags"
description = "Taggable labels for any record"

[table.tags]
display_name = "Tags"

[table.tags.fields.tag_name]
type = "string"
required = true

[table.tags.fields.color]
type = "string"
default = "#888888"
```

If both `manifest.toml` and `schema/tables.toml` contain table declarations, the schema file takes precedence.

---

## Shared Key Suggestions

Shared keys are the mechanism for cross-module data sharing. When two modules declare fields with the same `shared_key`, those fields can map to the same underlying data on an entity.

### Declaration

Shared key mappings in module schemas are **suggestions, not automatic bindings**. The module developer is expressing intent: "this field is semantically the same as this common concept."

```toml
# In the contacts module
[table.contacts.fields.full_name]
type = "string"
shared_key = "name"           # "I think this is the same as 'name' everywhere"

# In the scheduling module
[table.attendees.fields.attendee_name]
type = "string"
shared_key = "name"           # "I also think this is 'name'"
```

### Confirmation Flow

On module adoption or first table-linking, the system presents suggested field mappings based on shared key overlap. The user reviews and confirms each mapping:

1. User adopts a new module (or links two tables for the first time).
2. System scans both modules' shared key declarations.
3. System presents matches: "Contacts has 'full_name' and Scheduling has 'attendee_name' -- both suggest they map to 'name'. Should these be the same data?"
4. User confirms or rejects each suggested mapping.
5. Confirmed mappings activate: both fields read/write the same shared key on the entity.
6. Rejected mappings remain as separate namespaced fields.

### After Confirmation

Confirmed mappings behave identically to the old auto-bound shared key model:

- Writing to either field writes to the shared key at entity level.
- Both modules see the same value.
- Conflict detection treats them as a single field.

### User-Created Mappings

Users can create field mappings beyond what modules suggest:

- Map any field from one module to any field in another.
- Create custom shared keys that no module declared.
- Override module suggestions (e.g., map `full_name` to a custom `legal_name` shared key instead of `name`).

### Standard Shared Keys

The following shared keys are recommended conventions. Module authors should prefer these when applicable:

| Shared Key    | Type      | Description               |
| ------------- | --------- | ------------------------- |
| `name`        | string    | Display name              |
| `email`       | string    | Email address             |
| `phone`       | string    | Phone number              |
| `description` | string    | Long-form description     |
| `notes`       | string    | General notes             |

### Type Validation

If two modules suggest the same shared key with conflicting types (e.g., `name: string` vs `name: number`), the conflict is surfaced during the adoption confirmation flow. The user must resolve it before the mapping can be confirmed.

---

## Table Compatibility Hints

Tables declare `compatible_tables` as hints for the table-linking UI. These replace the old kind-compatibility constraints on facets.

```toml
[table.cues]
compatible_tables = ["sm.cues", "scheduling.events"]
```

**Behavior:**

- When a user links tables, compatible tables are shown as suggested matches.
- Incompatible combinations (no hint overlap) trigger a warning: "These tables are not typically linked. Proceed?"
- Warnings are non-blocking. The user decides.
- Compatibility hints are never enforced as hard constraints.

### Per-Entity Table Membership

Table linking is not always 1:1. Individual records can belong to multiple tables independently:

- LX 11 (called cue) is in both the Lighting Cues table and the SM Cues table.
- LX 11.1 (auto-follow) is in Lighting Cues only.
- SM Cue 15 (sound cue) is in SM Cues and Sound Cues, not in Lighting Cues.

Table-level linking ("all contacts are also attendees") is a convenience shortcut. Per-entity membership is the fundamental mechanism. Rules can automate per-entity membership: "Cues in Lighting where is_called == true also appear in SM Cues."

---

## Capabilities

Modules request host capabilities in their manifest. Capabilities are granted per-user and enforced by the core runtime. A module cannot access resources it has not declared and the user has not approved.

### Capability Declaration

```toml
[capabilities]
network = [
    "osc:8000-8010",           # OSC on ports 8000-8010
    "http.client",             # Outbound HTTP requests
]
filesystem = [
    "read:imports/*",          # Read files matching glob
    "write:exports/*",         # Write files matching glob
    "watch:imports/*",         # Watch directory for changes
]
data = [
    "read:lighting.*",         # Read own module's data
    "write:lighting.*",        # Write own module's data
    "read:contacts.name",      # Read a specific field from another module
]
```

### Capability Categories

**Network**

| Capability              | Description                   |
| ----------------------- | ----------------------------- |
| `osc:<ports>`           | OSC listen/send on ports      |
| `http.client`           | Outbound HTTP requests        |
| `http.server:<port>`    | HTTP server on port           |
| `midi`                  | MIDI input/output             |

**Filesystem**

| Capability              | Description                   |
| ----------------------- | ----------------------------- |
| `read:<glob>`           | Read files matching glob      |
| `write:<glob>`          | Write files matching glob     |
| `watch:<glob>`          | Watch files for changes       |

**Data Access**

| Capability              | Description                   |
| ----------------------- | ----------------------------- |
| `read:<scope>`          | Read data in scope            |
| `write:<scope>`         | Write data in scope           |

Data scopes use dot notation: `lighting.*` (all lighting data), `contacts.name` (specific field), `*` (all data).

**UI**

| Capability              | Description                   |
| ----------------------- | ----------------------------- |
| `ui.navigate`           | Navigate views programmatically |
| `ui.notify`             | Show toast notifications      |
| `ui.dialog`             | Show blocking dialogs         |

### Capability Enforcement

- Lua scripts start with an empty global environment. The core selectively exposes modules based on declared and granted capabilities.
- Capabilities are checked at each operation, not just at script startup.
- If a required capability is revoked while a script runs, the script exits immediately and the user is notified.
- Session-mode overlays are preserved for review after capability revocation.

### No Implicit Capabilities

Modules with no `[capabilities]` section can only read and write their own table data and use always-available APIs (core query/mutation, JSON, logging, context). They cannot touch the filesystem, network, or other modules' data.

---

## Views

Views are TypeScript components registered with the core view system. They render UI for the module's data.

### View Registration

Each `.tsx` file in the `views/` directory exports a default component that the core registers automatically:

```typescript
// views/CueList.tsx
import { useTable, useField, SmartField } from "@openprod/sdk";

interface CueListProps {
  tableId: string;
}

export default function CueList({ tableId }: CueListProps) {
  const cues = useTable("lighting.cues", {
    sort: [{ field: "cue_number", direction: "asc" }],
  });

  return (
    <div>
      {cues.map((cue) => (
        <div key={cue.id}>
          <SmartField entity={cue.id} field="cue_number" />
          <SmartField entity={cue.id} field="label" />
          <SmartField entity={cue.id} field="intensity" />
        </div>
      ))}
    </div>
  );
}

// View metadata for registration
CueList.viewMeta = {
  id: "lighting.cue-list",
  displayName: "Cue List",
  description: "Spreadsheet-style cue list view",
  defaultTable: "lighting.cues",
};
```

### View Guarantees

- View crashes do not corrupt data. Crashed views show an error boundary with a retry option.
- Views are read-only by default. Mutations go through the core API (which routes through overlays and the oplog).
- Views receive data reactively. When underlying data changes (local edit, sync, script output), views re-render automatically.
- UI state (scroll position, selection, expanded/collapsed) is local-only and never syncs.

### View SDK

The `@openprod/sdk` package provides hooks and components for views:

| Export          | Purpose                                          |
| --------------- | ------------------------------------------------ |
| `useTable`      | Query records from a table with filtering/sorting |
| `useRecord`     | Subscribe to a single record by ID               |
| `useField`      | Subscribe to a single field value                 |
| `SmartField`    | Renders a field with mode switching (discrete/reference/query) |
| `useEdges`      | Query edges from/to an entity                    |
| `useMutation`   | Returns mutation functions for creating/updating records |
| `useOverlay`    | Access the current overlay state for preview      |
| `useConflicts`  | Subscribe to active conflicts on a record         |

### Custom Conflict Resolution UI

Modules may provide custom conflict resolution views for their data types. For example, a lighting module could show a side-by-side cue comparison instead of the generic field-level conflict UI.

---

## Scripts

Scripts are Lua files in the `scripts/` directory. They extend the Lua scripting engine defined in [scripts.md](scripts.md). Module scripts inherit the module's declared capabilities.

### Three Execution Modes

| Mode           | Trigger                          | Lifecycle                        | Example                              |
| -------------- | -------------------------------- | -------------------------------- | ------------------------------------ |
| **Manual**     | User clicks "Run" in UI         | Runs once, completes             | Import CSV, renumber cues, export    |
| **On-change**  | Field value changes (trigger)    | Runs once per triggering event   | Auto-set status, recalculate totals  |
| **Background** | User starts, runs continuously   | Runs until stopped or cancelled  | OSC listener, file watcher           |

### Script Declaration in Manifest

```toml
[script.import_lightwright]
display_name = "Import Lightwright File"
description = "Imports fixture data from a Lightwright XML export"
execution_mode = "manual"
session_mode = "session"               # session | autoCommit
on_error = "skip"                      # skip | abort | prompt

[script.osc_listener]
display_name = "OSC Listener"
description = "Listens for OSC messages from the lighting console"
execution_mode = "background"
session_mode = "autoCommit"
shutdown_timeout = "5s"

[script.auto_mark_called]
display_name = "Auto-Mark Called Cues"
description = "Sets is_called=true when a cue is added to SM cue list"
execution_mode = "on-change"
session_mode = "autoCommit"

[script.auto_mark_called.trigger]
on_field_change = "lighting.cues.is_called"
when = "new_value == true"
```

### Script Semantics

- Scripts produce operation bundles, never mutate state directly.
- Script output is staged in an overlay until the script completes successfully (in session mode).
- Failed scripts produce no ops. Partial output is discarded.
- Script failure surfaces an error to the user with context.
- Background scripts check `context.cancelled()` for clean shutdown.

See [scripts.md](scripts.md) for the full scripting API, async model, session modes, error handling, and core API reference.

---

## Structured Imports and Exports

Imports and exports are scripts with additional guarantees for data integrity.

- Imports run inside staging overlays by default.
- Imports produce explicit operations recorded in history only upon user commit.
- Import preview shows the overlay state before commit -- the user sees what will change.
- Exports operate on derived views (read-only).
- Failed imports produce no operations. Discarding an import overlay discards all imported data.
- Import source metadata (filename, format, timestamp) is preserved in operation attribution.

---

## Local-Only Modules

Local-only modules produce data that exists only for one user and never syncs to canonical state.

- Local-only module data is excluded from canonical sync.
- Local-only data may reference canonical entities.
- Local-only modules must not emit canonical operations.
- Local-only data is stored separately from canonical data.
- Local-only data follows the same operation/bundle model locally.
- Local-only modules function fully offline.

**Use cases:**

- Personal notes and annotations
- Scratch data and working calculations
- Private workflows and experiments
- User-specific display preferences

Declared in the manifest:

```toml
[module]
name = "my-notes"
version = "1.0.0"
display_name = "My Notes"
local_only = true
```

---

## Templates

Templates are pre-configured workspaces that bundle a set of modules with pre-confirmed field mappings and optional seed data. They provide zero-friction onboarding for common workflows.

### Template Structure

```
templates/
  stage-management/
    template.toml             # Template manifest
    modules/                  # Bundled module folders
      contacts/
      lighting/
      sm-cues/
      scheduling/
    seed/                     # Optional seed data
      sample_show.json
```

### Template Manifest

```toml
[template]
name = "stage-management"
display_name = "Stage Management"
description = "Complete stage management workspace with contacts, cues, and scheduling"

# Modules included in this template
modules = ["contacts", "lighting", "sm-cues", "scheduling"]

# Pre-confirmed field mappings — these skip the confirmation flow
[[field_mappings]]
modules = ["contacts", "scheduling"]
shared_key = "name"
fields = ["contacts.contacts.full_name", "scheduling.attendees.attendee_name"]

[[field_mappings]]
modules = ["contacts", "scheduling"]
shared_key = "email"
fields = ["contacts.contacts.email", "scheduling.attendees.email"]

[[field_mappings]]
modules = ["lighting", "sm-cues"]
shared_key = "name"
fields = ["lighting.cues.name", "sm-cues.cues.cue_name"]

[[field_mappings]]
modules = ["lighting", "sm-cues"]
shared_key = "notes"
fields = ["lighting.cues.notes", "sm-cues.cues.notes"]

# Pre-configured table links
[[table_links]]
source = "lighting.cues"
target = "sm-cues.cues"
rule = "is_called == true"            # Only called cues appear in SM list
```

### Template Behavior

- Creating a workspace from a template installs all listed modules.
- Pre-confirmed field mappings activate immediately (no confirmation prompt).
- Pre-configured table links are established automatically.
- Seed data (if present) is imported into an overlay for user review before commit.
- Users can modify any template-provided configuration after creation.
- Templates are not live links. Changing a template does not affect workspaces created from it.

---

## Edge Declarations

Modules declare edge types for relationships between their records and optionally across tables:

```toml
[edge.mounted_at]
display_name = "Mounted At"
source_table = "lighting.fixtures"
target_description = "Position or location"
ordered = false
properties = { unit_number = "string", circuit = "string" }

[edge.in_cue_list]
display_name = "In Cue List"
source_table = "lighting.cues"
ordered = true
properties = { call_text = "string", timing_override = "number" }
```

Edge declarations follow the same independence principle: a module can constrain edges to its own tables but uses hints (not hard constraints) for cross-module edge targets.

---

## Complete Module Example

A full contacts module demonstrating all components:

### `manifest.toml`

```toml
[module]
name = "contacts"
version = "1.0.0"
display_name = "Contacts"
description = "Contact management for production teams"
author = "Openprod Community"

[capabilities]
filesystem = ["read:imports/*.csv", "write:exports/*.vcf"]
data = ["read:contacts.*", "write:contacts.*"]

[script.import_csv]
display_name = "Import Contacts from CSV"
execution_mode = "manual"
session_mode = "session"
on_error = "skip"

[script.export_vcards]
display_name = "Export as vCards"
execution_mode = "manual"
session_mode = "session"
```

### `schema/tables.toml`

```toml
[table.contacts]
display_name = "Contacts"
description = "Production team contacts and personnel"
compatible_tables = ["scheduling.attendees", "casting.actors"]

[table.contacts.fields.full_name]
type = "string"
required = true
display_name = "Name"
shared_key = "name"

[table.contacts.fields.email]
type = "string"
display_name = "Email"
shared_key = "email"

[table.contacts.fields.phone]
type = "string"
display_name = "Phone"
shared_key = "phone"

[table.contacts.fields.role]
type = "string"
display_name = "Role"
description = "Production role (e.g., Stage Manager, Lighting Designer)"

[table.contacts.fields.department]
type = "string"
display_name = "Department"

[table.contacts.fields.notes]
type = "string"
display_name = "Notes"
shared_key = "notes"
crdt = "text"

[table.contacts.fields.emergency_contact]
type = "string"
display_name = "Emergency Contact"

[table.contacts.fields.photo]
type = "blob"
display_name = "Photo"
```

### `scripts/import_csv.lua`

```lua
function main()
    local path = args.path
    local content = core.await(fs.read(path))
    local rows = csv.parse(content)

    ui.progress.start("Importing contacts", { total = #rows })

    local created, skipped = 0, 0
    for i, row in ipairs(rows) do
        local existing = core.query_one(
            "table == 'contacts.contacts' AND full_name == ?",
            row.name
        )

        if existing then
            skipped = skipped + 1
        else
            core.create_record("contacts.contacts", {
                full_name = row.name,
                email = row.email,
                phone = row.phone,
                role = row.role,
                department = row.department,
            })
            created = created + 1
        end

        ui.progress.update(i)
    end

    ui.progress.complete()
    ui.toast(created .. " created, " .. skipped .. " skipped (duplicates)")
end
```

### `views/ContactList.tsx`

```typescript
import { useTable, SmartField, useMutation } from "@openprod/sdk";

export default function ContactList() {
  const contacts = useTable("contacts.contacts", {
    sort: [{ field: "full_name", direction: "asc" }],
  });
  const { createRecord } = useMutation("contacts.contacts");

  return (
    <div>
      <button onClick={() => createRecord({ full_name: "New Contact" })}>
        Add Contact
      </button>
      <table>
        <thead>
          <tr>
            <th>Name</th>
            <th>Email</th>
            <th>Role</th>
            <th>Department</th>
          </tr>
        </thead>
        <tbody>
          {contacts.map((c) => (
            <tr key={c.id}>
              <td><SmartField entity={c.id} field="full_name" /></td>
              <td><SmartField entity={c.id} field="email" /></td>
              <td><SmartField entity={c.id} field="role" /></td>
              <td><SmartField entity={c.id} field="department" /></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

ContactList.viewMeta = {
  id: "contacts.contact-list",
  displayName: "Contact List",
  description: "Table view of all contacts",
  defaultTable: "contacts.contacts",
};
```

---

## Internal Mapping: Tables to Entity/Facet Layer

Module authors do not need to understand this section. It is provided for core implementers.

| Module author writes             | Core does internally                              |
| -------------------------------- | ------------------------------------------------- |
| Declare table `contacts.contacts` | Register facet type `contacts.contacts`           |
| Create a record in Contacts      | Create entity, attach `contacts.contacts` facet   |
| Query "my contacts table"        | Query: all entities with `contacts.contacts` facet |
| Link Contacts and Attendees      | User confirms field mappings, shared keys activate |
| Record in both Contacts and Attendees | One entity, two facets                        |
| Unlink Contacts from Attendees   | Detach facets, copy data to standalone entities    |
| Remove contacts module           | Facet data preserved, hidden from active views     |

---

## UI/UX Integration

These elements do not affect correctness but are explicitly allowed and encouraged:

- Conflict indicators are derived UI state, rendered by the view system.
- Badges and indicators must reflect underlying semantic state.
- Modules may embed conflict and overlay components contextually.
- Modules may provide custom conflict resolution UI for their table types.
- UI state (expanded/collapsed, scroll position, selection) is local-only and non-syncing.

---

## Open Questions

- Module distribution and marketplace mechanics.
- Hot-reload of views at runtime (avoid restarts).
- Module signing and trust model.
- Versioning strategy for module schema changes in the field.
