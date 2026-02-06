# Functionality and Behavior

This document explains what Openprod does, how it behaves, and why. It's written for stage managers, designers, technicians, and anyone who wants to understand the system without reading code or technical specifications.

**Status:** This document reflects validated architectural decisions through Feb 5, 2026.

---

## What is Openprod?

Openprod is a collaborative workspace for live entertainment production teams. It gives stage managers, designers, technicians, and operations staff a single place to work with production data -- schedules, cue sheets, contact lists, equipment inventories, notes -- without forcing everyone into a single workflow or tool.

### The Problem

Production teams juggle too many tools. Lighting has Vectorworks and Lightwright. Stage management has Word and Excel. Sound has their apps. Everyone digs through Slack and email trying to find what's "current."

The daily reality: stage managers calculate call times by hand for 50 people, lighting designers email Excel exports every night, technical directors build from outdated drafts. When something changes, every related document needs manual updates. Something always gets missed.

Much of this work is logically deterministic -- if you know who's in Scene 2 and when Scene 2 rehearses, you can calculate their call time -- but humans do it by hand, over and over, and mistakes happen.

### The Solution

Openprod treats the underlying data as the source of truth. The system stores relationships explicitly (this person is in this scene; this scene rehearses at this time), and information that depends on other information (call times, cue lists, reports) can be computed from that data using expressions you set up. When something changes, everything that depends on it updates -- but only if you've configured that computation.

A stage manager adds contacts to their Contacts table and events to their Schedule table. Both tables have a "name" field. Openprod notices the overlap and asks: "Contacts and Schedule both have a 'name' field. Should these be the same data?" The SM confirms, and now those fields stay in sync. No guessing, no hidden wiring -- just a clear decision the SM made.

### Core Principles

- **Offline and local-first** -- No subscription required. No cloud dependency. Your data lives on your machine and works without internet.
- **Explicit over implicit** -- The system never changes data on its own. All automation requires your configuration and approval.
- **Safety over convenience** -- When there's a conflict, surface it for human resolution. Never silently overwrite someone's work.
- **Module independence** -- Each module works on its own. Interoperability is opt-in, not required.
- **Deterministic and auditable** -- Every change is recorded. You can always see what changed, when, and by whom. All computed values are visible and traceable.

---

## Offline-First Collaboration

Offline is the default mode, not a fallback. When you open Openprod, you're working with a complete local copy of the workspace. Everything you do is saved immediately to local storage. There's no "save" button anxiety.

### Two Ways to Sync (V1)

V1 supports two sync modes, depending on your environment:

- **LAN session** -- Devices on the same network discover each other automatically and sync directly. No internet required. Perfect for rehearsal halls, theaters, and touring venues.
- **Offline** -- No sync at all. Changes accumulate locally. When you reconnect to LAN, everything merges.

Both modes use the same underlying approach: your device keeps a complete copy of the data, and sync means "send me any changes I don't have yet." You can switch between modes freely.

**Post-v1:** Cloud server sync (a central server your team connects to over the internet) is deferred to post-v1. A touring production will eventually be able to use cloud sync during pre-production, LAN sync during tech, and offline mode during performances.

**Key constraint:** Teams on isolated networks (lighting control networks, sound networks) without internet access can still sync on their local subnet. Openprod never assumes you have internet.

### Non-Blocking Sync

When you reconnect after working offline, or when another team member's changes arrive, the system does not freeze. You keep working. Changes integrate in the background. If something needs your attention -- a conflict, a significant change to something you're looking at -- it's surfaced visibly, but not intrusively.

### Conflicts Are Visible and Resolvable

When two people edit the same field while disconnected, both values are preserved. A conflict indicator appears. You see both sides of the conflict, understand who changed what and when, and choose a resolution. If you change your mind later, you can revisit. Nothing is silently overwritten. Nothing is lost.

**Example:** Lighting works Saturday, sound works Sunday. Monday morning, the stage manager sees "12 changes since you last synced" and "2 conflicts need resolution." They can review changes, see what each department did, and resolve conflicts one by one. They're never surprised.

---

## Tables, Records, and Fields

If you've used Excel, FileMaker, or Lightwright, you already understand Openprod's data model. Your data lives in **tables**. Each table has **records** (rows) and **fields** (columns).

### Tables

Each module you install adds tables to your workspace. A Contacts module gives you a Contacts table. A Lighting module gives you a Lighting Cues table. A Schedule module gives you an Events table.

When you click "Add Contact," you're adding a record to the Contacts table. When you click "New Cue," you're adding a record to the Lighting Cues table. That's it. No hidden abstractions.

### Fields

Every table has fields. Some fields are specific to that table (like a "shirt size" field in a Wardrobe table). Others might overlap with fields in other tables (like "name" appearing in both Contacts and Schedule). How those overlapping fields are handled is covered in the Field Mapping section below.

### A Record Can Belong to Multiple Tables

This is the key idea that makes Openprod flexible. A single record can appear in more than one table at the same time. When it does, the record shows up in both tables, and any shared data stays in sync.

**Example: The cue scenario**

In a theater production, a lighting cue that the stage manager calls belongs in two places:

- LX 11 (a called lighting cue) appears in both the **Lighting Cues** table AND the **SM Cues** table
- LX 11.1 (an auto-follow that fires automatically) appears in the **Lighting Cues** table ONLY -- the SM doesn't need to see it
- SM Cue 15 (a sound cue) appears in the **SM Cues** table AND the **Sound Cues** table, but NOT in Lighting Cues

Table membership is per-record. Not every lighting cue is an SM cue, and not every SM cue is a lighting cue. You control which records appear where.

**Example: People across departments**

Jane Doe is in the Contacts table (she's a company member) and the Wardrobe table (she needs costumes). One person, two tables. Her name and email stay consistent across both.

### How Records Get Into Tables

There are three ways a record ends up in a table:

1. **You create it there.** Click "Add Contact" in the Contacts table, and a new record is created in that table.

2. **You manually add it.** You find a record in one table and add it to another. "Add this contact to the Wardrobe table."

3. **A rule does it automatically.** You set up a rule: "Cues in the Lighting table where `is_called` is true should also appear in the SM Cues table." From then on, any called lighting cue automatically shows up in the SM's cue sheet. (You can also remove records from tables manually or through rules.)

### Table-Level Linking (A Shortcut)

Sometimes you want all records in one table to also appear in another. "Every contact is also a schedule attendee." This is a convenience shortcut -- instead of adding records one by one, you link the tables and all records flow through. Per-record membership is still the underlying mechanism, so you can always override individual records.

---

## Field Mapping: How Tables Share Data

When two tables both have a field called "name," that doesn't automatically mean they're the same data. Openprod lets modules suggest that certain fields should be shared, but you always confirm.

### How It Works

1. You install a Contacts module and a Schedule module.
2. Both modules have a "name" field.
3. Openprod notices the overlap and presents a suggestion: "Contacts and Schedule both have a 'name' field. Should these be the same data?"
4. You confirm: "Yes, those are the same."
5. From now on, editing a person's name in Contacts also updates their name in Schedule. It's the same underlying data.

If the modules had fields that don't obviously overlap -- say Contacts has "phone" and Schedule has "mobile" -- you could manually map those too. Or leave them separate. Your call.

### Why Confirmation Matters

Auto-binding fields sounds convenient, but it causes real problems. A "status" field in a Contacts table (active/inactive) is completely different from a "status" field in a Cue table (standby/go/complete). If those linked automatically, editing one would corrupt the other.

The principle: the system suggests potential matches, but it's always up to you to say "these fields are the same thing."

### Templates Can Pre-Confirm Mappings

Starter templates (like a "Stage Management" template) can come with field mappings already confirmed. You get a working setup out of the box without having to approve every mapping manually. You can always review and change these later.

### Private Fields

Not every field is shared. Each module can have fields that are private to that module. The Wardrobe module's "shirt size" field has no reason to appear in the Schedule module. Private fields stay in their own table.

---

## Smart Fields

Every field in Openprod has a small but powerful feature: you can switch how it gets its value. Click on any field and you'll see three modes:

### Discrete (The Default)

A plain value. You type "Jane Doe" into a name field, and that's the value. This is how fields work in any spreadsheet or database.

### Reference

The field points to a field on another record. Instead of typing a value, you say "use the value from that record's name field." If the source changes, this field changes with it.

**Example:** A quick-change tracking sheet needs the actor's name. Instead of retyping it, you set the name field to reference the actor's record in the Contacts table. If someone corrects the spelling of the actor's name in Contacts, the quick-change sheet updates too.

### Query (Expression) -- Post-V1

The field computes its value from an expression. This is where Openprod will handle things that used to require manual calculation -- but the expression language is deferred to post-v1.

**Example (post-v1):** A "call time" field that computes itself:

```
earliest_event_time(scenes_for(this.actor)) - 30 minutes
```

This says: "Find all the scenes this actor is in, get the earliest event time, and subtract 30 minutes." Whenever the schedule changes, the call time updates.

**Example (post-v1):** An "abbreviated name" field:

```
first_letter(this.name) + ". " + last_name(this.name)
```

"Jane Doe" becomes "J. Doe" automatically. If the name changes, the abbreviation updates.

**For V1:** Smart Fields support Discrete and Reference modes. Query mode requires the expression language, which is deferred to post-v1.

### What This Replaces

In the old design, there were separate concepts for "derived fields" (computed values) and "interface slots" (fields that pull data from somewhere else). Smart Fields replace both. A derived field is just a field in query mode (post-v1). An interface slot is just a field in reference mode. One concept instead of three.

### All Computation Is Visible

You can always see how a field gets its value. If a field is in query mode, the expression is right there -- click on it and read it. There are no hidden formulas. No background calculations you can't find. Every computed value can be traced back to the expression that produces it.

---

## Rules: Automatic Actions

All automation in Openprod is expressed as **rules**. Rules watch for conditions and take actions. Rules are scoped to tables -- each rule is associated with a specific table's data.

### Record Matching Rules

A matching rule identifies when two records in a table refer to the same real-world thing:

**Example:** "If two records in the Contacts table have the same name and email, suggest merging them."

When the system finds a match, it suggests a merge. You review and approve.

**Important:** Match rules require exact matches. "Jane Doe" and "J. Doe" will not trigger an automatic merge suggestion. The system never guesses at identity.

### Table Membership Rules

A membership rule controls when records should appear in additional tables:

**Example:** "Cues in the Lighting table where `is_called` is true should also appear in the SM Cues table."

This means whenever someone marks a lighting cue as "called," it automatically shows up in the stage manager's cue sheet. Auto-follows and other non-called cues stay only in the Lighting table.

### What Happens When Conditions Change?

If a cue's `is_called` field changes from true to false, the rule's condition is no longer met. The system lets you know: "LX 11 no longer meets the criteria for SM Cues. Remove it?" You decide.

Rules help you set things up right. Removal is always explicit.

### Merge and Split

**Manual merge:** If records don't match automatically but you recognize they're the same person (or cue, or prop), you can manually merge them. Select both records, choose "Merge," and see a preview. If field values differ, you choose which to keep.

After merging:
- One record exists with all data combined
- References from both original records still work
- Nothing is lost; merges can be undone via split

---

## Relationships Between Records

A **relationship** is a fact that two things are related in a specific way. When you assign an actor to a scene, you're creating a statement: "this actor participates in this scene."

Relationships have their own history, can be audited independently, and can carry extra information:

```
Relationship: assigned_to
  from: Jane (Contacts table)
  to: Scene 2 (Scenes table)
  details:
    character: "Juliet"
    entrance: "Enter from SR"
```

### Relationships Are Module-Owned

The Wardrobe module defines "assigned costume" relationships (actor to costume). The Schedule module defines "scheduled for" relationships (person to event). Each module controls when its relationships are created and removed.

### When a Record Is Deleted

When you delete a record, all relationships connected to it are also removed. This happens as a single action:

- Delete Costume #5
- Remove "assigned costume" link between Jane and Costume #5
- Remove "uses fabric" link between Costume #5 and Fabric #3

**Undo restores everything.** When you undo the deletion, the record and all its relationships come back -- unless the other record has also been deleted or the data has changed. In that case, relationship restoration is skipped gracefully.

No dangling links. Clean data. Reversible.

---

## Operations and History

All changes are recorded as **operations** in an append-only log. Operations cannot be altered once committed. The current state of your workspace is always the result of every operation that has been applied.

### Field-Level Granularity

Operations work at the field level. "Set Jane's name to Jane Smith" is one operation. This enables:

- Fine-grained conflict detection (two people editing different fields on the same record don't conflict)
- Field-level history and attribution
- Selective undo

### Bundles: Atomic Groups

Operations are grouped into **bundles** for atomicity. A bundle either fully commits or fully fails.

**The user interface determines bundling:**
- **Table view (spreadsheet-style):** Each field edit is a separate bundle (immediate commit)
- **Form view:** All edits in one bundle (you click "Save")

**Undo granularity follows bundling:**
- Table view: 3 edits = 3 separate undo actions
- Form view: 3 edits in 1 bundle = 1 undo action

### Undo and Redo

Each user has their own undo stack tracking their operations (not other users' operations).

**How undo works:**
- You hit Undo
- The system creates inverse operations for your last bundle
- **If no conflict:** Operations apply, change is reversed
- **If conflict detected:** Another user edited the same data after you. Undo is skipped with notification: "Cannot undo: this field was modified by another user." Your undo stack advances to the next operation.

**Undo stack:**
- Per-user (only your changes)
- Does not persist across app restarts
- Limited depth (50-100 operations)

---

## Conflicts and Resolution

A conflict happens when two people edit the same field while disconnected. If two fields are mapped together (same underlying data), editing either one can create a conflict.

### Conflicts Are Not Errors

Conflicts are expected in collaborative offline work. They signal that multiple valid intentions exist and need a human decision.

### How Conflicts Appear

You see a conflict indicator with:
- What changed ("Cue 42 timing")
- Who made each change ("Alex set 10:30, Jordan set 10:45")
- When changes were made
- Current values from all sides

### Resolving Conflicts

You choose one value:
- Pick Alex's value
- Pick Jordan's value
- Enter a new value entirely

The resolution is recorded in history. You can revisit and change the resolution later if needed.

### N-Way Conflicts

If three people edited the same field, you still see a list of competing values with authors and context, and choose one outcome. The system doesn't complicate things -- it's still "we got three different values, let's pick the one we're using."

---

## Overlays: Safe Experimentation

An **overlay** is a temporary sandbox where you can experiment without affecting your real data. Think of it as a rehearsal for your data changes.

### Creating an Overlay

You enter "staging mode" to experiment:
- The system creates a new overlay
- All your edits go to the overlay (not to the real data)
- Everything looks and works as if your changes were real -- but they're not committed yet

You see exactly what the workspace would look like if your changes went through.

### Overlays Are Local-Only

Overlays never sync to other team members. They exist only on your machine. Real data continues to update from team syncs while you're in an overlay.

### Overlays Behave Like Reality

While in an overlay:
- Queries update to reflect your experimental edits
- Computed fields (Smart Fields in query mode) recalculate
- Rules evaluate against your experimental state
- Conflicts appear if your edits would conflict with real data

It feels like making real changes -- because you are, just in isolation.

### Canonical Drift

While you're experimenting in an overlay, the real data continues to advance. Team members can sync changes. If someone else edits a field you're editing in your overlay:

- Your overlay value still shows (your experiment wins visually)
- A badge appears: "Real data changed to X while you were editing"
- You can see who changed it and when
- You decide:
  - **Ignore** (keep your value; committing will overwrite theirs)
  - **Accept theirs** (drop your edit for that field, keep their value)

You can selectively drop individual edits before committing -- this is how you exclude specific changes from an otherwise atomic commit.

### Committing an Overlay

When you commit:
- All remaining overlay edits become real, committed data (as a single atomic bundle)
- If conflicts are detected during commit, you resolve them before the commit completes
- The overlay is deleted
- Your changes are now part of the workspace and will sync to the team

### Discarding an Overlay

If you discard:
- The overlay is deleted
- Real data is unchanged
- No history written
- Nothing to undo

Overlays are disposable. They're a rehearsal, not a second performance.

---

## Scripts and Automation

Scripts are how you automate repetitive tasks, integrate external tools, and build custom workflows. Both module developers and end users can create scripts.

### Two Kinds of Scripts

**Module Scripts:**
- Written by module developers
- Bundled with the module
- Provide core functionality (imports, calculations, automations)
- Read-only (you can't edit them, but you can copy and modify)
- Updated when the module updates

**User Scripts:**
- Written by you
- Stored in your workspace
- Fully editable
- For custom automation and workflows

**All scripts are written in Lua** -- a simple, safe scripting language used in games, embedded systems, and creative tools worldwide. It's approachable, fast, and can't harm your system.

**V1 script modes:** Manual (user-triggered) and on-change (triggered by data changes). Background scripts (long-running listeners like OSC and file watchers) are deferred to post-v1.

### What Scripts Can Do

Scripts have access to everything you can do manually:

- **Read and write data** -- Query tables, update fields, create relationships
- **Automate workflows** -- Renumber cues, format names, calculate values
- **Integrate external tools** -- Listen for OSC from consoles, send MIDI, watch files
- **Process bulk data** -- Import 1000 contacts, renumber 500 cues, generate reports
- **React to changes** -- Automatically update related data when something changes
- **Navigate the UI** -- Show notifications, navigate to records, display progress

### Script Sessions: Safe Preview Before Commit

When a script runs, it works in a session (like an overlay):

```
Script starts  -->  Opens overlay
Script runs    -->  Operations accumulate (you see live preview)
                    "Importing contacts... 237/1000"
Script ends    -->  You review the overlay
                    You commit or discard
```

**You're always in control.** Scripts don't change real data until you approve.

**Example:** Import 1000 contacts from CSV
1. Script reads CSV, creates records
2. Operations accumulate in an overlay as the script runs
3. You see: "1000 contacts imported, 3 duplicates detected"
4. You review the overlay, fix duplicates
5. You commit -- all 1000 operations enter real history

### AutoCommit Mode (For Trusted Scripts)

If you trust a script, you can configure it to auto-commit:
```
Script runs  -->  Operations write directly to real data (no overlay)
Script ends  -->  Changes are live immediately
```

Use this for:
- Routine automations you don't need to review
- On-change scripts that should apply immediately

**Post-v1:** Background listeners (OSC from console, MIDI faders, file watchers) will use auto-commit mode when background scripts are available.

### Triggers: Automatic Script Execution

Scripts can run automatically when data changes.

**Example: Renumber cues when order changes**

You configure a trigger:
- **When:** cue order changes
- **Run:** renumber_cues script
- **Scope:** Only renumber cues in the affected scene

Now, whenever you drag cues to reorder them:
1. Trigger fires automatically
2. Script runs, renumbers cues
3. Overlay opens for you to review
4. You commit the changes

**Setting up triggers:**
- Right-click on a field, choose "Script triggers..."
- Choose when to trigger, which script to run
- Set execution mode (session with review, auto-commit, etc.)

**Conditional triggers:**
You can add conditions:
- **When:** cue status changes **to** "running"
- **Run:** log_cue_start script

Only fires when status becomes "running", not when it changes to anything else.

### Example: Module Script (OSC Integration) -- Post-V1

The lighting module will provide a script that listens to your console (requires background script mode, post-v1):

```lua
-- Listen for OSC messages from console
function main()
    local socket = core.await(osc.listen(8000))

    while true do
        local msg = core.await(socket:recv())

        if msg.path == "/cue/fire" then
            local cue_num = msg.args[1]
            local cue = core.query_one("table == 'lighting_cues' AND number == ?", cue_num)

            -- Update cue status
            core.set_field(cue.id, "status", "running")

            -- Navigate to show the cue
            ui.navigate.to_entity(cue.id)

            -- Notify user
            ui.toast("Cue " .. cue_num .. " started")
        end
    end
end
```

**What this does:**
- Listens for OSC on port 8000
- When the console sends "/cue/fire 42", the script updates Cue 42's status
- Navigates the UI to show the active cue
- Shows a toast notification

### Example: User Script (Renumber Cues)

You create a script to renumber cues:

```lua
function main()
    local scene_id = args.scene_id
    local cues = core.query("table == 'lighting_cues' AND scene == ? ORDER BY order_index", scene_id)

    ui.progress.start("Renumbering cues", { total = #cues })

    for i, cue in ipairs(cues) do
        core.set_field(cue.id, "number", i)
        ui.progress.update(i)
    end

    ui.progress.complete()
    ui.toast(#cues .. " cues renumbered")
end
```

**What this does:**
- Queries all cues in the scene (ordered by position)
- Updates each cue's number to match its position
- Shows a progress bar as it works
- Shows a toast when complete

**You see live preview:** As the script runs, operations appear in the overlay. You see exactly what will change before committing.

### Capabilities: What Scripts Can Access

Scripts request **capabilities** for external access:

**Network:**
- OSC (send/receive on specific ports)
- MIDI (input/output)
- HTTP (make requests, run webhook servers)

**Filesystem:**
- Read files (imports, config)
- Write files (exports, logs)
- Watch directories (auto-import when file changes)

**UI:**
- Navigate views
- Show notifications
- Display dialogs
- Show progress bars

**Data:**
- Read from tables
- Write to tables

**When a script requests a capability, you approve it:**
```
Script "osc_listener" wants to:
  - Listen on network port 8000 (OSC)
  - Write to lighting_cues.status field

Allow? [Always] [This Time] [Never]
```

**Capabilities keep you safe.** Scripts can only do what you've approved.

### Conflicts and Scripts

Scripts don't get special treatment for conflicts.

**If a script and a team member edit the same field:**
1. Script operations go to overlay
2. Team member's operations go to real data
3. When you commit the overlay, conflict detected
4. You choose: Script value, Team member's value, or New value

**Bulk conflicts:**

If a script renumbers 500 cues and a team member also renumbered them:
- System detects bulk conflict
- Instead of 500 individual conflicts, you see:
  ```
  Bulk conflict: Both of you renumbered cues in Scene 2

  Choose:
    * Use my renumbering (from script)
    * Use their renumbering
    * Resolve manually (500 individual conflicts)
  ```

### Script Errors

Scripts can encounter errors (record was deleted, field doesn't exist, network unavailable). You configure how scripts handle errors:

- **Skip:** Ignore the error, continue the script
- **Abort:** Stop the script, discard the overlay
- **Prompt:** Ask you what to do

### Live Preview and Progress

As scripts run, you see what they're doing:

**Progress bars:**
```
Importing contacts... 237/1000 (23%)
[===============>                    ]
```

**Operations accumulating:**
```
Overlay: 237 operations
  - Created 234 records
  - Updated 3 fields
  - 1 conflict detected
```

You're never blind to what a script is doing.

---

## Queries and Computed Views

A **query** is a question: "Given everything we know right now, show me the things that match these conditions."

Queries don't create data or change anything. They observe.

### Example: Call Sheet

"Who needs to be here tomorrow?"

The system looks at:
- Events scheduled for tomorrow
- Scenes in those events
- People in those scenes
- Returns: names, call times, departments

This feels like a call sheet, but it's computed live from underlying data.

### Queries Are Read-Only

You can't directly edit a row in a query result. To change something, you click through to the actual record in its table.

Computed views show results; tables store facts.

### Queries Reflect Context

When run inside an overlay, queries show overlay state. This means you can preview call sheets in a "what-if" scenario, see conflicts or missing people, and decide whether to commit or discard.

---

## Auditability and History

History exists so the team has a shared memory. You can always understand what happened, why, and when.

### The "Who Changed This?" Investigation

You click on a field and see:
- Current value
- Recent changes
- For each change: who made it, when, what changed, why (if context notes exist)

You can expand to see older changes, conflicts, resolutions, and related operations.

### History Shows More Than Field Changes

It includes:
- Structural changes (record creation/deletion, adding records to tables)
- Decision points (conflict creation, resolution)
- Automation (script outputs, rule-triggered actions)

### Mistakes and Corrections Are Both Visible

The original action, the later correction, who did each, when -- all preserved. Mistakes are part of real work. Corrections are part of learning.

History may be summarized, but never falsified.

### Accountability, Not Surveillance

The system only records committed operations -- not every keystroke, not overlay experiments, not drafts. Overlays and drafts are private until committed. History is the team's memory, not a surveillance camera.

---

## What Openprod Is Not

These are deliberate exclusions that protect the system's core values.

### Not Auto-Resolving Conflicts

"Just pick the most recent edit" would silently discard someone's work. Openprod does not guess which change is correct. Humans decide.

### Not Replacing Specialized Tools

Openprod is not a drafting tool, lighting console, audio editor, or email client. It coordinates information between tools, not instead of them. It's the stage manager's binder, not the stage itself.

### Not Requiring Internet or Subscription

Rehearsals happen in basements. Touring means unreliable connectivity. The system must always work without cloud access. Cloud sync exists as an option, but it's never required.

### Not Hiding Computation

Openprod supports expressions and computed values -- but they are always explicit, visible, and auditable. All computed values are user-configured. The system does not compute values unless you have explicitly set up an expression, reference, or rule. There are no hidden formulas or implicit calculations. The distinction is explicit vs. implicit, not "no computation."

### Not Running Automation Without Consent

Rules and triggers can automatically modify data -- but only when you've set them up. Every automated action can be traced to a rule or trigger that you explicitly created or approved. No automation runs without your consent.

### What Would Violate the System's Values

- Automatic conflict resolution or silent merges
- Hiding or rewriting history
- Changing data without explicit user action or user-approved automation
- Requiring a server or treating one user as authoritative
- Logging every keystroke or exposing drafts
- Hidden formulas or implicit computation

**Openprod chooses clarity and trust over automation and convenience.**

---

## Glossary

| Term | Definition |
|------|------------|
| **Table** | A collection of records with a defined set of fields (like a spreadsheet tab) |
| **Record** | A single entry in a table (a person, a cue, a prop -- a row) |
| **Field** | A named piece of data on a record (a column); can be discrete, reference, or query |
| **Smart Field** | Any field using reference or query mode to get its value dynamically |
| **Field Mapping** | A confirmed link between fields in different tables, making them share the same data |
| **Table Membership** | Which tables a record belongs to; a record can be in multiple tables |
| **Relationship** | A first-class link between two records (e.g., "actor is assigned to scene") |
| **Rule** | A table-scoped automation: matching records, managing table membership, etc. |
| **Match Rule** | A rule that identifies when records refer to the same thing and suggests a merge |
| **Expression** | A formula that computes a value from other data (used in Smart Fields, query mode) |
| **Operation** | A field-level change recorded in the history log |
| **Bundle** | Atomic group of operations (all succeed or all fail) |
| **Oplog** | Append-only log of all operations; the source of truth for workspace history |
| **Overlay** | Temporary sandbox for safe experimentation; changes aren't real until committed |
| **Canonical** | The real, authoritative state that syncs between team members |
| **Canonical Drift** | When real data changes while you're experimenting in an overlay |
| **Knockout** | Removing a specific edit from an overlay before committing |
| **Conflict** | When two people edit the same field while disconnected |
| **Module** | A package that adds tables, views, and scripts to your workspace |
| **Script** | Lua automation that produces operations (often in an overlay for review) |
| **Trigger** | A rule that runs a script automatically when data changes |
| **HLC** | Hybrid Logical Clock; provides deterministic ordering across devices |

---

*This document describes the validated behavior of Openprod based on architectural review completed Feb 5, 2026. See ARCHITECTURE.md for technical details and INVARIANTS.md for formal specifications.*
