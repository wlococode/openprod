# Workspace Specification

This document defines workspace lifecycle, structure, join modes, forks, and recovery.

---

## Workspace Model

- **Workspace** = isolated oplog namespace with unique ID
- Entities belong to exactly one workspace
- Entity IDs are workspace-scoped, not globally unique
- Sync only occurs between peers with the same workspace ID
- Different workspaces never leak or mutate each other's data

### Templates & Cloning

- Templates are point-in-time snapshots used to initialize new workspaces
- Templates can pre-confirm field mappings for zero-friction onboarding (e.g., a "Stage Management" starter)
- Clone creates new workspace with current state, no history
- Forking (with history) is not supported in v1
- No automatic cross-workspace sync in v1

### Personal Libraries

- Personal/library data is isolated from workspace data
- Imports from personal library create copies, not references
- Personal library syncs independently from workspace sync

---

## Workspace Structure

A workspace on disk follows this directory layout:

```
workspace/
+-- workspace.toml              # Workspace metadata and configuration
+-- oplog.db                    # SQLite database (oplog + materialized state)
+-- modules/                    # Installed modules
|   +-- contacts/
|   |   +-- module.toml         # Module manifest (table schema, field declarations)
|   |   +-- views/              # TypeScript UI components
|   |   +-- scripts/            # Lua scripts bundled with the module
|   |   +-- exports/            # Lua modules exportable to other scripts
|   +-- lighting/
|   |   +-- module.toml
|   |   +-- views/
|   |   +-- scripts/
|   |   +-- exports/
|   +-- scheduler/
|       +-- module.toml
|       +-- views/
|       +-- scripts/
|       +-- exports/
+-- scripts/                    # User-authored workspace scripts
|   +-- renumber_cues.lua
|   +-- import_contacts.lua
+-- modules.lua                 # Shared Lua code for workspace scripts
+-- blobs/                      # Content-addressed asset storage
+-- identity/                   # Workspace-specific actor roster
```

### Key Directories

| Directory | Purpose |
|-----------|---------|
| `modules/` | Installed modules, each containing schema (TOML), views (TypeScript), scripts (Lua), and exports |
| `scripts/` | User-authored Lua scripts for workspace-specific automation |
| `blobs/` | Content-addressed immutable file storage (PDFs, images, CSVs) |
| `identity/` | Workspace actor roster |

### Module Structure

Each module is a self-contained package:

```
modules/<module_name>/
+-- module.toml                 # Schema: tables, fields, edge types, rules
+-- views/                      # TypeScript UI components for this module's data
+-- scripts/                    # Lua scripts bundled with the module
+-- exports/                    # Lua modules that other scripts can require()
+-- assets/                     # Static assets (icons, templates)
```

The `module.toml` declares:
- **Tables:** What tables this module provides (schema, fields, types)
- **Edge types:** Relationships this module defines
- **Field mapping suggestions:** Suggested mappings to other modules' fields
- **Rules:** Automation rules this module provides
- **Capabilities:** What system capabilities this module's scripts need

---

## Workspace Lifecycle

### Workspace Creation

1. User creates a new workspace (optionally from a template)
2. System generates actor identity for the user (first actor, workspace owner)
3. Bootstrap operation: `{type: "bootstrap", actor: actor_id, owner: true}`
4. Recovery key is generated and shown to the user
5. User selects initial modules to install

### Workspace Identity

- Each workspace has a unique ID
- Workspace ID is generated at creation and never changes
- Workspaces are completely isolated; no cross-workspace data leakage

### Workspace Discovery

- Workspaces advertise on local network via mDNS
- Advertisement includes: workspace name, ID, join mode
- Users see available workspaces and can request to join
- Manual connection via server address is also supported
- Cloud server registration provides discovery beyond LAN

---

## Join Modes

> **Note:** Data-level permissions (Viewer/Editor/Admin roles) are deferred to post-v1. In V1, all actors who join a workspace have full read/write access. The workspace creator retains management privileges (join mode configuration, access key generation, recovery key access) as the workspace owner.

Workspaces have configurable join modes that control how new actors are onboarded.

### Open Mode

- Anyone who discovers the workspace can join automatically
- New actors receive full read/write access
- No approval required
- Use only in trusted environments (same room, private network)

**Flow:**
1. Workspace advertises via mDNS
2. User discovers workspace, selects it
3. User's actor ID is registered
4. Sync begins

### Access Key Mode

- Joining requires a short, speakable code (4-8 alphanumeric characters)
- Access keys are time-limited and use-limited
- Familiar UX (like Kahoot, Zoom, 2FA codes)
- Balances security with ease of onboarding

**Flow:**
1. Workspace owner generates access key with expiration and max uses
2. Owner shares code verbally or via message: "Join with code K7X9"
3. User discovers workspace (mDNS) or enters server address manually
4. User enters access key
5. System validates key, decrements use count
6. User's actor ID is registered, sync begins

### Request Mode

- Users submit a join request
- Workspace owner sees pending requests and approves/rejects
- Most controlled, but slower onboarding
- Good for ongoing workspaces with controlled access

**Flow:**
1. Workspace advertises via mDNS
2. User discovers workspace, submits join request
3. Workspace owner sees pending request in UI
4. Owner approves or rejects
5. On approval: user's actor ID is registered, sync begins
6. On rejection: user notified, no access granted

### Join Mode Configuration

- Join mode is a workspace setting (workspace owner can change)
- Access key generation requires workspace owner
- Join mode changes are auditable operations

---

## Access Keys

Access keys provide a balance between security and frictionless onboarding.

### Access Key Structure

```yaml
AccessKey:
  code: "K7X9"           # Short, speakable, case-insensitive
  expires_at: HLC        # Expiration (HLC-based, not wall-clock)
  max_uses: 20           # Optional limit (null = unlimited)
  uses: 0                # Current use count
  created_by: actor_id   # Audit trail
  created_at: HLC        # Creation timestamp
  # granted_role: ...    # Post-v1: role assigned on successful use
```

### Access Key Validation

On use, the system validates:
- Code matches (case-insensitive)
- Key has not expired (HLC comparison)
- Uses < max_uses (if max_uses is set)
- Workspace join mode is `access_key`

### Access Key Lifecycle

- Created by workspace owner with expiration and max uses
- Each successful use increments the use count
- Key is invalidated when: expired, max uses reached, or manually revoked
- Revoked keys reject all future use attempts
- Key creation and use are auditable operations

### Access Key Best Practices

- Short expiration for production calls (hours, not days)
- Max uses slightly higher than expected attendees
- Generate new keys for each session/call
- Revoke unused keys after the session

---

## Recovery Keys

Recovery keys provide a mechanism for ownership transfer and emergency access.

### Recovery Key Purpose

- Transfer workspace ownership without requiring existing owner presence
- Emergency recovery when the workspace owner is unavailable
- Preserve workspace continuity without creating divergence

### Recovery Key Structure

```yaml
RecoveryKey:
  workspace_id: "..."
  secret: "base64-encoded-high-entropy-token"
  created_at: HLC
  # No expiration - valid until used or regenerated
```

### Recovery Key Visibility

- Always visible to the workspace owner in workspace settings
- Can be copied/exported for secure storage
- Warning displayed: "This key grants workspace ownership. Store securely."

### Using a Recovery Key

1. User has the recovery key secret
2. User submits: `{type: "recovery_bootstrap", recovery_secret: X, actor: actor_id}`
3. System validates secret matches workspace
4. New actor becomes the workspace owner
5. Recovery key is **regenerated** (old secret invalidated)
6. New recovery key shown to the new owner

### Recovery Key Regeneration

- Recovery key regenerates on each use
- Prevents multiple people from using the same key
- New owner receives the new recovery key
- Old key is permanently invalidated

### Recovery Key vs Fork

| Scenario | Use Recovery Key | Use Fork |
| -------- | ---------------- | -------- |
| Transfer ownership | Yes | |
| Owner unavailable | Yes | Yes |
| Create venue variant | | Yes |
| Template for new workspace | | Yes |
| Preserve full history | Yes | |
| Clean slate | | Yes |

---

## Workspace Forks

Forks create a new workspace from the current state of an existing workspace.

### Fork Purpose

- Create workspace variants (touring shows, venue-specific data)
- Template new workspaces from a base
- Recovery when the workspace owner is gone and no recovery key exists
- Experimentation without affecting the original

### Fork Semantics

**Fork creates:**
- New workspace with new unique ID
- Snapshot of current entity state (all entities, facets, edges)
- Forking user as workspace owner
- New recovery key
- Default join mode (configurable)

**Fork does NOT copy:**
- Oplog history (starts fresh)
- Other users/actors (only the forker joins)
- Pending overlays
- Access keys
- Previous ownership (forker becomes the new workspace owner)

### Fork Audit Trail

- Source workspace records: `{type: "forked", by: actor_id, at: HLC, new_workspace: id}`
- New workspace records: `{type: "forked_from", source_workspace: id, at: HLC}`
- Fork history is preserved for provenance

### Fork Permissions

- Any actor in the workspace can fork it
- Forking does not require workspace owner privileges
- This ensures recovery is always possible

### Fork and Divergence

- Forks create intentional divergence
- Source and forked workspaces are independent after fork
- No automatic sync between them
- Changes in one do not affect the other
- Users can manually re-sync by exporting/importing if needed

### Fork Use Cases

**Touring/Venue Variants:**
1. Create base workspace with show data
2. Fork for each venue
3. Each venue workspace has venue-specific modifications
4. Base workspace remains unchanged as template

**Template Workflow:**
1. Create template workspace with common structure
2. Fork for each new production
3. Delete/modify template data as needed
4. Template workspace remains for future use

**Emergency Recovery:**
1. Workspace owner has left, no recovery key available
2. Any actor forks the workspace
3. Forker becomes workspace owner of new workspace
4. Work continues in forked workspace
5. Original workspace is effectively archived

---

## Failure & Recovery

### Crash Safety

- Partial writes never corrupt oplog
- Incomplete bundles are discarded on crash recovery
- Database is consistent after WAL replay

### Corruption Handling

- Corrupt ops are detected via checksum on read/sync
- Corrupt ops are quarantined, not applied
- Quarantined ops are logged for manual review
- System attempts recovery from peers before quarantine

### Recovery Tooling

- System provides oplog inspection tools (view history, search ops)
- System provides conflict history review (see past resolutions)
- System provides quarantine review (see rejected/corrupt ops)
- System provides "export current state" for emergency backup
- Emergency export is always available

### User Experience

- Failures surface human-readable explanations
- Recovery actions are explicit user choices, not automatic

---

## Open Questions

- Template format: full oplog snapshot or just entity state?
- Workspace archiving (read-only mode)?
- Cross-workspace references in future versions?
- Access key UX for non-networked join
- Module marketplace/registry for discovering and installing modules
