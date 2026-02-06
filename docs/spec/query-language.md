# Query Language Specification

This document defines the formal grammar, operators, and semantics of the query language used throughout the system.

---

## Overview

The query language is used in multiple contexts:

- **Rule conditions** (`when` clauses)
- **Entity filtering** (views, searches)
- **Edge traversal** (relationship queries)
- **Table membership queries** (which entities are in which tables)
- **Trigger conditions** (field change detection)

**Anchor invariant:** Queries are pure predicates. A query never mutates state; it only evaluates to true or false for a given entity.

---

## Formal Grammar (PEG)

```peg
# Top-level query
Query           <- OrExpr EOF

# Logical operators (lowest precedence)
OrExpr          <- AndExpr (OR AndExpr)*
AndExpr         <- NotExpr (AND NotExpr)*
NotExpr         <- NOT? CompareExpr

# Comparison expressions
CompareExpr     <- ExistsExpr
                 / InExpr
                 / ChangesExpr
                 / MatchExpr
                 / LikeExpr
                 / RelationalExpr
                 / FieldExpr
                 / '(' OrExpr ')'

# Existence check
ExistsExpr      <- FieldPath EXISTS

# Set membership
InExpr          <- FieldPath IN ArrayLiteral

# Change detection (for triggers)
ChangesExpr     <- FieldPath CHANGES

# Pattern matching
MatchExpr       <- FieldPath MATCHES StringLiteral
LikeExpr        <- FieldPath LIKE StringLiteral

# Relational comparison
RelationalExpr  <- FieldPath RelOp Literal

# Bare field (truthy check)
FieldExpr       <- FieldPath

# Field access
FieldPath       <- EdgeTraversal? Identifier ('.' Identifier)*
EdgeTraversal   <- Identifier '->' FieldPath '.'

# Operators
RelOp           <- '==' / '!=' / '<=' / '>=' / '<' / '>'

# Literals
Literal         <- StringLiteral
                 / NumberLiteral
                 / BooleanLiteral
                 / NullLiteral
                 / ArrayLiteral
                 / Reference

ArrayLiteral    <- '[' (Literal (',' Literal)*)? ']'
StringLiteral   <- '"' [^"]* '"' / "'" [^']* "'"
NumberLiteral   <- '-'? [0-9]+ ('.' [0-9]+)?
BooleanLiteral  <- 'true' / 'false'
NullLiteral     <- 'null'

# Dynamic references
Reference       <- '$' Identifier ('.' Identifier)*

# Identifiers
Identifier      <- [a-zA-Z_][a-zA-Z0-9_]*

# Keywords (case-insensitive)
AND             <- 'AND' / 'and'
OR              <- 'OR' / 'or'
NOT             <- 'NOT' / 'not'
EXISTS          <- 'exists' / 'EXISTS'
IN              <- 'IN' / 'in'
LIKE            <- 'LIKE' / 'like'
MATCHES         <- 'MATCHES' / 'matches'
CHANGES         <- 'changes' / 'CHANGES'

# Whitespace (implicit, consumed between tokens)
_               <- [ \t\n\r]*
EOF             <- !.
```

---

## Operator Precedence

Operators are listed from highest to lowest precedence:

| Precedence | Operator(s) | Associativity | Description |
|------------|-------------|---------------|-------------|
| 1 | `()` | - | Parentheses (grouping) |
| 2 | `.` | Left | Field access |
| 3 | `->` | Left | Edge traversal |
| 4 | `NOT` | Right | Logical negation |
| 5 | `==`, `!=`, `<`, `>`, `<=`, `>=` | Left | Relational comparison |
| 6 | `exists`, `IN`, `LIKE`, `MATCHES`, `changes` | - | Unary/binary predicates |
| 7 | `AND` | Left | Logical conjunction |
| 8 | `OR` | Left | Logical disjunction |

**Anchor invariant:** Parentheses override all precedence rules. When in doubt, use parentheses for clarity.

---

## Comparison Operators

### Equality

| Operator | Description | Example |
|----------|-------------|---------|
| `==` | Equal to | `table == "contacts"` |
| `!=` | Not equal to | `status != "archived"` |

### Relational

| Operator | Description | Example |
|----------|-------------|---------|
| `<` | Less than | `age < 18` |
| `>` | Greater than | `priority > 5` |
| `<=` | Less than or equal | `cue_number <= 100` |
| `>=` | Greater than or equal | `start_time >= $now` |

### Type Coercion

- String comparison is lexicographic
- Numeric comparison uses numeric ordering
- Comparing incompatible types returns `false` (does not error)
- `null` is only equal to `null`

---

## Logical Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `AND` | Both conditions must be true | `table == "contacts" AND department == "crew"` |
| `OR` | Either condition must be true | `status == "active" OR status == "pending"` |
| `NOT` | Negates the condition | `NOT is_archived` |

### Short-Circuit Evaluation

- `AND` stops evaluating if left operand is `false`
- `OR` stops evaluating if left operand is `true`

---

## Table Membership

### Table Query

Tests whether an entity belongs to a specific table:

```
table == "contacts"
```

- Returns `true` if the entity has the facet associated with the specified table
- Syntactic sugar for checking the table's corresponding facet existence
- The `table` identifier is a reserved keyword in the query language

### Multiple Table Membership

```
table == "contacts" AND table == "attendees"
```

Returns `true` if the entity belongs to both tables (has both facets attached).

### Table Exclusion

```
table == "contacts" AND NOT table == "archived_contacts"
```

---

## Existence and Set Operators

### EXISTS

Tests whether a field or facet is present (non-null).

```
contacts.Contact exists
```

- Returns `true` if the facet `contacts.Contact` is attached to the entity
- Returns `true` if the field has any non-null value
- Returns `false` if the field is null or the facet is not attached

### IN

Tests set membership.

```
department IN ["lighting", "sound", "video"]
status IN ["active", "pending"]
```

- Returns `true` if the field value is in the array
- Array elements must be literals (no references)
- Empty array always returns `false`

---

## Pattern Matching Operators

### LIKE

SQL-style pattern matching with wildcards.

```
name LIKE "Jane%"
email LIKE "%@example.com"
```

| Wildcard | Matches |
|----------|---------|
| `%` | Zero or more characters |
| `_` | Exactly one character |

- Case-sensitive by default
- Escape wildcards with backslash: `\%`, `\_`

### MATCHES

Regular expression matching.

```
phone MATCHES "^\+1-\d{3}-\d{4}$"
name MATCHES "(?i)^john"
```

- Uses standard regex syntax
- Flags can be embedded: `(?i)` for case-insensitive
- Must match the entire field value (implicitly anchored)

---

## Field Access Syntax

### Direct Fields

Direct field names access fields in the default namespace:

```
name == "Jane Doe"
email == "jane@example.com"
status == "active"
```

### Namespaced Fields

Module-specific fields use dot notation:

```
casting.role == "actor"
lighting.cue_number > 50
contacts.internal_notes exists
```

### Facet Existence

Check if a facet is attached:

```
contacts.Contact exists
casting.Actor exists
```

---

## Edge Traversal

Query across entity relationships using arrow notation.

### Outgoing Edges

```
assigned_to->target.name == "Act 1"
```

Reads as: "Follow the `assigned_to` edge to its target, then check the target's `name` field."

### Incoming Edges

```
<-assigned_to.source.department == "cast"
```

Reads as: "Find incoming `assigned_to` edges, then check the source entity's `department` field."

### Edge Properties

```
assigned_to.character == "Juliet"
placed_on.call_type == "go"
```

Reads as: "Check the `character` property on the `assigned_to` edge itself."

### Chained Traversal

```
assigned_to->target.scheduled_at->target.venue == "Main Stage"
```

Multiple edge traversals can be chained.

### Existence in Edges

```
assigned_to->target exists
<-worn_by exists
```

Check if any matching edge exists.

---

## Literal Types

### Strings

```
"double quoted string"
'single quoted string'
"string with \"escaped\" quotes"
```

### Numbers

```
42
-17
3.14159
-0.5
```

### Booleans

```
true
false
```

### Null

```
null
```

### Arrays

```
["active", "pending", "review"]
[1, 2, 3, 4, 5]
[true, false]
```

Arrays are only valid as the right operand of `IN`.

---

## Dynamic References

References provide access to contextual values at evaluation time.

### Available References

| Reference | Description | Context |
|-----------|-------------|---------|
| `$source` | The entity that triggered evaluation | Rules, triggers |
| `$source.<field>` | Field from the source entity | Rules, triggers |
| `$now` | Current timestamp (HLC) | All contexts |
| `$actor` | Current user/actor ID | All contexts |
| `$literal("value")` | Explicit static value | Disambiguation |

### Examples

```yaml
# Rule that copies a field from the triggering entity
defaults:
  cue_number:
    value: $source.lighting.cue_number

# Query comparing to current time
when: due_date < $now

# Query checking current actor
when: assigned_to == $actor
```

---

## Special Expressions

### CHANGES (Trigger Condition)

The `changes` keyword is used in trigger contexts:

```
name changes
casting.role changes
```

**Anchor invariant:** `changes` only appears in trigger contexts. It cannot be used in general queries or rule `when` clauses.

- Evaluates to `true` when the specified field has been modified in the current operation
- Used to define when computed fields should recompute

---

## Query Examples

### Simple Queries

```
# All entities in the contacts table
table == "contacts"

# All entities with a Contact facet
contacts.Contact exists

# All non-archived contacts
table == "contacts" AND NOT is_archived
```

### Compound Queries

```
# Actors in the casting table who are in the cast department
table == "casting" AND casting.role == "actor"

# Crew members in the contacts table who are not archived
table == "contacts" AND department == "crew" AND NOT is_archived

# Active or pending items
status == "active" OR status == "pending"
```

### Using IN

```
# Contacts in specific departments
table == "contacts" AND department IN ["lighting", "sound", "video"]

# Cues with specific call types
table == "lighting_cues" AND call_type IN ["go", "standby", "warn"]
```

### Pattern Matching

```
# Names starting with "J"
name LIKE "J%"

# Valid phone numbers
phone MATCHES "^\+?[0-9\-\s]+$"
```

### Edge Queries

```
# Actors assigned to any scene
table == "casting" AND casting.role == "actor" AND assigned_to->target exists

# Cues placed on a specific page
table == "lighting_cues" AND placed_on->target.page_number == 42

# Entities with incoming "worn_by" edges
<-worn_by exists
```

### With References

```
# Entities I created
created_by == $actor

# Overdue items
due_date < $now AND status != "complete"

# Items in the same department as the triggering entity
department == $source.department
```

### Table Traversal

```
# Entities that are in both contacts and attendees tables
table == "contacts" AND table == "attendees"

# Contacts who are NOT in the attendees table
table == "contacts" AND NOT table == "attendees"

# All entities in any lighting-related table
table == "lighting_cues" OR table == "lighting_fixtures"
```

---

## Error Handling

### Parse Errors

| Error | Cause | Example |
|-------|-------|---------|
| `UnexpectedToken` | Invalid syntax | `table = "contacts"` (single `=`) |
| `UnterminatedString` | Missing closing quote | `name == "Jane` |
| `InvalidOperator` | Unknown operator | `name ~= "Jane"` |
| `MissingOperand` | Incomplete expression | `table ==` |

### Evaluation Errors

| Error | Cause | Behavior |
|-------|-------|----------|
| `FieldNotFound` | Field does not exist | Returns `false` (not error) |
| `TypeMismatch` | Incompatible comparison | Returns `false` (not error) |
| `InvalidRegex` | Malformed regex pattern | Returns error |
| `EdgeNotFound` | Edge type does not exist | Returns `false` (not error) |
| `CircularReference` | Self-referential query | Returns error |

**Anchor invariant:** Missing fields and type mismatches fail gracefully (return `false`). Only syntactically invalid queries or invalid regex patterns produce errors.

### Error Response Format

```yaml
QueryError:
  type: parse_error | evaluation_error
  code: UnexpectedToken | InvalidRegex | ...
  message: "Human-readable description"
  position:
    line: 1
    column: 15
  context: "table == \"contacts\" AND ^"  # caret shows error position
```

---

## Implementation Notes

### Indexing Hints

Certain query patterns can be optimized with indexes:

| Pattern | Recommended Index |
|---------|-------------------|
| `table == "..."` | Index on facet attachment (table membership) |
| `<field> == <literal>` | Index on specific field |
| `<facet> exists` | Facet attachment index |
| `<edge>->target exists` | Edge source index |

### Query Planning

- Queries are parsed once and cached
- Table membership checks and field existence checks are evaluated first (cheapest)
- `AND` clauses with indexed fields are evaluated early
- `OR` clauses may require multiple index scans

---

## Query Execution Context

Every query executes within a context that determines what state it sees.

**Anchor invariant:** Queries see a consistent snapshot of state. The snapshot is either pure canonical state, or canonical state merged with a single overlay. Overlays are isolated from each other.

### Context Definition

```yaml
QueryContext:
  canonical: <derived canonical state>
  overlay: <overlay_id> | null
```

### Context Determination

| Trigger | Query Context |
|---------|---------------|
| User direct edit (no overlay active) | `{ canonical, overlay: null }` |
| User edit in active overlay | `{ canonical, overlay: user_overlay_id }` |
| Rule triggered by canonical edit | `{ canonical, overlay: null }` |
| Rule triggered by overlay edit | `{ canonical, overlay: triggering_overlay_id }` |
| Script execution (session mode) | `{ canonical, overlay: script_overlay_id }` |
| Script in session mode | `{ canonical, overlay: script_session_overlay_id }` |
| Script in autoCommit mode | `{ canonical, overlay: null }` |

**Key principle:** A query in one overlay context cannot see another overlay's changes. Each overlay is isolated.

---

## Overlay Merging Algorithm

When a query executes with an overlay context, the system constructs a merged view by applying overlay changes on top of canonical state.

### Merge Strategy: Per-Field Override

The overlay is a sparse set of changes. Merging applies overlay values on top of canonical values at the field level.

```
function get_merged_value(entity_id, field_key, context):
    if context.overlay:
        overlay_value = context.overlay.get_field(entity_id, field_key)
        if overlay_value is not MISSING:
            return overlay_value

    return canonical.get_field(entity_id, field_key)
```

### Entity Existence

Entities may be created or deleted in an overlay. Deleted entities do not appear in query results.

```
function entity_exists(entity_id, context):
    if context.overlay:
        if context.overlay.has_delete(entity_id):
            return false  # Deleted in overlay -> hidden from queries
        if context.overlay.has_create(entity_id):
            return true   # Created in overlay -> visible to queries

    return canonical.entity_exists(entity_id)
```

### Entity Enumeration

When a query enumerates entities (e.g., `table == "contacts"`):

```
function get_matching_entities(query, context):
    entities = canonical.get_all_entities()

    if context.overlay:
        # Add entities created in overlay
        entities = entities.union(context.overlay.created_entities())

        # Remove entities deleted in overlay
        entities = entities.subtract(context.overlay.deleted_entities())

    # Apply query predicate to each entity using merged field values
    return [e for e in entities if evaluate(query, e, context)]
```

### Edge Handling

Edges in overlay can reference canonical entities. Cross-context traversal is valid.

```
function get_edges(entity_id, edge_type, direction, context):
    edges = canonical.get_edges(entity_id, edge_type, direction)

    if context.overlay:
        # Add edges created in overlay
        edges = edges.union(context.overlay.get_edges(entity_id, edge_type, direction))

        # Remove edges deleted in overlay
        edges = edges.subtract(context.overlay.deleted_edges(entity_id, edge_type, direction))

    return edges
```

**Edge traversal:** When traversing an edge, the target entity is resolved using the same merged context. An overlay edge pointing to a canonical entity is valid and traversable.

### Facet Existence

```
function facet_exists(entity_id, facet, context):
    if context.overlay:
        if context.overlay.has_detach_facet(entity_id, facet):
            return false
        if context.overlay.has_attach_facet(entity_id, facet):
            return true

    return canonical.facet_exists(entity_id, facet)
```

---

## Query Evaluation Example

```
Canonical state:
  Entity A: { tables: ["contacts"], status: "inactive" }
  Entity B: { tables: ["contacts"], status: "active" }

Overlay (user's uncommitted changes):
  SetField(A, status, "active")     # Modified
  CreateEntity(C)                   # Created
  AddToTable(C, "contacts")        # Added to contacts table
  SetField(C, status, "active")
  DeleteEntity(B)                   # Deleted

Query: table == "contacts" AND status == "active"
Context: { canonical, overlay: user_overlay }

Evaluation:
  Entity A: exists, table="contacts", status="active" (overlay) -> MATCH
  Entity B: deleted in overlay -> SKIP
  Entity C: created in overlay, table="contacts", status="active" -> MATCH

Result: [A, C]
```

---

## Permission Filtering

Query results are not filtered by permissions. Queries return entity references; permission checks occur when accessing specific fields.

**Rationale:** This keeps query logic simple and consolidates permission enforcement in field access. A user querying `table == "contacts"` receives all matching entity IDs, but attempting to read `hr.salary` on those entities will fail if the user lacks `can_read` on the `hr` facet.

---

## Open Questions

- Case-insensitive string comparison mode?
- Full-text search operator (`CONTAINS`)?
- Aggregate queries (COUNT, SUM, AVG)?
- Subqueries for complex edge patterns?
- Query result ordering (`ORDER BY`)?
- Query result limits (`LIMIT`, `OFFSET`)?
- Temporal queries ("as of" a specific time)?
- Query parameterization for prepared statements?
