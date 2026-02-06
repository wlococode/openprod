# Module System Design

This document defines the Lua module system for code sharing within plugins, across plugins, and within user workspaces.

---

## Overview

**Purpose:** Enable code sharing while maintaining the security and simplicity of the scripting system.

**Core principles:**

- **Standard Lua semantics** — `require()` returns what the module returns (typically a table)
- **Implicit namespacing** — First path component determines the source
- **Bundled dependencies** — Plugins vendor their dependencies; no shared resolution
- **Caller's capabilities** — Modules run in the importing script's capability context
- **Fail-fast** — Broken requires error immediately

**Require syntax:**

| Pattern | Resolves to |
|---------|-------------|
| `require("./utils")` | Relative to current file |
| `require("../shared/helpers")` | Relative with parent traversal |
| `require("lighting/helpers")` | `plugins/lighting/exports/helpers.lua` |
| `require("workspace/utils")` | `workspace/modules/utils.lua` |

**Resolution rules:**

1. Starts with `./` or `../` → relative path from current file
2. Otherwise → first component is plugin name (or literal `workspace`)

**Reserved name:** `workspace` cannot be used as a plugin name.

---

## Directory Structure

### Plugin layout

```
plugins/
  lighting/
    plugin.toml           # Plugin manifest
    exports/              # Public modules (importable by others)
      helpers.lua
      xml/
        parse.lua
        format.lua
    scripts/              # Plugin scripts (not importable)
      import_lightwright.lua
      osc_listener.lua
    internal/             # Internal modules (not importable by others)
      validation.lua
```

### Workspace layout

```
workspace/
  scripts/                # User scripts
    my_import.lua
    cleanup.lua
  modules/                # User modules
    helpers.lua
    csv_utils.lua
```

### Visibility rules

| Location | Same plugin | Other plugins | User scripts |
|----------|-------------|---------------|--------------|
| `exports/` | Yes | Yes | Yes |
| `scripts/` | Yes (relative only) | No | No |
| `internal/` | Yes | No | No |
| `workspace/modules/` | n/a | No | Yes |

**Key point:** Only `exports/` is visible externally. `internal/` gives plugins a place for shared code that isn't part of their public API.

---

## Resolution Algorithm

**Step-by-step resolution for `require(path)`:**

```
1. If path starts with "./" or "../":
   → Resolve relative to current file's directory
   → Append ".lua" if no extension
   → File must exist, else error

2. Otherwise, split path on first "/":
   → namespace = first component
   → rest = remaining path (or "init" if empty)

3. If namespace == "workspace":
   → Look in workspace/modules/{rest}.lua
   → File must exist, else error

4. Otherwise namespace is a plugin name:
   → Plugin must be installed, else error
   → Look in plugins/{namespace}/exports/{rest}.lua
   → File must exist, else error
```

### Examples

| Require | From file | Resolves to |
|---------|-----------|-------------|
| `"./utils"` | `plugins/lighting/scripts/import.lua` | `plugins/lighting/scripts/utils.lua` |
| `"../exports/helpers"` | `plugins/lighting/scripts/import.lua` | `plugins/lighting/exports/helpers.lua` |
| `"lighting/helpers"` | anywhere | `plugins/lighting/exports/helpers.lua` |
| `"lighting/xml/parse"` | anywhere | `plugins/lighting/exports/xml/parse.lua` |
| `"workspace/utils"` | anywhere | `workspace/modules/utils.lua` |

### Init files

`require("lighting")` resolves to `plugins/lighting/exports/init.lua` if it exists.

---

## Runtime Behavior

### Caching

Modules are cached per script execution. Multiple requires return the same table:

```lua
local a = require("lighting/helpers")
local b = require("lighting/helpers")
assert(a == b)  -- same table instance
```

Cache is cleared when the script ends. Background scripts retain cache for their lifetime.

### Error handling

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

### Error messages

| Failure | Message |
|---------|---------|
| File not found | `module not found: "lighting/helpers" (tried plugins/lighting/exports/helpers.lua)` |
| Plugin not installed | `plugin not installed: "csv-parser"` |
| Syntax error | `syntax error in "lighting/helpers": [line]:[col]: [message]` |
| Circular dependency | `circular require: a.lua → b.lua → a.lua` |

### Circular dependency detection

Runtime maintains a "currently loading" set. If a module is required while already loading, error immediately with the full cycle path.

---

## Capability Integration

**Principle:** Modules execute with the caller's capabilities. No additional capability model.

```lua
-- scripts/import.lua (has fs.read capability)
local parser = require("csv-parser/parse")
local content = core.await(fs.read("data.csv"))  -- allowed
local data = parser.parse(content)               -- parser inherits fs.read
```

**What this means:**

- Modules cannot access capabilities the calling script lacks
- Modules don't declare their own capabilities
- If a module calls `fs.read()` but caller lacks capability, runtime error occurs
- This matches how functions work — no special scoping for modules

### Native capabilities in modules

Plugin-provided Rust functions (like `lightwright.parse()`) are exposed to the plugin's scripts and modules. Other plugins cannot access them unless explicitly re-exported:

```lua
-- plugins/lighting/exports/lightwright.lua
-- Re-exports native capability for other plugins (if desired)
return {
    parse = lightwright.parse,
    write = lightwright.write
}
```

This is explicit — plugins choose what to share.

---

## Module Authoring

### Standard pattern

Modules return a table of exports:

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

### Single-function modules

Modules can return a single function:

```lua
-- exports/parse_csv.lua
return function(content)
    local rows = {}
    for line in content:gmatch("[^\n]+") do
        table.insert(rows, split(line, ","))
    end
    return rows
end
```

```lua
local parse_csv = require("csv-parser/parse_csv")
local data = parse_csv(content)
```

### Private helpers

Locals not in the return table are private:

```lua
-- exports/helpers.lua
local function private_helper(x)
    return x * 2
end

local M = {}

function M.public_function(x)
    return private_helper(x) + 1
end

return M  -- private_helper not exposed
```

---

## Spec Updates Required

### Changes to `scripts.md`

1. Add new "Module System" section documenting:
   - Require syntax and resolution rules
   - Caching behavior
   - Error handling (`require` vs `require.try`)
   - Circular dependency detection

2. Update "Always-Available Modules" table to include `require`

3. Remove "Module system" from Open Questions

### Changes to `plugins.md`

1. Update "Plugin Anatomy" to document directory structure:
   - `exports/` — public modules
   - `internal/` — private modules
   - `scripts/` — executable scripts (unchanged)

2. Add note that `workspace` is a reserved plugin name

### Changes to `plugin.toml` manifest

No changes required. Convention-based (`exports/`) means no manifest updates needed.

---

## Future Considerations

These are explicitly **not** part of v1 but documented for future reference:

**Shared dependency resolution:** If needed later, add a `shared/` namespace prefix for workspace-resolved dependencies. Existing requires continue working unchanged.

**Explicit manifest exports:** Optional whitelist in `plugin.toml`:
```toml
[plugin.lighting]
exports = ["helpers", "xml/parse"]  # only these are importable
```

**Module hot reload:** Tied to broader hot reload story for background scripts.

**Module versioning:** Currently tied to plugin versioning. Cross-plugin requires get whatever version of the plugin is installed.

---

## Summary

| Feature | Description |
|---------|-------------|
| `require("./path")` | Relative imports within same plugin/workspace |
| `require("plugin/path")` | Cross-plugin imports from `exports/` |
| `require("workspace/path")` | User module imports from `workspace/modules/` |
| `require.try()` | Optional imports that return nil on failure |
| `exports/` directory | Convention for public plugin modules |
| `internal/` directory | Convention for private plugin modules |

**Not in v1:**

- Shared dependency resolution (plugins bundle their own deps)
- Explicit manifest exports (convention-based only)
- Module hot reload
- Module versioning beyond plugin versioning
