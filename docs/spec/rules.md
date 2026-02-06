# Rules Specification

This document defines unified rules, automation configuration, and cycle detection.

---

## Unified Rules

All automation is expressed as rules. Rules replace the previous concepts of match rules, presence rules, parity rules, and field mappings.

**Anchor invariant:** Rules are deterministic. Given the same state, a rule produces the same action. All rule actions are either proposed for user confirmation or auto-executed based on configuration.

### Rule Structure

```yaml
rule:
  name: "Actors are Contacts"
  when: <query>
  propose: <action>
```

### Rule Types

**Table Membership (replaces presence rules):**

```yaml
rule:
  name: "Called cues appear in SM cues table"
  when: table == "lighting_cues" AND lighting.is_called == true
  propose:
    action: add_to_table
    table: sm_cues
    defaults:
      department: "lighting"
      cue_number: $source.lighting.cue_number
```

**Cross-Module Table Membership:**

```yaml
rule:
  name: "Actors are Contacts"
  when: table == "casting" AND casting.role == "actor"
  propose:
    action: add_to_table
    table: contacts
```

**Entity Matching (replaces match rules):**

```yaml
rule:
  name: "Match contacts by name"
  when: table == "contacts"
  match_on: [name, email]
  propose:
    action: merge_entities
```

### Match Rule Null Handling

**Anchor invariant:** `null == null` is NOT a match. Any null value is considered "missing data."

- Match rules require non-null values on all match keys
- An entity with `email: null` will not match another entity with `email: null`
- Both are considered "missing data," not identical values
- This prevents accidental merges of unrelated entities with incomplete data

---

## Query Scope

The `when` clause IS the query scope. No separate scoping concept needed.

```yaml
# By table membership (recommended)
rule:
  when: table == "contacts"
  match_on: [name, email]
  propose: merge_entities

# By facet presence
rule:
  when: contacts.Contact exists
  match_on: [name]
  propose: merge_entities

# Complex custom query
rule:
  when: table == "contacts" AND department == "crew" AND NOT is_archived
  match_on: [name, email]
  propose: merge_entities
```

---

## Default Values on Attachment

Rules can specify initial values when adding entities to tables:

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

---

## Dynamic References in Rules

- `$source.<field>` -- field from entity that triggered the rule
- `$now` -- current timestamp
- `$actor` -- user who triggered the change
- `$literal("value")` -- explicit static value

---

## Settings and Automation Levels

Each rule can be configured:

| Setting | Options | Default |
| ------- | ------- | ------- |
| `auto_accept` | true / false | false (propose) |
| `on_condition_lost` | ignore / propose_remove | propose_remove |

---

## Triggering

- Rules are evaluated when relevant fields change
- Rules execute at most once per triggering event
- Rules must not self-trigger or create cycles

### Rule Authority

**Anchor invariant:** Rule authority is frozen at the triggering operation's HLC. Permission state is derived deterministically from the oplog at that HLC.

```
User action: SetField(task.status = "done") @ HLC 500
  -> Permission check at HLC 500: Does user have can_edit on status? Yes

Rule triggers: AddToTable(entity, "completed_tasks") (uses user's authority)
  -> NO additional permission check (rule inherits user's authority at HLC 500)
  -> Entity added to table even if user doesn't have direct permission on that table
```

**Atomicity:** The triggering operation and all rule-triggered operations form an atomic unit:
- Either all succeed (triggering op + rule ops commit together)
- Or all fail (if rule can't complete, triggering op is also rejected)

**Determinism:** All peers compute the same permission state at HLC 500, so all peers make the same success/failure decision. No race conditions.

### Rules in Overlay Context

**Anchor invariant:** Rules trigger in overlay context. Operations produced by rules go to the same destination as the triggering operation.

- When an overlay operation triggers a rule, the rule's output goes to the overlay
- This enables full preview: user sees what rules would fire before committing
- Rule-triggered operations are part of the overlay and subject to commit/discard
- On commit, rule-triggered operations become canonical along with user operations

**Query context:**

Rules query the same context as their triggering operation (see [query-language.md](query-language.md) for full algorithm):

| Triggering Operation | Rule Query Context |
|---------------------|-------------------|
| Canonical edit (no overlay) | Canonical state only |
| Overlay edit | Canonical + triggering overlay (merged) |

When triggered by an overlay operation, rules see the merged view:

| Query target | What rules see |
|--------------|----------------|
| Entity fields | Canonical value, overridden by overlay if modified |
| Entity existence | Includes entities created in overlay, excludes deleted |
| Edges | Includes edges created in overlay |
| Facets / table membership | Includes facets attached in overlay |

This ensures rules see the "world as it would be" if the overlay were committed, enabling accurate preview of rule behavior.

**Example:**
```yaml
# User creates entity in overlay, adds to contacts table
# Rule watches: table == "contacts"
# Rule queries overlay context -> sees the new entity
# Rule fires, adding entity to attendees table -> goes to same overlay
# On commit: entity creation + table memberships both become canonical
```

---

## Condition Changes

- If a condition becomes false after an entity was added to a table (e.g., `role` changes from "actor" to "designer"):
  - The entity is NOT automatically removed from the table
  - Default behavior: propose removal (surface choice to user)
  - Configurable: ignore (entity stays), auto_remove (not recommended)
- Rules help you set things up right; removal is always explicit

---

## Rule Cycle Detection

Rules must not self-trigger or create cycles. This section defines the detection algorithm that enforces this invariant.

**Anchor invariant:** Cycles are detected at rule creation/modification time. A rule that would introduce a cycle is rejected before it can execute.

### What Constitutes a Cycle

A cycle exists when a chain of rule triggers forms a loop:

| Cycle Type | Description | Example |
|------------|-------------|---------|
| Direct self-trigger | Rule A's action triggers Rule A | Rule writes to `status`, also watches `status` |
| Indirect cycle | Rule A triggers B triggers ... triggers A | A writes `status` -> B watches `status`, writes `priority` -> A watches `priority` |

```
Direct self-trigger:

    +-----------------+
    |     Rule A      |
    |                 |
    |  watches: name  |
    |  writes: name   |------+
    +-----------------+      |
            ^                |
            +----------------+

Indirect cycle:

    +-----------------+         +-----------------+
    |     Rule A      |         |     Rule B      |
    |                 |         |                 |
    | watches: status |<--------|  writes: status |
    | writes: priority|-------->|watches: priority|
    +-----------------+         +-----------------+
```

### Dependency Graph Model

The cycle detector builds a directed graph from rule definitions:

- **Nodes:** Each rule is a node
- **Edges:** An edge from Rule X to Rule Y exists if X can trigger Y

**Edge derivation:**

```
Rule X can trigger Rule Y  iff
    there exists field F such that:
        F in X.writes  AND  F in Y.watches
```

Where:
- `X.writes` = all fields that Rule X's action can modify
- `Y.watches` = all fields referenced in Rule Y's `when` clause

**Anchor invariant:** The dependency graph is derived purely from rule definitions. It does not require runtime state to construct.

### Field Analysis

To build accurate edges, the system analyzes each rule:

**Watched fields (from `when` clause):**

```yaml
# Example rule
when: table == "contacts" AND casting.role == "actor"
```

Watched fields: `[table_membership, casting.role]`

The query language grammar (see query-language.md) defines how field references are extracted:
- Direct field access: `field`, `namespace.field`
- Table membership: `table == "..."` watches facet attachment
- Edge traversal fields: `edge->target.field`
- Existence checks: `facet exists` watches facet attachment

**Written fields (from action):**

| Action Type | Written Fields |
|-------------|----------------|
| `add_to_table` | Table facet attachment state, plus any `defaults` fields |
| `set_field` | The specified field |
| `merge_entities` | All fields on merged entities |

### When Detection Runs

**Static analysis at rule creation/modification:**

```
+-----------------+     +-----------------+     +-----------------+
|  Rule submitted |---->| Build/update    |---->| Detect cycles   |
|  (create/edit)  |     | dependency graph|     | (find SCCs)     |
+-----------------+     +-----------------+     +-----------------+
                                                        |
                        +-------------------------------+-------------------------------+
                        |                               |                               |
                        v                               v                               v
                 +-------------+                +-------------+                +-------------+
                 |  No cycles  |                |Cycle detected|                |Cycle detected|
                 |  -> accept  |                |  -> reject   |                |(with override)|
                 +-------------+                +-------------+                +-------------+
```

**Why runtime detection is insufficient:**

1. **Unbounded execution:** A cycle could run indefinitely before detection
2. **Inconsistent state:** Partial cycle execution leaves state in undefined condition
3. **Resource exhaustion:** Cycles consume CPU, storage (if persisted), and memory
4. **User confusion:** Difficult to diagnose which rule caused the problem after the fact

Runtime detection serves only as a safety net, not primary defense.

### Detection Algorithm

The algorithm detects strongly connected components (SCCs) using Tarjan's algorithm:

```
function detectCycles(rules):
    graph = buildDependencyGraph(rules)
    sccs = tarjanSCC(graph)

    cycles = []
    for scc in sccs:
        if |scc| > 1:
            cycles.append(scc)  # Multiple nodes = cycle
        else if scc.node has edge to itself:
            cycles.append(scc)  # Self-loop

    return cycles

function buildDependencyGraph(rules):
    nodes = rules
    edges = []

    for each rule X in rules:
        writes = extractWrittenFields(X.action)
        for each rule Y in rules:
            watches = extractWatchedFields(Y.when)
            if writes intersection watches is not empty:
                edges.append(X -> Y)

    return Graph(nodes, edges)
```

**Complexity:** O(V + E) where V = number of rules, E = number of trigger relationships

### Error Handling

**When a cycle is detected:**

```yaml
CycleDetectionError:
  type: cycle_detected
  rules_involved:
    - rule_a_id
    - rule_b_id
    - rule_c_id
  cycle_path: "rule_a -> rule_b -> rule_c -> rule_a"
  shared_fields:
    - field: "status"
      written_by: rule_a_id
      watched_by: rule_b_id
    - field: "priority"
      written_by: rule_b_id
      watched_by: rule_a_id
  message: "Rule 'Auto-assign priority' would create a cycle with existing rules"
```

**User options:**

| Option | Description | Use Case |
|--------|-------------|----------|
| Reject (default) | Rule is not saved | Most cases |
| Acknowledge cycle | Save with `cycle_acknowledged: true` | Expert users who understand the risk |

**Anchor invariant:** Acknowledged cycles still have runtime safety nets. Acknowledgment means "I understand this might cycle" not "disable all protection."

### Runtime Safety Net

Even with static detection, runtime protection is required for:
- Rules with `cycle_acknowledged: true`
- Edge cases missed by static analysis
- Trigger cycles (see Scripts specification)

**Execution limits:**

```yaml
runtime_safety:
  max_rule_chain_depth: 1000    # Maximum rules triggered in sequence
  max_execution_time: 30s       # Maximum wall-clock time for rule chain
  max_field_writes_per_op: 1000 # Maximum field modifications per operation
```

These limits are set high to accommodate legitimate complex chains while preventing true runaway execution. Hitting these limits indicates either:
- A bug in static cycle detection (should be fixed)
- A deliberately massive chain (review the design)

**When limit exceeded:**

```yaml
RuntimeCycleError:
  type: cycle_limit_exceeded
  trigger_chain:
    - rule: rule_a_id
      field: status
      depth: 1
    - rule: rule_b_id
      field: priority
      depth: 2
    # ... truncated
  action: execution_halted
  state: rolled_back  # All changes from this operation reverted
  notification: user  # User/admin notified of potential issue
```

### Edge Cases

**Conditional triggers:**

Rules with conditional logic may not always trigger:

```yaml
rule:
  name: "Set priority for urgent items"
  when: status == "urgent"
  action:
    set_field: priority
    value: 1
```

This rule only writes `priority` when `status == "urgent"`. Static analysis must be conservative:

**Anchor invariant:** Static analysis assumes all conditional branches can execute. This may flag false positives but ensures no cycles are missed.

**Field-dependent writes:**

```yaml
rule:
  name: "Copy source field"
  when: table == "tasks"
  action:
    set_field: $source.target_field  # Dynamic field name
    value: $source.value
```

Dynamic field names cannot be statically analyzed:

| Approach | Behavior |
|----------|----------|
| Conservative | Assume rule can write any field -> reject if any cycle possible |
| Permissive | Require explicit field declaration for cycle checking |

The system uses the conservative approach by default.

**Cross-module rule interactions:**

Modules may define rules that interact through mapped fields:

```
Module A: writes "status" (mapped field)
Module B: watches "status" (mapped field)
Module B: writes "contacts.priority" (namespaced)
Module A: watches "contacts.priority" (cross-module read)
```

**Anchor invariant:** Cycle detection operates on the unified rule set across all modules. Module boundaries do not exempt rules from cycle detection.

The dependency graph must include:
- All workspace-level rules
- All module-provided rules active in the workspace
- Rules from all enabled modules, even if defined in separate manifests

---

## Configuration Hierarchy

Configuration cascades from module defaults to workspace overrides to per-entity exceptions.

**Anchor invariant:** Modules ship with sensible defaults. Users customize at workspace level. Per-entity overrides are rare but possible.

### Override Cascade

```
Module defaults
    | (overridden by)
Workspace config
    | (overridden by)
Per-entity overrides (rare)
```

### Module Defaults

Modules ship with sensible defaults:
- Field mapping suggestions
- Table membership rules
- Default automation settings

### Workspace Config

Users customize at workspace level:
- Override field mappings
- Define rules
- Configure automation levels

### Per-Entity Overrides

Rare, but possible:
- Merge exceptions
- Entity-specific configurations

---

## Open Questions

- Rules that create edges in addition to table memberships
- Cycle detection with dynamic field names: should the system require explicit field declarations, or always assume worst-case?
- Should acknowledged cycles have configurable depth limits per-rule, or use global limits only?
- How should cycle detection errors surface in the UI? Inline editor feedback vs. separate validation step?
- Should cycle analysis results be cached and incrementally updated, or recomputed on each rule change?
- Cross-workspace rule dependencies: if workspaces share modules, can rules in different workspaces create cycles?
