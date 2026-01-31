Plugin/E2E Scenarios:

# 1 Cross-Plugin Scene Interaction and Rehearsal Scheduling

## 1.1 Abstract

Stage Managers are entirely responsible for personnel management and coordinating time constraints, breaks, and availability with venue, rehearsal needs, script segments, etc. There is no standardized system for calculating schedules, and it's often done through referencing dozens of files: Create full-team email contact group, determine rehearsal block (Scene 3-5), reference Who/What/Where doc or Scene Breakdown doc for info on "Who is in scenes 3-5", when does each person need to be called, etc., then must verify each actor has no conflicts, build the schedule, find times for breaks, calculate the add and release times for each person individually, double check, then send out to the full team. Each actor and crew member are responsible for reading the full call properly, and it can be confusing to understand exactly when they show up.

## 1.2 Scenario

- Team installs and workspace adopts "Script" plugin, basically PDF viewer with annotation elements.
- Stage Manager or ASM can flag pages and ranges with bookmarks, Act/Scene info, and "interactions" like actor fights or intimacy, as well as page ranges for characters, props, etc. onstage
- Properties techs can use "Props" or "Inventory" plugin, allowing dynamic references like "Radio — used in Scenes 3, 5, and 7" or "Napkin Ring — used in Scenes 2—4"
- Stage Manager can build a "deck run" plot for transitions in a "Deck Run" plugin. Creates persistent scenic elements like table, chairs, etc. or referenced elements from props inventory, and arranges them for transition objects. A transition = Scene N -> Scene N+1 or custom insertions if desired. Can assign stage crew from "Contacts" plugin to items or "tracks". System can automatically calculate what crew is available, who should be assigned where, etc.
- Stage Manager builds a rehearsal's schedule for actors and crew using "Scheduler" plugin: Inserts rehearsal block event(s), can select "Attendees -> Derived -> Fight Interaction AND Scene Number is between 1 and 10", which would get all actors that have a fight interaction between scenes 1 and 10.
- Similarly for crew scheduling, if a rehearsal only covers scenes 1-3, a query could set Attendees to "In Stage Crew group AND is in Scenes 1-3". Easy to make arbitrary queries with intuitive UX.
- Based on structured ruleset templates, mandatory breaks can be inserted (Equity 5/55, 10/80, AGMA standard, etc.), so a SM never has to say "Where can I fit a break into this 2 hour block?", breaks could be global, per-department, or per-person.
- After a schedule is built and we know who is needed for each segment, first calls and labor calculations are easy: "Person A is first used at 7:00, can be breaked between 8:00-8:30 (exceeds 20min minimum down time), and is released at 10:00."

## 1.3 Results

- Known exactly what actors and crew are needed for which scenes, transitions, etc.
- Cross-plugin interaction allows "smart value" derivation using simple queries that users can understand
- Mitigates user error in calls (forgot to put this person on this transition, missed this person in this event, didn't update this first call, etc.)
- Custom per-person schedules and calls can be emailed out automatically, no more actors hunting for info or misreading complex schedules

# 2 Costume Fitting Scheduling

## 2.1 Abstract

There is often a need in production to schedule one-off or recurring events based on availability, which varies per-person. Costume fittings are a good example of this. We have a cast of 50 actors, their costumes will be ready for try-on fittings on Jan 30th, and we need everyone to try on their costume within 2 weeks. This is usually done manually by referencing contact conflict/availability forms against the costume shop's availability.

## 2.2 Scenario

- Contacts plugin exposes contact info like name, role, email, phone, etc.
- Availability plugin extends an availability matrix: fields like name, email, etc. are linked via a "Concept" (or whatever shared identity structure we use) to contacts. Each linked person now has availability[] with entries like {entry: "conflict", type: "recurring", frequency: "weekly", time: "Monday, 5-10PM CST", description: "Soccer practice"}
- The same idea is extended for the costume shop (can be applied to contacts, spaces/rooms, groups, etc.)
- The same scheduler plugin referenced in scenario 1 could also have the functionality to achieve this per-person scheduling automatically. Each plugin functions on its own, but with the derived queries and smart fields, things get exciting. The scheduler could expose/accept name, email, travel_time, etc. as well as a set of availability[] items. When we dedupe and condense identity between the plugins, they all work together
- Scheduler plugin uses availability matrix to find slots for each person.

## 2.3 Result

- 50+ individual appointment events are scheduled effortlessly
- Easy surfacing of errors like "no availability within the given constraints"
- Easy to swap, defer, or modify
- Simple to export paperwork, send personalized emails to each person, and full schedule to costume shop
- Name or email changes in one plugin? Propogates through system since everything is referenced and linked.

# 3 Lighting Automatic Patching

## 3.1 Abstract

Lighting designers and programmers often manage hundreds or thousands of fixtures, between inventory, drafting, paperwork, and the console. Updates are manually propogated through paperwork and much of the work is repetitive and tedious. Getting fixture info into the console is either done by hand or with file export -> flash drive -> import.

## 3.2 Scenario

- Lighting Designer installs "Fixture Type Library Database" plugin and "Lighting/Fixtures" plugin or suite with tools for patching, etc.
- Lights and accessories could be stored in shared inventory with props under a blanket "Inventory" plugin or separately.
- Lighting designer imports ETC Fixture Library database HTML/CSV with info about console naming conventions for 100,000 fixture types.
- Lighting designer imports CSV from Lightwright or Vectorworks with fixture info: string_type, channel, address, universe, hanging_position, focus_point, etc.
- Lighting plugin/system and user can map each fixture type to one in the fixture library database, to say "S4 Lustr2 36°" means "ETC_Fixtures S4_Series_2_Lustr_Direct" in the console.
- Once types are assigned, a job can perform patching simply: "Patch All Fixtures -> Group by hanging_position -> Sort by unit_number ascending -> Ignore if channel is empty -> Modes like Universe Per Position", assigns DMX universe and address to all fixtures.
- Once fixtures are patched, another job with OSC capability can send data to the console: "For Each Fixture with Address AND Not Ignored -> Send OSC Command: Patch Channel X Address Y Type Z Label W". Could also support 3D pos/rot data.
- While patching or after, get console fixture IDs and map to entities by channel number or some other identifier.
- Easy to import updated fixtures or change fields, and push changes to console safely. Everything is deterministic, versioned, etc.

## 3.3 Result

- Full tracking and versioning of fixtures and information
- Automate repetitive DMX addressing and console patching safely
- Query console via OSC to get info like "What fixtures are not used in cue 103?"
- Same principles apply for cue lists, presets, other console objects

# 4 Notes, Tasks, etc.

## 4.1 Abstract

There's a constant need for note-taking, task status tracking, to-do lists, etc. and so many variants: work notes, line notes, etc. that need attention from different departments and department scopes: specific person, group of people, scenic crew, design team, all production, actors, etc.

## 4.2 Scenario + Results

- Notes plugin could be very light and simply reference any entity or entities via edges, and have description, assignees, etc.
- Tasks plugin could be more complex. Can also attach to any entity or entities, but also supports: dependency/prerequesite tasks, departments, tagging, etc. Could expose events or a duration_estimate field that the scheduler could pick up. Determine how much time is needed pre-show for each department to complete its tasks.

# 5 Prompt Cue Book Consolidation

## 5.1 Abstract

Any theatrical show that has called cues relies on a stage manager's prompt book. This is a binder with a script that has every called cue written into it, plus standbys and notes. A prompt book/script contains cues for every department: Lighting, Sound, Video, Scenic/Deck, Automation, etc. as well as annotations. To get cues into the book, the SM and director have to meet with each designer/domain, get all cues written into the book, ensure they don't conflict with other action happening, and understand what each cue does. If a designer makes a change or update, they repeat the process. Hours and days are spent updating and managing cues, ensuring info is current, and fixing mistakes/typos.

## 5.2 Scenario & Result

- Script View plugin allows managing and annotating a PDF script
- Consider an optional Document Management plugin as well, for splicing/arranging PDF files
- Peers can insert their department's cues onto the same script and see live updates when connected
- Toggleable allow for separation of concerns, "Lighting" layer and "Sound" layers for different disciplines
- Dynamic "standby" objects can be insterted as well, with formats like "Show all cues on this page and next 3 pages for all layers"
- Easily export PDF with customized layers, page layout/margins, styling, etc.

In all of these scenarios, collaborative replication and conflict resolution are assumed working as expected.
