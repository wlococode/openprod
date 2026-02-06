# Scripts Specification

This document defines the Lua scripting API, async model, sessions, triggers, and capabilities.

---

## Scripts

Scripts are the automation layer for Openprod, enabling both module developers and end users to create workflows that emit auditable operations.

**Anchor invariant:** Scripts emit operations like manual user actions. Scripts execute in sessions (overlay transactions) for safe preview and atomic commit.

### Language Choice: Lua

Scripts are written in **Lua 5.4**, chosen for:

- **Maturity** -- 30+ years, battle-tested in thousands of applications
- **Async via coroutines** -- Native support for background tasks
- **Lightweight** -- ~300KB runtime, fast startup
- **Cross-platform** -- Runs on desktop, mobile, and web (via WASM)
- **Sandboxed** -- Core controls exactly what scripts can access
- **Approachable** -- Simple syntax, extensive learning resources

### Script Types

| Type | Author | Location | Editable | Updates |
|------|--------|----------|----------|---------|
| **Module scripts** | Module developers | Bundled with module | Read-only | With module |
| **User scripts** | End users | Workspace | Fully editable | Manual |

Module scripts may call native Rust functions (exposed as capabilities) for performance-critical operations like XML parsing.

---

## Async Model: Coroutines

Lua's coroutines provide cooperative multitasking without callback complexity.

### How It Works

```
+-------------------------------------------------------------+
|                         Lua Script                          |
|                                                             |
|   local data = core.await(fs.read("show.lw6"))             |
|                           |                                 |
|                           +---> coroutine.yield(future)     |
|                                                             |
|   -- script pauses here, waiting for I/O --                 |
|                                                             |
|   -- I/O completes, script resumes with result --           |
|   process(data)                                             |
+-------------------------------------------------------------+
                              |
                              | yield / resume
                              v
+-------------------------------------------------------------+
|                    Rust Async Runtime (tokio)                |
|                                                             |
|   1. Script yields with future handle                       |
|   2. Rust awaits the actual I/O operation                   |
|   3. I/O completes                                          |
|   4. Rust resumes coroutine with result                     |
+-------------------------------------------------------------+
```

### The `core.await()` Pattern

Scripts use `core.await()` for any async operation:

```lua
-- File I/O
local content = core.await(fs.read("show.lw6"))

-- Network
local socket = core.await(osc.listen(8000))
local msg = core.await(socket:recv())

-- Timers
core.await(core.sleep(1000))  -- milliseconds

-- HTTP
local response = core.await(http.get("https://api.example.com"))
```

**Script authors write linear, synchronous-looking code.** The complexity of async is hidden in the runtime.

### Script Execution Modes

| Mode | Lifecycle | Example | V1 Status |
|------|-----------|---------|-----------|
| **Manual (one-shot)** | User-triggered, runs once, completes | Import CSV, renumber cues | **V1** |
| **On-change / Trigger** | Fires automatically when a field changes | Renumber cues on reorder, log cue start | **V1** |
| **Background (long-running)** | Runs continuously until stopped | OSC listener, file watcher, scheduled tasks | **Post-v1 / Deferred** |

> **Note:** Background/long-running script execution (including continuous listeners, file watchers, and scheduled/periodic execution) is deferred to post-v1. V1 supports manual (user-triggered) scripts and on-change trigger scripts only.

```lua
-- Manual (one-shot) script
function main()
    local csv = core.await(fs.read(args.path))
    import_contacts(csv)
    core.commit()
    -- script ends
end
```

### Script Cancellation (Post-v1 / Deferred)

> **Note:** Script cancellation is primarily relevant to background scripts and is deferred to post-v1 along with background script execution. One-shot scripts that exceed execution time limits are terminated by the runtime.

**Cancellation sources:**
- User stops the script manually
- Permission revoked
- Workspace closed
- App shutdown

**Checking for cancellation:**

```lua
-- In long-running loops, check context.cancelled()
while not context.cancelled() do
    local msg = core.await(socket:recv())
    process(msg)
end

-- After loop exits, script continues to on_shutdown (if defined)
```

**Cleanup with on_shutdown:**

```lua
function main()
    local socket = core.await(osc.listen(8000))

    while not context.cancelled() do
        local msg = core.await(socket:recv())
        handle_osc(msg)
    end
end

-- Optional: called after main() exits (normal completion or cancellation)
function on_shutdown()
    log.info("OSC listener shutting down")
    -- Cleanup resources, close connections, etc.
end
```

**Behavior:**
- `context.cancelled()` returns `true` when cancellation is requested
- `core.await()` calls check cancellation and may exit early
- `on_shutdown()` is called after `main()` exits, regardless of exit reason
- `on_shutdown()` timeout is configurable (default 5s, max 60s)
- If `on_shutdown()` is not defined, script exits immediately

**Cancellation and bundle atomicity:**

Scripts run in overlay mode by default. Cancellation behavior:

| Scenario | Outcome |
|----------|---------|
| Cancel during overlay mode | Overlay discarded, nothing persists |
| Cancel during autoCommit mode | Completed bundles stay, in-flight bundle rolled back |
| `on_shutdown()` completes in time | Its bundles commit |
| `on_shutdown()` exceeds timeout mid-bundle | In-flight bundle rolled back |

Bundle atomicity always holds--no partial bundles ever persist.

---

## Script Sessions (Overlay Transactions)

**Anchor invariant:** Scripts execute in sessions that control operation routing and commitment.

### Session Modes

| Mode | Behavior | Use Case |
|------|----------|----------|
| `session` (default) | Operations accumulate in overlay; user reviews before commit | Imports, bulk edits |
| `autoCommit` | Operations write directly to canonical | Real-time listeners, trusted automation |

### Session Semantics

- Script opens session on start (or uses canonical if `autoCommit`)
- Operations route to session destination
- Session ends when script completes (success or error)
- Overlay sessions are atomic: commit succeeds entirely or fails entirely
- Operations in overlay are visible to queries within same script

### Streaming Operations

- Scripts emit operations incrementally (not one batch at end)
- User sees live preview as operations accumulate
- Progress indicators update during execution
- Background scripts continue emitting until stopped

---

## Configuration

```toml
[script.renumber_cues]
execution_mode = "session"      # session | autoCommit
on_error = "skip"               # skip | abort | prompt
isolation = "snapshot"          # snapshot | live
shutdown_timeout = "5s"         # max 60s, default 5s
```

### Error Handling Model

Scripts use **try/catch + manifest defaults**:

1. **Manifest declares per-category defaults:**
   ```toml
   [script.my_import]
   on_error = "skip"              # Global default
   # Future: per-category overrides
   # on_permission_denied = "abort"
   # on_validation_error = "skip"
   ```

2. **Scripts use try/catch for custom handling:**
   ```lua
   local ok, err = pcall(function()
       core.set_field(entity_id, "field", value)
   end)
   if not ok then
       log.warn("Skipping: " .. tostring(err))
   end
   ```

3. **Unhandled errors fall through to manifest defaults**

Scoped error handlers (per-operation mode switching) may be added in future versions if scripts become complex enough to need them.

---

## Triggers

**Anchor invariant:** Triggers are user-configured rules that automatically execute scripts when conditions are met.

### Trigger Types

| Type | Description | Example |
|------|-------------|---------|
| **Field-change** | Execute when field changes | `on_field_change = "cue.status"` |
| **Conditional** | Execute when condition met | `when = "new_value == 'running'"` |
| **Scoped** | Find related entity via edge | `scope = "scene"` |

### Trigger Configuration

```toml
[trigger.renumber_on_reorder]
on_field_change = "lighting.cue.order_index"
scope = "scene"
run_script = "renumber_cues"
params = { scene_id = "$scope_entity_id" }
mode = "session"

[trigger.cue_started]
on_field_change = "lighting.cue.status"
when = "new_value == 'running'"
run_script = "log_cue_start"
mode = "autoCommit"
```

### Scope and Edge Traversal

The `scope` field specifies an edge type to traverse from the triggering entity to find a related "scope entity."

**Example:** When a Cue's `order_index` changes, find the Scene it belongs to:

```
Cue (triggering entity)
  |
  +--[scene]--> Scene (scope entity)
```

**How it works:**

1. A Cue entity's `order_index` field changes (someone reordered cues)
2. The trigger has `scope = "scene"` -- follow the "scene" edge from the Cue
3. The Scene entity at the other end becomes the "scope entity"
4. `$scope_entity_id` resolves to that Scene's entity ID
5. The script receives `scene_id` and can query all cues in that scene

**Trigger-specific references:**

| Reference | Description |
|-----------|-------------|
| `$source` | The entity whose field changed (the Cue) |
| `$source.<field>` | Field value from the triggering entity |
| `$scope_entity_id` | Entity ID at the end of the scope edge (the Scene) |
| `$old_value` | Previous value of the changed field |
| `$new_value` | New value of the changed field |

**Note:** `scope` must be a valid edge type from the triggering entity. If the edge doesn't exist for a particular entity, the trigger does not fire for that entity.

### Trigger Cycle Detection

**Anchor invariant:** Trigger cycles are detected at configuration time. A trigger that would create a cycle is rejected.

Detection uses Tarjan's SCC algorithm, same as rules:
- Builds dependency graph: Trigger -> Script -> writes field -> Trigger
- Cross-references rules (a rule can trigger a trigger's watched field)
- Runtime safety net: max depth 1000, max time 30s

---

## Capabilities

**Anchor invariant:** Capabilities control what scripts can access. Scripts can only call functions explicitly exposed by the core.

### Capability Model

Lua starts with an empty global environment. The core selectively exposes modules:

```rust
// Rust: create sandboxed Lua environment
fn create_script_env(lua: &Lua, caps: &Capabilities) -> Result<()> {
    // Always available (no capability required)
    lua.globals().set("core", create_core_module(lua)?)?;
    lua.globals().set("json", create_json_module(lua)?)?;
    lua.globals().set("log", create_log_module(lua)?)?;
    lua.globals().set("context", create_context_module(lua)?)?;
    lua.globals().set("args", script_args)?;

    // Capability-gated
    if caps.has("fs.read") {
        lua.globals().set("fs", create_fs_module(lua)?)?;
    }
    if caps.has("osc") {
        lua.globals().set("osc", create_osc_module(lua)?)?;
    }
    if caps.has("midi") {
        lua.globals().set("midi", create_midi_module(lua)?)?;
    }
    // Script cannot access anything not explicitly exposed
    Ok(())
}
```

### Always-Available Modules

These modules require no capability declaration:

| Module | Purpose |
|--------|---------|
| `core` | Query, mutation, session, conflict operations |
| `json` | `json.encode(table)`, `json.decode(string)` |
| `log` | `log.debug()`, `log.info()`, `log.warn()`, `log.error()` |
| `context` | Trigger context, user info, timestamps |
| `args` | Script arguments passed from trigger or invocation |
| `require` | Module imports (see Module System section) |

All other modules (fs, osc, midi, http, ui) require explicit capabilities.

### Capability Declaration

**Module capabilities:**

```toml
[module.lighting]
capabilities = [
    "net.osc:8000-8010",        # OSC on ports 8000-8010
    "net.http.client",          # HTTP requests
    "data.write:lighting.*",    # Write lighting facets
    "ui.navigate",              # Navigate views
    "ui.notify"                 # Show notifications
]
```

**User script capabilities:**

```toml
[script.my_import]
capabilities = [
    "fs.read:imports/*",
    "data.write:contacts.*"
]
```

### Capability Types

| Capability | Description |
|------------|-------------|
| `net.osc:<ports>` | OSC listen/send on ports |
| `net.http.client` | HTTP client requests |
| `net.http.server:<port>` | HTTP server on port |
| `net.midi` | MIDI input/output |
| `fs.read:<glob>` | Read files matching glob |
| `fs.write:<glob>` | Write files matching glob |
| `fs.watch:<glob>` | Watch files for changes |
| `data.read:<facet>` | Read facet data |
| `data.write:<facet>` | Write facet data |
| `ui.navigate` | Navigate views/entities |
| `ui.notify` | Show toasts/notifications |
| `ui.dialog` | Show blocking dialogs |

### Permission Revocation

**Anchor invariant:** If required permissions are revoked while a script runs, the script exits immediately.

- Permissions checked on each operation
- Revocation terminates script cleanly
- Error logged with context
- User notified
- Session-mode overlays preserved for review

> **Note:** Role-based permissions for script execution are deferred to post-v1. In V1, all users can execute any script. The capability system (which controls what APIs a script can access) is still enforced in V1.

---

## Native Capabilities (Rust Extensions)

For performance-critical operations, modules can expose Rust functions to Lua.

### Example: Lightwright XML Parsing

```rust
// Rust: register native capability
fn register_lightwright_capability(lua: &Lua) -> Result<()> {
    let lightwright = lua.create_table()?;

    lightwright.set("parse", lua.create_async_function(|_, path: String| async move {
        // Fast Rust XML parsing
        let content = tokio::fs::read_to_string(&path).await?;
        let data = parse_lightwright_xml(&content)?;
        Ok(data)  // Returns Lua table
    })?)?;

    lightwright.set("write", lua.create_async_function(|_, (path, data): (String, Table)| async move {
        let xml = serialize_lightwright_xml(&data)?;
        tokio::fs::write(&path, xml).await?;
        Ok(())
    })?)?;

    lua.globals().set("lightwright", lightwright)?;
    Ok(())
}
```

```lua
-- Lua: use native capability
local lw_data = core.await(lightwright.parse("show.lw6"))

for _, fixture in ipairs(lw_data.fixtures) do
    local existing = core.query_one(
        "table == 'lighting_fixtures' AND unit_number == ?",
        fixture.unit
    )
    if existing then
        core.set_field(existing.id, "channel", fixture.channel)
    else
        local entity = core.create_entity({
            table = "lighting_fixtures",
            fields = {
                unit_number = fixture.unit,
                channel = fixture.channel
            }
        })
    end
end

core.commit()
```

**Pattern:** Rust handles heavy lifting (parsing, encoding); Lua orchestrates logic.

---

## Error Handling

**Anchor invariant:** Script errors are handled according to configuration, not silently ignored.

### Error Modes

| Mode | Behavior |
|------|----------|
| `skip` | Skip failed operation, continue execution |
| `abort` | Exit immediately, discard overlay |
| `prompt` | Ask user to skip or abort |

### Error Types

| Error | Description |
|-------|-------------|
| `EntityNotFound` | Entity deleted or doesn't exist |
| `FieldNotFound` | Field doesn't exist on facet |
| `PermissionDenied` | Missing capability or permission |
| `InvalidOperation` | Violates schema constraints |
| `NetworkError` | External service unavailable |
| `Timeout` | Exceeded execution time limit |

### Error Handling in Lua

```lua
-- Protected call with error handling
local ok, err = pcall(function()
    core.set_field(entity_id, "field", value)
end)

if not ok then
    log.warn("Failed to set field: " .. tostring(err))
    -- handle gracefully
end

-- Or let the runtime handle it (respects on_error config)
core.set_field(entity_id, "field", value)  -- may skip/abort/prompt
```

---

## Isolation Levels

**Anchor invariant:** Scripts may execute while canonical state changes. Behavior depends on isolation level.

| Level | Behavior |
|-------|----------|
| `snapshot` (default) | Script sees canonical state at session start |
| `live` | Script re-queries current canonical + overlay on each query |

```lua
-- Snapshot mode: sees state at script start
local cues = core.query("table == 'lighting_cues'")
-- Even if peer syncs new cues, this list doesn't change

-- Live mode: sees current state
local cues = core.query_live("table == 'lighting_cues'")
-- Includes any cues synced since script started
```

---

## Conflict Interaction

**Anchor invariant:** Script operations are subject to normal conflict detection. Scripts do not auto-resolve conflicts.

- Script operations in overlay can conflict with canonical
- Conflicts detected on commit
- User resolves before commit completes
- Script operations have no special priority

### Bulk Conflict Detection

When script produces >100 conflicting operations:
- System detects bulk conflict pattern
- UI offers "choose one state" instead of individual resolution
- User can: accept all script ops, accept all peer ops, or resolve manually

---

## Core API

### Query Operations

```lua
-- Query entities by table membership
local contacts = core.query("table == 'contacts'")
local fixture = core.query_one("table == 'lighting_fixtures' AND channel == ?", 42)

-- Query with live isolation
local cues = core.query_live("table == 'lighting_cues'")

-- Field access
local channel = core.get_field(entity_id, "lighting.channel")

-- Schema introspection
local tables = core.schema.list_tables()
local fields = core.schema.get_fields("lighting_fixtures")
```

### Mutation Operations

```lua
-- Create entity in a table
local entity = core.create_entity({
    table = "lighting_fixtures",
    fields = {
        unit_number = "A1",
        channel = 1,
        position = "FOH"
    }
})

-- Set field
core.set_field(entity.id, "channel", 42)

-- Delete entity
core.delete_entity(entity.id)

-- Add/remove from tables
core.add_to_table(entity.id, "sm_fixtures", { department = "lighting" })
core.remove_from_table(entity.id, "sm_fixtures")

-- Attach/detach facets (lower-level)
core.attach_facet(entity.id, "lighting.Fixture", { channel = 1 })
core.detach_facet(entity.id, "lighting.Fixture", { preserve = true })

-- Edges
core.create_edge("mounted_at", fixture_id, position_id, {
    unit_number = "A1"
})
core.delete_edge(edge_id)
```

### CRDT Field Operations

Scripts edit CRDT fields through a high-level API. The runtime translates these to `ApplyCRDT` operations. See [crdt.md](crdt.md) for CRDT semantics.

```lua
-- Text CRDT operations
core.text.insert_at(entity_id, "description", 0, "New intro: ")
core.text.delete_range(entity_id, "description", 50, 60)
core.text.replace_range(entity_id, "description", 10, 20, "replacement")

-- Get current text value
local text = core.get_field(entity_id, "description")  -- Returns rendered string

-- List CRDT operations (for primitive lists)
core.list.insert(entity_id, "tags", 2, "urgent")       -- Insert at index
core.list.append(entity_id, "tags", "review")          -- Append to end
core.list.remove(entity_id, "tags", "done")            -- Remove by value
core.list.remove_at(entity_id, "tags", 0)              -- Remove by index

-- Get current list value
local tags = core.get_field(entity_id, "tags")  -- Returns array
```

**Full replacement:** Scripts can still use `set_field` for full replacement:

```lua
-- Replaces entire CRDT state (loses edit history)
core.set_field(entity_id, "description", "Completely new text")
core.set_field(entity_id, "tags", { "tag1", "tag2" })
```

### Ordered Edge Operations

Scripts create and reorder ordered edges through dedicated APIs. See [ordered-edges.md](ordered-edges.md) for semantics.

```lua
-- Create ordered edge (insert at end)
core.create_ordered_edge("in_cue_list", cue_id, list_id, {
    properties = { call_text = "GO" }
})

-- Create ordered edge (insert at specific position)
core.create_ordered_edge("in_cue_list", cue_id, list_id, {
    after = prev_edge_id,        -- Insert after this edge
    properties = { call_text = "STANDBY" }
})

-- Create ordered edge (insert at start)
core.create_ordered_edge("in_cue_list", cue_id, list_id, {
    before = first_edge_id,      -- Insert before this edge
    properties = { call_text = "WARN" }
})

-- Move ordered edge to new position
core.move_ordered_edge(edge_id, {
    after = new_prev_edge_id
})

-- Query ordered edges (returns in position order)
local cues = core.query_ordered_edges(list_id, "in_cue_list")
for _, edge in ipairs(cues) do
    print(edge.source_id, edge.properties.call_text)
end
```

### Identity Operations

```lua
-- Merge entities
core.merge_entities(entity_a, entity_b, {
    name = "Jane Doe"  -- resolution for conflicting field
})

-- Split entity
core.split_entity(source_id, {
    facet_distribution = {
        [new_id_1] = { "lighting.Fixture" },
        [new_id_2] = { "sm.Cue" }
    }
})
```

### Session Operations

```lua
-- Commit overlay to canonical
core.commit()

-- Discard overlay
core.discard()

-- List operations in current overlay
local ops = core.overlay.operations()
```

### Conflict Operations

```lua
-- Get open conflicts
local conflicts = core.get_conflicts()

-- Resolve conflict
core.resolve_conflict(conflict_id, chosen_value)
```

---

## Async APIs (Capability-Gated)

### File System

```lua
-- Read file (requires fs.read capability)
local content = core.await(fs.read("path/to/file.txt"))

-- Write file (requires fs.write capability)
core.await(fs.write("path/to/output.txt", content))

-- Watch directory (requires fs.watch capability)
local watcher = core.await(fs.watch("imports/"))
while not context.cancelled() do
    local event = core.await(watcher:next())
    if event.type == "modified" then
        handle_file_change(event.path)
    end
end
```

### OSC

```lua
-- Listen for OSC (requires net.osc capability)
local socket = core.await(osc.listen(8000))

while not context.cancelled() do
    local msg = core.await(socket:recv())

    if msg.path == "/cue/fire" then
        local cue_num = msg.args[1]
        local cue = core.query_one(
            "table == 'lighting_cues' AND number == ?",
            cue_num
        )
        core.set_field(cue.id, "status", "running")
        ui.toast("Cue " .. cue_num .. " fired")
    end
end

-- Send OSC
core.await(osc.send("192.168.1.100", 9000, "/cue/go", { 42 }))
```

### MIDI

```lua
-- Open MIDI input (requires net.midi capability)
local midi_in = core.await(midi.open_input("Port 1"))

while not context.cancelled() do
    local event = core.await(midi_in:recv())

    if event.type == "control_change" and event.cc == 20 then
        local intensity = event.value / 127.0
        core.set_field(active_cue_id, "intensity", intensity)
    end
end

-- Send MIDI
core.await(midi.send_note(1, 60, 127))  -- channel, note, velocity
```

### HTTP

```lua
-- HTTP client (requires net.http.client capability)
local response = core.await(http.get("https://api.example.com/data"))
local data = json.decode(response.body)

local response = core.await(http.post("https://api.example.com/update", {
    headers = { ["Content-Type"] = "application/json" },
    body = json.encode({ status = "complete" })
}))

-- HTTP server (requires net.http.server capability)
local server = core.await(http.serve(3000))

while not context.cancelled() do
    local req = core.await(server:accept())

    if req.path == "/trigger" then
        activate_preset(req.params.id)
        req:respond(200, "OK")
    else
        req:respond(404, "Not Found")
    end
end
```

### Timers

```lua
-- Sleep (delay execution)
core.await(core.sleep(1000))  -- milliseconds

-- Periodic task (runs every hour) -- Post-v1: background/scheduled execution deferred
-- while not context.cancelled() do
--     export_backup()
--     core.await(core.sleep(3600 * 1000))
-- end

-- Timeout pattern
local result = core.await(core.race(
    http.get("https://slow-api.example.com"),
    core.sleep(5000)  -- 5 second timeout
))
if result.timeout then
    log.warn("Request timed out")
end
```

**Note:** All timing uses the coroutine pattern (`core.await`). There is no callback-based timer API--this keeps the async model consistent and code flow linear.

---

## UI Operations

```lua
-- Navigation (requires ui.navigate capability)
ui.navigate.to_view("contacts")
ui.navigate.to_entity(entity_id)
ui.navigate.filter({ status = "active" })
ui.navigate.sort("name", "asc")

-- Notifications (requires ui.notify capability)
ui.toast("Import complete!")
ui.toast.warn("3 duplicates found")
ui.toast.error("Connection failed")

-- Progress
ui.progress.start("Importing contacts", { total = 1000 })
ui.progress.update(237)
ui.progress.complete()

-- Dialogs (requires ui.dialog capability)
local confirmed = core.await(ui.dialog.confirm("Delete 50 entities?"))
local input = core.await(ui.dialog.prompt("Enter cue number:"))
local choice = core.await(ui.dialog.select("Choose option:", { "A", "B", "C" }))
```

---

## Context & Utilities

```lua
-- Script context
local trigger_entity = context.trigger_entity()   -- What triggered this?
local trigger_field = context.trigger_field()     -- Which field changed?
local old_value = context.old_value()             -- Previous value
local new_value = context.new_value()             -- New value
local user = context.user()                       -- Who is running this?
local timestamp = context.hlc_time()              -- Current HLC timestamp

-- Cancellation (for background scripts)
local should_stop = context.cancelled()           -- Has cancellation been requested?

-- Script arguments (passed from trigger or manual invocation)
local scene_id = args.scene_id
local dry_run = args.dry_run or false

-- Logging
log.debug("Processing entity: " .. entity.id)
log.info("Import complete")
log.warn("Duplicate detected")
log.error("Failed to connect")
```

---

## Example: Module Script (OSC Listener) (Post-v1)

> **Note:** This example demonstrates a background/long-running script pattern, which is deferred to post-v1.

```lua
-- modules/lighting/scripts/osc_listener.lua
-- Listens to lighting console and updates cue status

function main()
    local port = args.port or 8000
    local socket = core.await(osc.listen(port))

    log.info("OSC listener started on port " .. port)

    while not context.cancelled() do
        local msg = core.await(socket:recv())

        if msg.path == "/cue/fire" then
            local cue_num = msg.args[1]
            local cue = core.query_one(
                "table == 'lighting_cues' AND number == ?",
                cue_num
            )

            if cue then
                core.set_field(cue.id, "status", "running")
                core.set_field(cue.id, "fired_at", context.hlc_time())

                ui.navigate.to_entity(cue.id)
                ui.toast("Cue " .. cue_num .. " fired")
            else
                log.warn("Cue not found: " .. cue_num)
            end

        elseif msg.path == "/cue/stop" then
            local cue_num = msg.args[1]
            local cue = core.query_one(
                "table == 'lighting_cues' AND number == ?",
                cue_num
            )

            if cue then
                core.set_field(cue.id, "status", "stopped")
            end
        end
    end
end

function on_shutdown()
    log.info("OSC listener stopped")
end
```

---

## Example: User Script (Renumber Cues)

```lua
-- workspace/scripts/renumber_cues.lua
-- Renumbers cues in a scene sequentially

function main()
    local scene_id = args.scene_id

    -- Query cues in the lighting cues table, ordered by position
    local cues = core.query(
        "table == 'lighting_cues' AND scene == ? ORDER BY order_index",
        scene_id
    )

    if #cues == 0 then
        ui.toast.warn("No cues found in scene")
        return
    end

    ui.progress.start("Renumbering cues", { total = #cues })

    for i, cue in ipairs(cues) do
        core.set_field(cue.id, "number", i)
        ui.progress.update(i)

        if i % 10 == 0 then
            log.debug("Renumbered " .. i .. " cues")
        end
    end

    ui.progress.complete()
    ui.toast(#cues .. " cues renumbered")

    -- Session mode: user reviews and commits
    -- AutoCommit mode: already committed
end
```

**Trigger configuration:**

```toml
[trigger.renumber_on_reorder]
on_field_change = "lighting.cue.order_index"
scope = "scene"
run_script = "renumber_cues"
params = { scene_id = "$scope_entity_id" }
mode = "session"
```

---

## Example: Lightwright Sync

```lua
-- modules/lighting/scripts/lightwright_sync.lua
-- Bidirectional sync with Lightwright XML

function main()
    local lw_path = args.path

    -- Parse Lightwright file (native Rust capability)
    local lw_data = core.await(lightwright.parse(lw_path))

    ui.progress.start("Syncing fixtures", { total = #lw_data.fixtures })

    local created, updated, unchanged = 0, 0, 0

    for i, lw_fixture in ipairs(lw_data.fixtures) do
        local existing = core.query_one(
            "table == 'lighting_fixtures' AND unit_number == ?",
            lw_fixture.unit_number
        )

        if existing then
            -- Check for changes
            local changed = false

            if existing.channel ~= lw_fixture.channel then
                core.set_field(existing.id, "channel", lw_fixture.channel)
                changed = true
            end

            if existing.dimmer ~= lw_fixture.dimmer then
                core.set_field(existing.id, "dimmer", lw_fixture.dimmer)
                changed = true
            end

            if changed then
                updated = updated + 1
            else
                unchanged = unchanged + 1
            end
        else
            -- Create new fixture
            core.create_entity({
                table = "lighting_fixtures",
                fields = {
                    unit_number = lw_fixture.unit_number,
                    channel = lw_fixture.channel,
                    dimmer = lw_fixture.dimmer,
                    fixture_type = lw_fixture.instrument_type,
                    position = lw_fixture.position
                }
            })
            created = created + 1
        end

        ui.progress.update(i)
    end

    ui.progress.complete()

    local msg = string.format(
        "Sync complete: %d created, %d updated, %d unchanged",
        created, updated, unchanged
    )
    ui.toast(msg)
    log.info(msg)
end

-- Background file watcher version (Post-v1: background scripts deferred)
-- function watch()
--     local lw_path = args.path
--     local watcher = core.await(fs.watch(lw_path))
--
--     log.info("Watching Lightwright file: " .. lw_path)
--
--     while not context.cancelled() do
--         local event = core.await(watcher:next())
--
--         if event.type == "modified" then
--             log.info("Lightwright file changed, syncing...")
--             main()
--         end
--     end
-- end
--
-- function on_shutdown()
--     log.info("File watcher stopped")
-- end
```

---

## Cross-Platform Considerations

### Runtime by Platform

| Platform | Lua Runtime | Async Backend |
|----------|-------------|---------------|
| Desktop | mlua (Rust) | tokio |
| Mobile | mlua (cross-compiled) | tokio |
| Web | wasmoon (WASM) | JavaScript Promises |

### Platform-Specific Capabilities

| Capability | Desktop | Mobile | Web |
|------------|---------|--------|-----|
| `fs.read/write` | Native FS | Native FS | OPFS / File API |
| `net.osc` | UDP sockets | UDP sockets | WebSocket bridge |
| `net.midi` | Native MIDI | Limited | WebMIDI |
| `net.http` | Native | Native | Fetch API |

**Scripts are portable.** Platform differences are hidden in capability implementations.

---

## Determinism & Replay

- Scripts must be deterministic given same canonical state
- Use `context.hlc_time()` for timestamps (not system time)
- External data access recorded in operation metadata
- Non-deterministic inputs acceptable if metadata captures them

---

## Multi-Peer Script Execution

**Anchor invariant:** Multiple peers can run the same script simultaneously. No coordination or locking.

- Each peer's operations attributed to that peer
- Operations merge on sync (conflicts detected if overlap)
- Operations NOT deduplicated (attribution preserved)
- Idempotent state derivation avoids false conflicts

---

## Script Versioning

- Module scripts versioned with module
- Script hash recorded in operation metadata
- User scripts: manual versioning (git recommended)

---

## Explicit Non-Goals

- Scripts must not infer user intent or make autonomous decisions
- Scripts must not auto-resolve conflicts
- Scripts must not bypass permissions or capabilities
- Scripts must not hide operations from history
- Scripts must not directly manipulate UI structure (only navigate/notify)

---

## Module System

Scripts can import shared code using `require()`. Modules enable code reuse within modules, across modules, and within user workspaces.

### Require Syntax

| Pattern | Resolves to |
|---------|-------------|
| `require("./utils")` | Relative to current file |
| `require("../shared/helpers")` | Relative with parent traversal |
| `require("lighting/helpers")` | `modules/lighting/exports/helpers.lua` |
| `require("workspace/utils")` | `workspace/modules/utils.lua` |

### Resolution Rules

1. Paths starting with `./` or `../` resolve relative to the current file
2. Otherwise, the first path component is the namespace:
   - Module name -> looks in `modules/{name}/exports/{rest}.lua`
   - `workspace` -> looks in `workspace/modules/{rest}.lua`

### Module Caching

Modules are cached per script execution. Multiple requires return the same table:

```lua
local a = require("lighting/helpers")
local b = require("lighting/helpers")
assert(a == b)  -- same table instance
```

Cache is cleared when the script ends. Background scripts retain cache for their lifetime.

### Error Handling

`require()` throws on failure. Use `require.try()` for optional dependencies:

```lua
-- Throws if not found
local csv = require("csv-parser/parse")

-- Returns nil, error if not found
local yaml, err = require.try("yaml-parser/parse")
if yaml then
    data = yaml.parse(content)
else
    log.info("YAML not available, using JSON")
    data = json.decode(content)
end
```

### Error Messages

| Failure | Message |
|---------|---------|
| File not found | `module not found: "lighting/helpers" (tried modules/lighting/exports/helpers.lua)` |
| Module not installed | `module not installed: "csv-parser"` |
| Syntax error | `syntax error in "lighting/helpers": [line]:[col]: [message]` |
| Circular dependency | `circular require: a.lua -> b.lua -> a.lua` |

### Circular Dependencies

Circular dependencies are detected at runtime and error immediately. The runtime maintains a "currently loading" set; if a module is required while already loading, an error is thrown with the full cycle path.

### Capability Integration

Modules execute with the caller's capabilities. A module cannot access capabilities the calling script lacks. If a module calls `fs.read()` but the caller lacks the `fs.read` capability, a runtime error occurs.

### Authoring Modules

Modules return a table of exports (standard Lua pattern):

```lua
-- exports/helpers.lua
local M = {}

function M.format_name(first, last)
    return first .. " " .. last
end

function M.validate_email(email)
    return email:match("^[^@]+@[^@]+%.[^@]+$") ~= nil
end

return M
```

Locals not in the return table are private to the module.

### Init Files

`require("lighting")` resolves to `modules/lighting/exports/init.lua` if it exists.

---

## Open Questions

1. **Script testing** -- Simulation/mock framework for testing scripts
2. **Debugging** -- Breakpoints, step-through, variable inspection
3. **Profiling** -- Performance analysis for slow scripts
4. **Hot reload** -- Reloading background scripts without restart
5. **Script marketplace** -- Sharing user scripts between workspaces
