# Wire Format Specification

This document defines the binary encoding for operations, bundles, and sync messages.

---

## Binary Encoding: MessagePack

**Anchor invariant:** All wire-format data is encoded using MessagePack. The format is self-describing, compact, and has ubiquitous library support across languages.

### Why MessagePack over CBOR

| Criterion | MessagePack | CBOR |
|-----------|-------------|------|
| **Library maturity** | Mature, battle-tested in Redis, Fluentd | Good, but fewer production deployments |
| **Performance** | Excellent encode/decode speed | Comparable, slightly larger output |
| **Ecosystem** | Broad (Rust, Swift, TypeScript, Python) | Narrower |
| **Debugging** | Many inspection tools available | Fewer tools |
| **Extension types** | Native support | Requires tags |
| **Size** | Slightly smaller for typical payloads | Slightly larger |

MessagePack wins on ecosystem breadth and tooling. CBOR's IETF standardization is valuable but less critical for a local-first system where we control both ends.

### Extension Types

MessagePack extension types are used for domain-specific values:

| Ext Type | Value | Description |
|----------|-------|-------------|
| `0x01` | HLC | Hybrid Logical Clock (10 bytes, see [hlc.md](hlc.md)) |
| `0x02` | UUID | Entity/Operation ID (16 bytes) |
| `0x03` | Signature | Ed25519 signature (64 bytes) |
| `0x04` | PublicKey | Ed25519 public key (32 bytes) |
| `0x05` | Hash | BLAKE3 hash (32 bytes) |

---

## Compression: zstd

**Anchor invariant:** Bundles are compressed with zstd before transmission. Compression is transparent to the application layer.

### Why zstd

| Criterion | zstd | gzip | lz4 |
|-----------|------|------|-----|
| **Compression ratio** | Excellent | Good | Moderate |
| **Decompression speed** | Very fast | Moderate | Fastest |
| **Dictionary support** | Yes | No | Limited |
| **Streaming support** | Yes | Yes | Yes |
| **Adoption** | Growing (Linux kernel, HTTP) | Universal | Niche |

zstd provides the best balance of compression ratio and speed. Dictionary support enables better compression of small, similar messages (operations share common field names).

### Compression Parameters

```yaml
Compression:
  algorithm: zstd
  level: 3                    # Balance of speed and ratio (1-22 scale)
  dictionary: optional        # Pre-trained on operation schema
  min_size: 256               # Don't compress payloads under 256 bytes
```

- Level 3 provides good compression with minimal CPU overhead
- Dictionary training on typical operation payloads improves small-message compression by ~30%
- Messages under 256 bytes are sent uncompressed (overhead exceeds benefit)

### Compression Indicator

The first byte of any wire message indicates compression:

| Byte | Meaning |
|------|---------|
| `0x00` | Uncompressed MessagePack follows |
| `0x28` | zstd magic number (compressed data follows) |

zstd's magic number (`0x28 0xB5 0x2F 0xFD`) is self-identifying, so no additional framing is needed.

---

## Operation Wire Format

Individual operations are encoded as MessagePack maps:

```yaml
Operation:
  v: 1                        # Wire format version
  id: <UUID ext>              # Unique operation ID
  actor: <PublicKey ext>      # Actor's public key (32 bytes)
  hlc: <HLC ext>              # Hybrid Logical Clock
  plugins:                    # Plugin versions at creation time
    contacts: "1.1.0"
    scheduler: "2.0.0"
  payload:                    # Operation-specific data
    type: "set_field"
    entity: <UUID ext>
    field: "name"
    value: "Jane Doe"
  sig: <Signature ext>        # Ed25519 signature (64 bytes)
```

### Field Order

Fields are encoded in the order shown above. This enables:
- Deterministic serialization for signature verification
- Streaming parsing (signature last, after all signed content)

### Signature Computation

```
signed_content = msgpack([v, id, actor, hlc, plugins, payload])
signature = ed25519_sign(private_key, blake3(signed_content))
```

The signature covers all fields except itself. BLAKE3 hashing before signing ensures consistent performance regardless of payload size.

### Wire Size Estimates

| Operation Type | Typical Size (uncompressed) | Compressed |
|----------------|----------------------------|------------|
| Field edit | 180-250 bytes | 120-180 bytes |
| Entity create | 200-300 bytes | 140-220 bytes |
| Facet attach | 150-200 bytes | 100-150 bytes |
| Edge create | 220-320 bytes | 160-240 bytes |

---

## Bundle Wire Format

Bundles group operations for atomicity and are the unit of transmission.

**Anchor invariant:** A bundle is the atomic unit of commit. All operations in a bundle succeed or fail together.

```yaml
Bundle:
  v: 1                        # Wire format version
  id: <UUID ext>              # Unique bundle ID
  type: <BundleType>          # Semantic bundle type (see below)
  actor: <PublicKey ext>      # Bundle author
  hlc: <HLC ext>              # Bundle timestamp (max HLC of contained ops)

  # Entity lifecycle markers
  creates: [<UUID ext>, ...]  # Entities created in this bundle
  deletes: [<UUID ext>, ...]  # Entities deleted in this bundle

  # Operations
  ops: [<Operation>, ...]     # Ordered list of operations

  # Metadata
  meta:
    source_overlay: <UUID>    # If committed from overlay
    display_name: "Import from CSV"

  # Bundle signature (covers all above)
  sig: <Signature ext>
```

### Bundle Types

```yaml
BundleType:
  user_edit: 0x01       # Direct user interaction
  script_output: 0x02   # Produced by a script execution
  import: 0x03          # External data import
  merge_resolution: 0x04 # Conflict/merge resolution
  rule_triggered: 0x05  # Automatic rule execution
  migration: 0x06       # Schema migration
  system: 0x07          # System maintenance operations
```

| Type | Semantic Meaning | Typical Source |
|------|------------------|----------------|
| `user_edit` | Interactive changes by a human | UI edits, form submissions |
| `script_output` | Results of script execution | Import scripts, automation, batch processing |
| `import` | Data from external sources | CSV import, API sync, file ingestion |
| `merge_resolution` | Resolving conflicts or entity merges | Conflict resolution UI, identity repair |
| `rule_triggered` | Automatic changes from rules | Facet attachment rules, derived fields |
| `migration` | Schema evolution changes | Plugin upgrades, field migrations |
| `system` | Internal system operations | GC markers, checkpoint markers |

Bundle type is informational and auditable. It does not affect processing semantics.

### Bundle Signature

The bundle signature covers the entire bundle including all operations:

```
signed_content = msgpack([v, id, type, actor, hlc, creates, deletes, ops, meta])
signature = ed25519_sign(private_key, blake3(signed_content))
```

Operations within a bundle also retain their individual signatures for:
- Independent verification
- Extraction and re-bundling if needed
- Audit trail integrity

---

## Sync Message Wire Format

Sync messages are exchanged between peers during replication.

### Message Envelope

All sync messages share a common envelope:

```yaml
SyncMessage:
  v: 1                        # Protocol version
  type: <MessageType>         # Message type discriminator
  sender: <PublicKey ext>     # Sender's actor ID
  seq: <uint64>               # Message sequence number (per sender)
  payload: <type-specific>    # Message payload
```

### Message Types

```yaml
MessageType:
  # Vector clock exchange
  vector_clock_request: 0x10
  vector_clock_response: 0x11

  # Operation exchange
  ops_request: 0x20
  ops_response: 0x21
  ops_push: 0x22

  # Bundle exchange
  bundle_push: 0x30
  bundle_ack: 0x31
  bundle_nack: 0x32

  # Leader protocol
  election_start: 0x40
  election_vote: 0x41
  leader_announce: 0x42
  leader_redirect: 0x43

  # State verification
  state_hash_request: 0x50
  state_hash_response: 0x51

  # Presence
  heartbeat: 0x60
  presence_update: 0x61
```

### Vector Clock Exchange

```yaml
VectorClockRequest:
  # Empty - requests peer's vector clock

VectorClockResponse:
  clock:
    <actor_id>: <HLC ext>     # Last seen HLC per actor
    <actor_id>: <HLC ext>
    ...
```

### Operations Exchange

```yaml
OpsRequest:
  since:                      # Vector clock of requester
    <actor_id>: <HLC ext>
    ...
  limit: 1000                 # Max operations to return

OpsResponse:
  ops: [<Operation>, ...]     # Operations not covered by `since`
  complete: true              # Whether all missing ops are included

OpsPush:
  bundle: <Bundle>            # New bundle to apply
```

### Bundle Acknowledgment

```yaml
BundleAck:
  bundle_id: <UUID ext>

BundleNack:
  bundle_id: <UUID ext>
  reason: <NackReason>
  details: "Invalid signature on op 3"

NackReason:
  invalid_signature: 0x01
  schema_violation: 0x02
  unknown_actor: 0x03
  duplicate_bundle: 0x04
  future_hlc: 0x05            # HLC too far in future
```

### State Hash Verification

```yaml
StateHashRequest:
  # Empty - requests current state hash

StateHashResponse:
  hash: <Hash ext>            # BLAKE3 hash of derived state
  op_count: <uint64>          # Number of operations in canonical history
  latest_hlc: <HLC ext>       # Most recent HLC
```

---

## Stream Framing

**Anchor invariant:** Stream messages are length-prefixed. The length prefix enables efficient parsing and memory allocation.

### Frame Format

```
┌──────────────────┬─────────────────────────────────────┐
│  Length (4 bytes)│  Payload (variable)                 │
│  big-endian u32  │  [compression indicator + data]     │
└──────────────────┴─────────────────────────────────────┘
```

- Length is a 4-byte big-endian unsigned integer
- Length includes the compression indicator byte
- Maximum frame size: 16 MiB (enforced, see below)

### Why Length-Prefixed

| Approach | Pros | Cons |
|----------|------|------|
| Length-prefixed | Fast parsing, pre-allocation, easy skip | Requires knowing size upfront |
| Delimiter-based | Streaming-friendly | Escaping overhead, slower parsing |
| Self-describing | No framing needed | Must parse to find boundaries |

Length-prefixed is optimal for our use case:
- Bundles are fully constructed before sending
- Receivers can allocate exact buffer size
- Enables efficient batch parsing

### Streaming Large Responses

For large sync responses (e.g., catch-up with thousands of operations):

```
┌─────────────────────────────────────────────────────────┐
│  Frame 1: OpsResponse (complete: false, ops: [...])     │
├─────────────────────────────────────────────────────────┤
│  Frame 2: OpsResponse (complete: false, ops: [...])     │
├─────────────────────────────────────────────────────────┤
│  Frame 3: OpsResponse (complete: true, ops: [...])      │
└─────────────────────────────────────────────────────────┘
```

- Large responses are chunked into multiple frames
- Each frame is independently valid
- `complete: false` indicates more frames follow
- Receiver can process incrementally

---

## Bundle Size Limits

### Advisory Limits

| Limit | Value | Rationale |
|-------|-------|-----------|
| **Soft limit** | 1 MiB | Target for normal bundles |
| **Hard limit** | 16 MiB | Maximum enforceable limit |
| **Operation count** | 10,000 | Max operations per bundle |

### Soft Limit Handling

Bundles exceeding 1 MiB trigger a warning but are accepted:

```yaml
BundleSizeWarning:
  bundle_id: <UUID>
  size: 2_500_000
  message: "Bundle exceeds soft limit (1 MiB). Consider splitting."
```

Large bundles may:
- Delay sync for other operations
- Cause memory pressure on constrained devices
- Increase conflict probability

### Hard Limit Enforcement

Bundles exceeding 16 MiB are rejected:

```yaml
BundleNack:
  bundle_id: <UUID>
  reason: size_exceeded
  details: "Bundle size 18.5 MiB exceeds 16 MiB limit"
```

### Chunking Strategy

For large imports or bulk operations:

1. Split into multiple bundles at natural boundaries (e.g., per-entity)
2. Link bundles via metadata if needed:

```yaml
Bundle:
  meta:
    batch_id: <UUID>          # Links related bundles
    batch_index: 3
    batch_total: 7
```

Linked bundles are still independently valid and atomic.

---

## Schema Versioning in Wire Format

### Version Fields

Every message includes version information:

```yaml
# Wire format version (this spec)
v: 1

# Plugin versions (in operations)
plugins:
  contacts: "1.1.0"
  scheduler: "2.0.0"
```

### Wire Format Version

The `v` field indicates wire format version:

| Version | Description |
|---------|-------------|
| `1` | Initial version (this spec) |

Future versions may add fields. Receivers must:
- Accept messages with `v` equal to or less than their supported version
- Reject messages with `v` greater than their supported version
- Ignore unknown fields in messages (forward compatibility)

### Plugin Version Handling

Operations include `plugins` map for schema interpretation:

```yaml
Operation:
  plugins:
    contacts: "1.1.0"         # Schema version used when creating this op
```

During replay:
1. Look up operation's plugin version
2. Apply any necessary schema migrations
3. Interpret payload according to that schema version

See [schema-evolution.md](schema-evolution.md) for migration details.

---

## HLC Encoding

Hybrid Logical Clocks are encoded as a 10-byte extension type. See [hlc.md](hlc.md) for the full specification.

```
┌─────────────────────────────────────────────────────────┐
│  Wall Time (8 bytes)       │  Counter (2 bytes)         │
│  big-endian uint64         │  big-endian uint16         │
│  milliseconds since epoch  │  logical counter           │
└─────────────────────────────────────────────────────────┘
```

- **Wall time**: 64-bit milliseconds since Unix epoch (big-endian)
- **Counter**: 16-bit logical counter for ordering within same millisecond (big-endian)

No node ID component is included; operations use `op_id` (UUIDv7) for tiebreaking when HLCs are equal.

### HLC Comparison

HLCs are compared lexicographically as byte strings:
1. Compare wall time (most significant)
2. Compare counter (least significant)

If HLCs are equal, the operation's `op_id` serves as the tiebreaker. This produces a deterministic total order consistent with causality.

---

## Security Considerations

### Signature Verification Order

When receiving a bundle:

1. Verify frame length is within limits
2. Decompress payload
3. Parse MessagePack structure
4. **Verify bundle signature** (reject if invalid)
5. **Verify each operation signature** (reject if any invalid)
6. Validate schema and business rules
7. Apply to local state

Signature verification happens before any state modification.

### Replay Protection

- Each operation has a unique `(actor, hlc)` pair
- Duplicate operations (same ID) are detected and deduplicated
- HLCs from the future (>5 minutes) are rejected as suspicious

### Compression Bombs

- Decompression is bounded by hard limit (16 MiB uncompressed)
- Decompression timeout: 5 seconds
- Abort and reject if limits exceeded

---

## Example: Complete Sync Session

```
Alice                              Bob
  │                                 │
  │ ──── VectorClockRequest ──────> │
  │                                 │
  │ <─── VectorClockResponse ────── │
  │      { alice: HLC_100,          │
  │        bob: HLC_95 }            │
  │                                 │
  │ ──── OpsRequest ──────────────> │
  │      { since: { alice: HLC_90,  │
  │                 bob: HLC_80 } } │
  │                                 │
  │ <─── OpsResponse ───────────────│
  │      { ops: [...],              │
  │        complete: true }         │
  │                                 │
  │ ──── StateHashRequest ────────> │
  │                                 │
  │ <─── StateHashResponse ──────── │
  │      { hash: 0xabc123... }      │
  │                                 │
  │   [Alice computes same hash]    │
  │   [Sync complete, converged]    │
```

---

## Implementation Notes

### Recommended Libraries

| Language | MessagePack | zstd | Ed25519 |
|----------|-------------|------|---------|
| Rust | `rmp-serde` | `zstd` | `ed25519-dalek` |
| Swift | `MessagePack.swift` | `ZSTDSwift` | `CryptoKit` |
| TypeScript | `@msgpack/msgpack` | `zstd-codec` | `tweetnacl` |

### Testing

Wire format implementations must pass:
- Round-trip encoding tests (encode then decode = original)
- Cross-language compatibility tests
- Signature verification tests with known test vectors
- Compression/decompression tests with edge cases

---

## Open Questions

- Dictionary pre-training dataset and distribution mechanism
- Exact timeout values for network operations
- Handling of extremely clock-skewed peers (>1 hour drift)
- Binary diff/delta encoding for operation payloads?
- Encryption layer for peer-to-peer transport?
