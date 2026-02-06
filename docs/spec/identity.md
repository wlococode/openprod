# Identity Specification

This document defines actor identity for v1: how users are identified via Ed25519 keypairs, how identity is used in the oplog, how operations are signed, and how actor IDs relate to conflict attribution and the HLC.

---

## Overview

In v1, identity is based on Ed25519 cryptographic keypairs. The actor ID IS the Ed25519 public key (32 bytes, hex-encoded for display). Every operation is signed with the actor's private key, and peers verify signatures using the public key (actor_id) before applying operations.

**Anchor invariant:** Every operation has exactly one actor. Actor identity is immutable once created. Every operation is cryptographically signed by its author.

---

## Actor ID

### Format

- Actor ID is an Ed25519 public key (32 bytes, hex-encoded or base64-encoded for display/storage)
- Actor ID is globally unique across all workspaces and devices (derived from cryptographic keypair)
- Actor ID is a stable, permanent identifier for a single user

### Generation

- On first launch, generate an Ed25519 keypair
- The public key (32 bytes) becomes the actor_id
- The private key (32 bytes) is stored securely in local persistent storage
- No server round-trip required

### Storage

```yaml
LocalIdentity:
  actor_id: <Ed25519 public key, 32 bytes hex>
  private_key: <Ed25519 private key, 32 bytes, stored securely>
  display_name: "Jane Smith"
  created_at: <ISO 8601 timestamp>
```

The local identity file is stored outside any workspace. It is user-level, not workspace-level. The private key must be protected with appropriate OS-level file permissions (e.g., 0600 on Unix systems).

### Immutability

- Once generated, an actor ID (public key) never changes
- The display name can be updated at any time
- Display name changes do not affect historical operation attribution

---

## Operation Signing

Every operation is signed with the actor's Ed25519 private key before it is written to the oplog or transmitted to peers.

### Signing Process

1. Construct the signing payload: concatenate `id + actor_id + hlc + payload` (canonical byte representation)
2. Sign the payload with the actor's Ed25519 private key
3. Attach the 64-byte signature to the operation

### Verification Process

1. Extract the `actor_id` (public key) from the operation
2. Reconstruct the signing payload from `id + actor_id + hlc + payload`
3. Verify the Ed25519 signature against the public key
4. Reject the operation if verification fails

### Signing Scope

- All operations in the canonical oplog are signed
- Bundle metadata is also signed
- Local-only data (overlays, module local data) does not require signing

---

## Usage in Operations

Every operation in the oplog includes the actor ID and a cryptographic signature:

```yaml
Operation:
  id: <UUIDv7>
  actor_id: <Ed25519 public key>
  hlc: <HLC timestamp>
  payload: { ... }
  signature: <Ed25519 signature of (id + actor_id + hlc + payload)>
```

The actor ID is written at operation creation time and is immutable. The signature proves that the holder of the corresponding private key authored the operation. This enables audit, conflict attribution, display purposes, and tamper detection.

---

## Usage in HLC

The actor ID is used in vector clocks to track sync state per actor:

```yaml
VectorClock:
  actor_a: <last seen HLC from actor_a>
  actor_b: <last seen HLC from actor_b>
```

Each entry in a vector clock is keyed by actor ID (public key). This allows the sync protocol to request "send me all operations from actor X after HLC Y."

See [hlc.md](hlc.md) for full HLC specification.

---

## Usage in Conflict Attribution

When a conflict is detected between concurrent edits, the actor IDs of the involved operations are surfaced to the user:

- "Jane Smith and Alex Chen both edited this field while offline"
- The conflict resolution UI shows who made each conflicting change
- Actor ID links the operation to a display name for human-readable attribution

See [conflicts.md](conflicts.md) for full conflict detection and resolution specification.

---

## Display Name

### Storage

Each actor's display name is stored locally and broadcast to peers during sync:

```yaml
ActorMetadata:
  actor_id: <Ed25519 public key, 32 bytes hex>
  display_name: "Jane Smith"
```

### Propagation

- On first sync with a workspace, a peer announces its actor ID (public key) and display name
- Peers cache display names for all known actors
- Display name updates are propagated on subsequent syncs
- If a display name is unknown (e.g., viewing historical operations from an actor not yet seen), the UI falls back to a truncated actor ID (hex prefix)

### Constraints

- Display names are not unique (two users can have the same display name)
- The actor ID (public key) is the authoritative identifier; display name is cosmetic
- Display names have no effect on conflict detection, resolution, or operation ordering

---

## Multi-Device Identity

### Same User, Multiple Devices

For v1, a user who wants the same actor ID across multiple devices must transfer their keypair:

- Export identity from device A (keypair + display name)
- Import identity on device B
- Both devices now produce operations with the same actor ID and can sign with the same private key

### Transfer Mechanism

Identity transfer requires securely transferring the private key:

```yaml
IdentityExport:
  actor_id: <Ed25519 public key, 32 bytes hex>
  private_key: <Ed25519 private key, 32 bytes, encrypted or securely transferred>
  display_name: "Jane Smith"
  exported_at: <ISO 8601 timestamp>
```

This can be transferred via encrypted file, QR code, or secure cloud sync (implementation-defined). The private key MUST be protected during transfer (e.g., encrypted with a user-chosen passphrase).

### Alternative: One Actor Per Device

If a user does not transfer identity, each device gets its own keypair and actor ID. This is acceptable for v1:

- Operations from different devices are attributed to different actor IDs (different public keys)
- The user appears as two distinct actors in the oplog
- No merging of actor identities after the fact (this is a post-v1 concern)

---

## Actor Registration in Workspace

When an actor first contributes to a workspace, the workspace records the actor:

```yaml
WorkspaceActor:
  actor_id: <Ed25519 public key, 32 bytes hex>
  display_name: "Jane Smith"
  first_seen_hlc: <HLC of first operation>
```

This allows the workspace to maintain a roster of known actors for display in the UI (conflict attribution, operation history, presence indicators).

---

## Trust Model

For v1, trust is based on cryptographic verification:

- Every operation is signed with the actor's Ed25519 private key
- Peers verify operation signatures using the actor's public key (which IS the actor_id) before applying operations
- Operations with invalid signatures are rejected
- Any actor who can connect to a workspace (via join mode) is trusted to write operations
- This model ensures that operations cannot be forged or tampered with, even on untrusted networks

Post-v1 enhancements (key rotation, trust hierarchies, certificate chains) will further strengthen this model.

---

## Roles (Deferred to Post-v1)

For v1, there is no role-based access control. Any actor who joins a workspace can read and write all data (all operations, all entities, all fields). Everyone is effectively an editor.

Roles and permissions are planned for post-v1 and will include:

- **Viewer** -- read-only access
- **Editor** -- read/write access
- **Admin** -- full access including workspace configuration
- Fine-grained permissions at the table, facet, and field level
- Custom user-defined roles

The actor_id and signing infrastructure introduced in v1 provides the foundation for role enforcement in future versions.

---

## Implementation Notes

### Identity File Location

- macOS: `~/Library/Application Support/openprod/identity.json`
- Linux: `~/.local/share/openprod/identity.json`
- Windows: `%APPDATA%/openprod/identity.json`

### Identity File Contents

```json
{
  "actor_id": "<64 hex chars, Ed25519 public key>",
  "private_key": "<64 hex chars, Ed25519 private key>",
  "display_name": "Jane Smith",
  "created_at": "2025-01-15T10:30:00Z"
}
```

The identity file MUST be stored with restrictive permissions (readable only by the owner).

### First Launch Flow

1. Check for existing identity file
2. If none exists, generate new Ed25519 keypair
3. The public key becomes the actor_id
4. Prompt user for display name (or use OS username as default)
5. Write identity file (with restrictive file permissions)
6. Identity is now available for all workspace operations

### Testing

Implementations must pass:
- **Uniqueness:** Generated actor IDs (public keys) do not collide (Ed25519 guarantees)
- **Persistence:** Actor ID and keypair survive application restart
- **Attribution:** All operations carry the correct actor ID
- **Signing:** All operations are signed with the correct private key
- **Verification:** Peers reject operations with invalid signatures
- **Display fallback:** Unknown actor IDs render gracefully in the UI (truncated hex)

---

## Deferred to Post-v1

The following identity features are deferred. The v1 Ed25519 identity model is designed to be forward-compatible with these additions:

- **Key rotation:** Replacing a compromised or lost keypair while preserving actor history
- **Device-specific keys:** Each device gets its own keypair under a single user identity
- **Trust and verification models:** Peer trust hierarchies, certificate chains, trust-on-first-use (TOFU)
- **Fine-grained permissions:** Permission scopes at Table, Facet, and Field levels
- **Custom roles:** User-defined roles with specific permission combinations

---

## Related Documents

- [hlc.md](hlc.md) -- HLC format and vector clocks (keyed by actor ID)
- [operations.md](operations.md) -- Operation structure including actor_id and signature fields
- [conflicts.md](conflicts.md) -- Conflict detection and actor attribution
- [sync.md](sync.md) -- Sync protocol and actor metadata propagation
