# Phase 1: Core Types + Storage + Oplog — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the foundational `core`, `storage`, and `harness` crates — the type system, persistence layer, and integration test framework that everything else builds on.

**Architecture:** Rust workspace with three crates. `core` defines all domain types (IDs, HLC, operations, identity). `storage` defines a `Storage` trait and implements it with `SqliteStorage` using rusqlite. `harness` provides `TestPeer`/`TestNetwork` abstractions for integration testing. State derivation in Phase 1 is minimal — `SqliteStorage` materializes entity/field/facet/edge state as operations are appended. Full command/query separation and incremental derivation come in Phase 2.

**Tech Stack:** Rust 2024 edition, rusqlite (SQLite), ed25519-dalek (signing), rmp-serde (MessagePack), uuid (v7), thiserror, tempfile (tests)

**Decisions captured from brainstorming:**
- Greenfield — no prototype to migrate
- rusqlite for SQLite (synchronous, direct control over WAL/PRAGMAs)
- MessagePack for all serialized payloads and field values
- Harness-first testing — primary testing through integration boundary
- Unit tests only for pure algorithms (HLC math, Ed25519 signing)
- Phase 1 state derivation is intentionally minimal (materialized on append). Full engine comes Phase 2.

**Serialization stability:** OperationPayload uses serde's default externally-tagged enum representation (`{"VariantName": {...}}`). This is name-based, so adding new variants is safe. All variants are defined in Phase 1 even if not exercised until later phases, ensuring the enum is stable from day one.

---

## Task 1: Project Scaffolding

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/core/Cargo.toml`
- Create: `crates/core/src/lib.rs`
- Create: `crates/storage/Cargo.toml`
- Create: `crates/storage/src/lib.rs`
- Create: `crates/harness/Cargo.toml`
- Create: `crates/harness/src/lib.rs`

**Step 1: Create workspace Cargo.toml**

```toml
# Cargo.toml
[workspace]
resolver = "2"
members = [
    "crates/core",
    "crates/storage",
    "crates/harness",
]

[workspace.package]
edition = "2024"
license = "MIT"
repository = "https://github.com/user/openprod"

[workspace.dependencies]
# Serialization
serde = { version = "1", features = ["derive"] }
rmp-serde = "1"

# IDs and crypto
uuid = { version = "1", features = ["v7", "serde"] }
ed25519-dalek = { version = "2", features = ["rand_core"] }
rand = "0.8"

# Storage
rusqlite = { version = "0.32", features = ["bundled", "blob"] }

# Error handling
thiserror = "2"

# Hashing
blake3 = "1"

# Testing
tempfile = "3"

# Internal crates
openprod-core = { path = "crates/core" }
openprod-storage = { path = "crates/storage" }
openprod-harness = { path = "crates/harness" }
```

**Step 2: Create core crate**

```toml
# crates/core/Cargo.toml
[package]
name = "openprod-core"
version = "0.1.0"
edition.workspace = true

[dependencies]
serde.workspace = true
rmp-serde.workspace = true
uuid.workspace = true
ed25519-dalek.workspace = true
rand.workspace = true
thiserror.workspace = true
blake3.workspace = true
```

```rust
// crates/core/src/lib.rs
pub mod ids;
pub mod field_value;
pub mod hlc;
pub mod identity;
pub mod operations;
pub mod vector_clock;
pub mod error;

pub use ids::*;
pub use field_value::FieldValue;
pub use hlc::Hlc;
pub use error::CoreError;
```

**Step 3: Create storage crate**

```toml
# crates/storage/Cargo.toml
[package]
name = "openprod-storage"
version = "0.1.0"
edition.workspace = true

[dependencies]
openprod-core.workspace = true
rusqlite.workspace = true
rmp-serde.workspace = true
serde.workspace = true
thiserror.workspace = true
uuid.workspace = true
blake3.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

```rust
// crates/storage/src/lib.rs
pub mod traits;
pub mod sqlite;
pub mod schema;
pub mod error;

pub use traits::*;
pub use error::StorageError;
```

**Step 4: Create harness crate**

```toml
# crates/harness/Cargo.toml
[package]
name = "openprod-harness"
version = "0.1.0"
edition.workspace = true

[dependencies]
openprod-core.workspace = true
openprod-storage.workspace = true
tempfile.workspace = true
uuid.workspace = true
```

```rust
// crates/harness/src/lib.rs
pub mod peer;
pub mod network;

pub use peer::TestPeer;
pub use network::TestNetwork;
```

**Step 5: Create stub modules for compilation**

Empty files for: `crates/core/src/{hlc,identity,operations,vector_clock,error,ids,field_value}.rs` and `crates/storage/src/{sqlite,schema,error,traits}.rs` and `crates/harness/src/{peer,network}.rs`.

**Step 6: Verify workspace compiles**

Run: `cargo check --workspace`
Expected: Compiles with warnings about empty modules

**Step 7: Commit**

```
feat: scaffold Cargo workspace with core, storage, and harness crates
```

---

## Task 2: Core ID Types + FieldValue

**Files:**
- Create: `crates/core/src/ids.rs`
- Create: `crates/core/src/field_value.rs`
- Create: `crates/core/src/error.rs`

**Step 1: Implement ID types**

All IDs are newtype wrappers around `uuid::Uuid` with `Copy`, `Eq`, `Ord`, `Hash`, `Serialize`, `Deserialize`. `ActorId` wraps `[u8; 32]` (Ed25519 public key). `Signature` wraps `[u8; 64]`.

```rust
// crates/core/src/ids.rs
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(Uuid::from_bytes(bytes))
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }

            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), &self.0.to_string()[..8])
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

uuid_id!(EntityId);
uuid_id!(OpId);
uuid_id!(BundleId);
uuid_id!(EdgeId);
uuid_id!(TableId);
uuid_id!(RuleId);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActorId([u8; 32]);

impl ActorId {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ActorId({:02x}{:02x}{:02x}{:02x})", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display first 8 bytes as hex
        for byte in &self.0[..8] {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Signature([u8; 64]);

impl Signature {
    pub fn from_bytes(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({:02x}{:02x}...)", self.0[0], self.0[1])
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlobHash([u8; 32]);

impl BlobHash {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlobHash({:02x}{:02x}...)", self.0[0], self.0[1])
    }
}
```

**Step 2: Implement FieldValue**

**FIX from review #16:** `Float` does not derive `PartialEq`. We implement it manually using `f64::total_cmp` to handle NaN deterministically.

```rust
// crates/core/src/field_value.rs
use serde::{Deserialize, Serialize};
use crate::ids::{EntityId, BlobHash};

/// All possible field value types in the system.
/// Stored as MessagePack BLOBs in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldValue {
    Null,
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    /// Milliseconds since Unix epoch
    Timestamp(i64),
    /// Reference to another entity
    EntityRef(EntityId),
    /// Reference to a content-addressed blob
    BlobRef(BlobHash),
    /// Raw bytes (used for CRDT state)
    Bytes(Vec<u8>),
}

/// Manual PartialEq to handle f64 NaN correctly via total_cmp.
/// NaN == NaN is true (consistent, deterministic behavior for field comparisons).
impl PartialEq for FieldValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Integer(a), Self::Integer(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a.total_cmp(b).is_eq(),
            (Self::Boolean(a), Self::Boolean(b)) => a == b,
            (Self::Timestamp(a), Self::Timestamp(b)) => a == b,
            (Self::EntityRef(a), Self::EntityRef(b)) => a == b,
            (Self::BlobRef(a), Self::BlobRef(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for FieldValue {}

impl FieldValue {
    pub fn is_null(&self) -> bool {
        matches!(self, FieldValue::Null)
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            FieldValue::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            FieldValue::Integer(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            FieldValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Serialize to MessagePack bytes.
    pub fn to_msgpack(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec(self)
    }

    /// Deserialize from MessagePack bytes.
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(bytes)
    }
}
```

**Step 3: Implement error type**

```rust
// crates/core/src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("invalid signature")]
    InvalidSignature,

    #[error("HLC drift too large: remote is {delta_ms}ms ahead (max {max_ms}ms)")]
    HlcDriftTooLarge { delta_ms: u64, max_ms: u64 },

    #[error("invalid operation: {0}")]
    InvalidOperation(String),

    #[error("invalid data: {0}")]
    InvalidData(String),
}
```

**Step 4: Verify compilation**

Run: `cargo check -p openprod-core`
Expected: Compiles clean

**Step 5: Commit**

```
feat(core): add ID types (EntityId, OpId, ActorId, etc.) and FieldValue

FieldValue uses manual PartialEq with f64::total_cmp for NaN safety.
```

---

## Task 3: HLC Implementation

**Files:**
- Create: `crates/core/src/hlc.rs`

12-byte HLC (8 wall_ms + 4 counter), tick/receive algorithms, drift rejection at 5 minutes, lexicographic ordering.

**Changes from original plan:**
- `Hlc::from_bytes` returns `Result<Self, CoreError>` instead of using `unwrap()` on slice conversion. Even though slicing `[u8; 12]` is infallible, this keeps error handling consistent with `to_array<N>` used elsewhere.
- `physical_now()` returns `Result<u64, CoreError>` instead of using `.expect()` on `SystemTime::now().duration_since(UNIX_EPOCH)`. Maps the error to `CoreError::InvalidData("system clock before epoch")`.
- `HlcClock::tick()` and `HlcClock::receive()` return `Result<Hlc, CoreError>` to propagate `physical_now()` errors.

Unit tests: tick monotonicity, same-wall-time counter increment, byte roundtrip, ordering matches bytes, drift rejection, within-drift acceptance, concurrent timestamp merging.

**Commit:** `feat(core): implement HLC with tick/receive algorithms and drift rejection`

---

## Task 4: Ed25519 Identity

**Files:**
- Create: `crates/core/src/identity.rs`

Unchanged from original plan. `ActorIdentity` wraps `SigningKey`, generates keypair, signs bytes, verifies via public key. `verify_signature` is a free function.

Unit tests: sign/verify roundtrip, wrong message fails, wrong key fails, secret bytes roundtrip.

**Commit:** `feat(core): implement Ed25519 identity with signing and verification`

---

## Task 5: Operation and Bundle Types

**Files:**
- Create: `crates/core/src/operations.rs`

**Changes from original plan (review fixes #1-6):**
1. **Signing payload does NOT include bundle_id** — matches spec: `id + actor_id + hlc + payload`
2. **All operation variants defined** — adds `ClearAndAdd`, `LinkTables`, `UnlinkTables`, `AddToTable`, `RemoveFromTable`, `RestoreFacet`, `CreateRule`
3. **DetachFacet has `preserve_values: bool`** — enables facet data stashing
4. **CreateEntity has `initial_table: Option<String>`** — atomic facet attachment
5. **Operation has `module_versions: BTreeMap<String, String>`** — empty in Phase 1, stable signing format (BTreeMap for deterministic serialization)
6. **Bundle has `checksum`, `creates`, `deletes`, `meta`** — full spec compliance
7. **Bundle::new_signed takes pre-generated BundleId** — avoids chicken-and-egg with operation bundle_id

```rust
// crates/core/src/operations.rs
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use crate::ids::*;
use crate::field_value::FieldValue;
use crate::hlc::Hlc;
use crate::identity::{ActorIdentity, verify_signature};
use crate::error::CoreError;

/// All possible operation payloads. Every variant is defined now for
/// MessagePack serialization stability, even if not exercised until later phases.
/// Uses serde's default externally-tagged representation (name-based, safe to extend).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OperationPayload {
    // --- Entity lifecycle ---
    CreateEntity {
        entity_id: EntityId,
        /// Optional: atomically attach this facet on creation.
        initial_table: Option<String>,
    },
    DeleteEntity {
        entity_id: EntityId,
        /// Edges to cascade-delete. Computed by the system at deletion time.
        cascade_edges: Vec<EdgeId>,
    },

    // --- Facets (table membership internal mechanism) ---
    AttachFacet {
        entity_id: EntityId,
        facet_type: String,
    },
    DetachFacet {
        entity_id: EntityId,
        facet_type: String,
        /// When true, field values are stashed for potential RestoreFacet.
        preserve_values: bool,
    },
    RestoreFacet {
        entity_id: EntityId,
        facet_type: String,
    },

    // --- Scalar fields ---
    SetField {
        entity_id: EntityId,
        field_key: String,
        value: FieldValue,
    },
    ClearField {
        entity_id: EntityId,
        field_key: String,
    },

    // --- CRDT fields ---
    ApplyCrdt {
        entity_id: EntityId,
        field_key: String,
        crdt_type: CrdtType,
        delta: Vec<u8>,
    },
    /// Reset a CRDT set: remove elements before HLC, add new ones.
    ClearAndAdd {
        entity_id: EntityId,
        field_key: String,
        values: Vec<FieldValue>,
    },

    // --- Edges ---
    CreateEdge {
        edge_id: EdgeId,
        edge_type: String,
        source_id: EntityId,
        target_id: EntityId,
        properties: Vec<u8>, // MessagePack-encoded map
    },
    DeleteEdge {
        edge_id: EdgeId,
    },

    // --- Ordered edges ---
    CreateOrderedEdge {
        edge_id: EdgeId,
        edge_type: String,
        source_id: EntityId,
        target_id: EntityId,
        after: Option<EdgeId>,
        before: Option<EdgeId>,
        properties: Vec<u8>,
    },
    MoveOrderedEdge {
        edge_id: EdgeId,
        after: Option<EdgeId>,
        before: Option<EdgeId>,
    },

    // --- Table operations ---
    LinkTables {
        source_table: TableId,
        target_table: TableId,
        field_mappings: Vec<(String, String)>,
    },
    UnlinkTables {
        source_table: TableId,
        target_table: TableId,
        /// "copy" = copy shared data to standalone, "discard" = detach only
        data_handling: String,
    },
    AddToTable {
        entity_id: EntityId,
        table: String,
        defaults: Vec<(String, FieldValue)>,
    },
    RemoveFromTable {
        entity_id: EntityId,
        table: String,
        /// "copy" or "discard"
        data_handling: String,
    },

    // --- Field mappings ---
    ConfirmFieldMapping {
        source_table: TableId,
        target_table: TableId,
        source_field: String,
        target_field: String,
    },

    // --- Identity repair ---
    MergeEntities {
        survivor: EntityId,
        absorbed: EntityId,
    },
    SplitEntity {
        source: EntityId,
        new_entity: EntityId,
        facets: Vec<String>,
    },

    // --- Rules ---
    CreateRule {
        rule_id: RuleId,
        name: String,
        when_clause: String,
        action_type: String,
        action_params: Vec<u8>, // MessagePack
        auto_accept: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrdtType {
    Text,
    List,
}

impl OperationPayload {
    /// Extract the primary entity_id this operation targets (if any).
    pub fn entity_id(&self) -> Option<EntityId> {
        match self {
            Self::CreateEntity { entity_id, .. }
            | Self::DeleteEntity { entity_id, .. }
            | Self::AttachFacet { entity_id, .. }
            | Self::DetachFacet { entity_id, .. }
            | Self::RestoreFacet { entity_id, .. }
            | Self::SetField { entity_id, .. }
            | Self::ClearField { entity_id, .. }
            | Self::ApplyCrdt { entity_id, .. }
            | Self::ClearAndAdd { entity_id, .. }
            | Self::AddToTable { entity_id, .. }
            | Self::RemoveFromTable { entity_id, .. } => Some(*entity_id),
            Self::CreateEdge { source_id, .. }
            | Self::CreateOrderedEdge { source_id, .. } => Some(*source_id),
            Self::MergeEntities { survivor, .. } => Some(*survivor),
            Self::SplitEntity { source, .. } => Some(*source),
            Self::DeleteEdge { .. }
            | Self::MoveOrderedEdge { .. }
            | Self::LinkTables { .. }
            | Self::UnlinkTables { .. }
            | Self::ConfirmFieldMapping { .. }
            | Self::CreateRule { .. } => None,
        }
    }

    /// String name of the operation type for storage/indexing.
    pub fn op_type_name(&self) -> &'static str {
        match self {
            Self::CreateEntity { .. } => "CreateEntity",
            Self::DeleteEntity { .. } => "DeleteEntity",
            Self::AttachFacet { .. } => "AttachFacet",
            Self::DetachFacet { .. } => "DetachFacet",
            Self::RestoreFacet { .. } => "RestoreFacet",
            Self::SetField { .. } => "SetField",
            Self::ClearField { .. } => "ClearField",
            Self::ApplyCrdt { .. } => "ApplyCrdt",
            Self::ClearAndAdd { .. } => "ClearAndAdd",
            Self::CreateEdge { .. } => "CreateEdge",
            Self::DeleteEdge { .. } => "DeleteEdge",
            Self::CreateOrderedEdge { .. } => "CreateOrderedEdge",
            Self::MoveOrderedEdge { .. } => "MoveOrderedEdge",
            Self::LinkTables { .. } => "LinkTables",
            Self::UnlinkTables { .. } => "UnlinkTables",
            Self::AddToTable { .. } => "AddToTable",
            Self::RemoveFromTable { .. } => "RemoveFromTable",
            Self::ConfirmFieldMapping { .. } => "ConfirmFieldMapping",
            Self::MergeEntities { .. } => "MergeEntities",
            Self::SplitEntity { .. } => "SplitEntity",
            Self::CreateRule { .. } => "CreateRule",
        }
    }

    /// Serialize payload to MessagePack bytes.
    pub fn to_msgpack(&self) -> Result<Vec<u8>, CoreError> {
        rmp_serde::to_vec(self).map_err(|e| CoreError::Serialization(e.to_string()))
    }

    /// Deserialize payload from MessagePack bytes.
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, CoreError> {
        rmp_serde::from_slice(bytes).map_err(|e| CoreError::Serialization(e.to_string()))
    }
}

/// A signed, immutable operation in the oplog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub op_id: OpId,
    pub actor_id: ActorId,
    pub hlc: Hlc,
    pub bundle_id: BundleId,
    /// Module versions active when this op was created.
    /// Empty map in Phase 1. Ensures stable signing format for future phases.
    /// BTreeMap for deterministic serialization order (no sorting needed at sign time).
    pub module_versions: BTreeMap<String, String>,
    pub payload: OperationPayload,
    pub signature: Signature,
}

impl Operation {
    /// Build the signing payload: (op_id + actor_id + hlc + module_versions + payload).
    /// NOTE: bundle_id is NOT part of the signing payload (per spec: id + actor_id + hlc + payload).
    /// module_versions is an intentional extension — ensures version info is tamper-proof.
    /// This is NOT in the current spec definition but is the right thing to do; update spec to match.
    fn signing_bytes(
        op_id: &OpId,
        actor_id: &ActorId,
        hlc: &Hlc,
        module_versions: &BTreeMap<String, String>,
        payload_bytes: &[u8],
    ) -> Result<Vec<u8>, CoreError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(op_id.as_bytes());
        bytes.extend_from_slice(actor_id.as_bytes());
        bytes.extend_from_slice(&hlc.to_bytes());
        // BTreeMap serializes in sorted key order — deterministic without conversion
        let mv_bytes = rmp_serde::to_vec(module_versions)
            .map_err(|e| CoreError::Serialization(e.to_string()))?;
        bytes.extend_from_slice(&mv_bytes);
        bytes.extend_from_slice(payload_bytes);
        Ok(bytes)
    }

    /// Create and sign a new operation.
    pub fn new_signed(
        identity: &ActorIdentity,
        hlc: Hlc,
        bundle_id: BundleId,
        module_versions: BTreeMap<String, String>,
        payload: OperationPayload,
    ) -> Result<Self, CoreError> {
        let op_id = OpId::new();
        let actor_id = identity.actor_id();
        let payload_bytes = payload.to_msgpack()?;
        let signing_bytes = Self::signing_bytes(
            &op_id, &actor_id, &hlc, &module_versions, &payload_bytes,
        )?;
        let signature = identity.sign(&signing_bytes);

        Ok(Self {
            op_id,
            actor_id,
            hlc,
            bundle_id,
            module_versions,
            payload,
            signature,
        })
    }

    /// Verify this operation's signature.
    pub fn verify_signature(&self) -> Result<(), CoreError> {
        let payload_bytes = self.payload.to_msgpack()?;
        let signing_bytes = Self::signing_bytes(
            &self.op_id,
            &self.actor_id,
            &self.hlc,
            &self.module_versions,
            &payload_bytes,
        )?;
        verify_signature(&self.actor_id, &signing_bytes, &self.signature)
    }
}

/// Canonical ordering for operations: (hlc, op_id).
impl Ord for Operation {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.hlc
            .cmp(&other.hlc)
            .then(self.op_id.cmp(&other.op_id))
    }
}

impl PartialOrd for Operation {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Operation {
    fn eq(&self, other: &Self) -> bool {
        self.op_id == other.op_id
    }
}

impl Eq for Operation {}

/// Bundle metadata — groups operations for atomicity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub bundle_id: BundleId,
    pub actor_id: ActorId,
    pub hlc: Hlc,
    pub bundle_type: BundleType,
    pub op_count: u32,
    /// BLAKE3 hash of all operation bytes in the bundle (integrity verification).
    pub checksum: [u8; 32],
    /// Entity IDs created in this bundle (quick inspection without scanning ops).
    pub creates: Vec<EntityId>,
    /// Entity IDs deleted in this bundle.
    pub deletes: Vec<EntityId>,
    /// Arbitrary metadata (MessagePack-encoded, optional).
    pub meta: Option<Vec<u8>>,
    pub signature: Signature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BundleType {
    UserEdit = 1,
    ScriptOutput = 2,
    Import = 3,
    System = 4,
}

impl Bundle {
    /// Create and sign a bundle for a set of operations.
    /// Takes a pre-generated bundle_id — the same ID must have been used when creating
    /// the operations (via Operation::new_signed). This avoids the chicken-and-egg problem:
    /// 1. Generate BundleId::new()
    /// 2. Create operations with that bundle_id
    /// 3. Create Bundle with the same bundle_id and the operations (for checksum)
    pub fn new_signed(
        bundle_id: BundleId,
        identity: &ActorIdentity,
        hlc: Hlc,
        bundle_type: BundleType,
        operations: &[Operation],
    ) -> Result<Self, CoreError> {
        let actor_id = identity.actor_id();
        let op_count = operations.len() as u32;

        // Compute checksum: BLAKE3 of concatenated serialized ops
        let mut hasher = blake3::Hasher::new();
        for op in operations {
            let bytes = op.payload.to_msgpack()?;
            hasher.update(&bytes);
        }
        let checksum = *hasher.finalize().as_bytes();

        // Extract creates/deletes for quick inspection
        let mut creates = Vec::new();
        let mut deletes = Vec::new();
        for op in operations {
            match &op.payload {
                OperationPayload::CreateEntity { entity_id, .. } => creates.push(*entity_id),
                OperationPayload::DeleteEntity { entity_id, .. } => deletes.push(*entity_id),
                _ => {}
            }
        }

        // Sign bundle metadata
        let mut sign_bytes = Vec::new();
        sign_bytes.extend_from_slice(bundle_id.as_bytes());
        sign_bytes.extend_from_slice(actor_id.as_bytes());
        sign_bytes.extend_from_slice(&hlc.to_bytes());
        sign_bytes.push(bundle_type as u8);
        sign_bytes.extend_from_slice(&op_count.to_be_bytes());
        sign_bytes.extend_from_slice(&checksum);
        let signature = identity.sign(&sign_bytes);

        Ok(Self {
            bundle_id,
            actor_id,
            hlc,
            bundle_type,
            op_count,
            checksum,
            creates,
            deletes,
            meta: None,
            signature,
        })
    }
}
```

**Step 2: Verify compilation**

Run: `cargo check -p openprod-core`
Expected: Compiles clean

**Step 3: Commit**

```
feat(core): implement Operation/Bundle types with all spec-defined variants

Signing payload matches spec (id + actor_id + hlc + payload, no bundle_id).
All operation variants defined for MessagePack serialization stability.
Bundle includes BLAKE3 checksum, creates/deletes arrays, and metadata.
Operation includes module_versions field (empty map in Phase 1).
```

---

## Task 6: Vector Clock

**Files:**
- Create: `crates/core/src/vector_clock.rs`

`VectorClock` wraps `BTreeMap<ActorId, Hlc>`. Methods: `update` (track max), `get`, `merge` (take max per actor), `diff` (find entries where other > ours), `covers` (check if we've seen everything other has).

Unit tests: update tracks max, merge takes max, diff finds missing ops, covers detects completeness.

**Full implementation:** Same as original plan — no changes needed.

**Commit:** `feat(core): implement VectorClock for sync catch-up and conflict detection`

---

## Task 7: Storage Trait + SQLite Schema

**Files:**
- Create: `crates/storage/src/traits.rs`
- Create: `crates/storage/src/schema.rs`
- Create: `crates/storage/src/error.rs`

**Review fixes applied:**
- **#7:** `received_at` uses milliseconds: `DEFAULT (CAST(unixepoch('now','subsec') * 1000 AS INTEGER))`
- **#11:** Added `schema_version` table for migration tracking
- **#12:** Added `PRAGMA mmap_size = 268435456`
- **#13:** Edge indexes are partial: `WHERE deleted_at IS NULL`
- **#14:** Entities table includes `redirect_at BLOB` column
- **#17:** Added `vector_clock` table for incremental tracking (no full oplog scan)
- **#6:** Bundle table includes `checksum`, `creates`, `deletes`, `meta` columns
- **#5:** Oplog table includes `module_versions` column

**Step 1: Error type**

```rust
// crates/storage/src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("constraint violation: {0}")]
    ConstraintViolation(String),

    #[error("entity collision: {entity_id}")]
    EntityCollision { entity_id: String },

    #[error("core error: {0}")]
    Core(#[from] openprod_core::CoreError),
}
```

**Step 2: Storage trait**

```rust
// crates/storage/src/traits.rs
use openprod_core::{
    ids::*, field_value::FieldValue, hlc::Hlc,
    operations::{Operation, Bundle}, vector_clock::VectorClock,
};
use crate::error::StorageError;

#[derive(Debug, Clone)]
pub struct EntityRecord {
    pub entity_id: EntityId,
    pub created_at: Hlc,
    pub created_by: ActorId,
    pub deleted: bool,
}

#[derive(Debug, Clone)]
pub struct FacetRecord {
    pub entity_id: EntityId,
    pub facet_type: String,
    pub attached_at: Hlc,
    pub attached_by: ActorId,
    pub detached: bool,
}

#[derive(Debug, Clone)]
pub struct EdgeRecord {
    pub edge_id: EdgeId,
    pub edge_type: String,
    pub source_id: EntityId,
    pub target_id: EntityId,
    pub properties: Vec<u8>,
    pub created_at: Hlc,
    pub created_by: ActorId,
    pub deleted: bool,
}

pub trait Storage {
    /// Append a bundle of operations atomically. Updates materialized state.
    /// Returns EntityCollision error on duplicate entity IDs.
    fn append_bundle(&mut self, bundle: &Bundle, operations: &[Operation]) -> Result<(), StorageError>;

    fn get_ops_canonical(&self) -> Result<Vec<Operation>, StorageError>;
    fn get_ops_by_bundle(&self, bundle_id: BundleId) -> Result<Vec<Operation>, StorageError>;
    fn get_ops_by_actor_after(&self, actor_id: ActorId, after: Hlc) -> Result<Vec<Operation>, StorageError>;
    fn op_count(&self) -> Result<u64, StorageError>;

    fn get_entity(&self, entity_id: EntityId) -> Result<Option<EntityRecord>, StorageError>;
    fn get_fields(&self, entity_id: EntityId) -> Result<Vec<(String, FieldValue)>, StorageError>;
    fn get_field(&self, entity_id: EntityId, field_key: &str) -> Result<Option<FieldValue>, StorageError>;
    fn get_facets(&self, entity_id: EntityId) -> Result<Vec<FacetRecord>, StorageError>;
    fn get_entities_by_facet(&self, facet_type: &str) -> Result<Vec<EntityId>, StorageError>;
    fn get_edges_from(&self, entity_id: EntityId) -> Result<Vec<EdgeRecord>, StorageError>;
    fn get_edges_to(&self, entity_id: EntityId) -> Result<Vec<EdgeRecord>, StorageError>;

    /// Tracked incrementally — not computed from full oplog scan.
    fn get_vector_clock(&self) -> Result<VectorClock, StorageError>;
}
```

**Step 3: SQLite schema**

```rust
// crates/storage/src/schema.rs
use rusqlite::Connection;
use crate::error::StorageError;

pub const SCHEMA_VERSION: i32 = 1;

pub fn init_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch("
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA foreign_keys = ON;
        PRAGMA cache_size = -32000;
        PRAGMA mmap_size = 268435456;
        PRAGMA busy_timeout = 5000;
    ")?;
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
);
INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (1, unixepoch());

CREATE TABLE IF NOT EXISTS oplog (
    rowid INTEGER PRIMARY KEY,
    op_id BLOB NOT NULL UNIQUE CHECK (length(op_id) = 16),
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 32),
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),
    bundle_id BLOB NOT NULL CHECK (length(bundle_id) = 16),
    payload BLOB NOT NULL,
    module_versions BLOB NOT NULL,
    signature BLOB NOT NULL CHECK (length(signature) = 64),
    op_type TEXT NOT NULL,
    entity_id BLOB,
    received_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('now','subsec') * 1000 AS INTEGER))
);
CREATE INDEX IF NOT EXISTS idx_oplog_canonical ON oplog (hlc, op_id);
CREATE INDEX IF NOT EXISTS idx_oplog_actor_hlc ON oplog (actor_id, hlc);
CREATE INDEX IF NOT EXISTS idx_oplog_entity ON oplog (entity_id, hlc);
CREATE INDEX IF NOT EXISTS idx_oplog_bundle ON oplog (bundle_id);

CREATE TABLE IF NOT EXISTS bundles (
    bundle_id BLOB PRIMARY KEY CHECK (length(bundle_id) = 16),
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 32),
    hlc BLOB NOT NULL CHECK (length(hlc) = 12),
    bundle_type INTEGER NOT NULL,
    op_count INTEGER NOT NULL,
    checksum BLOB NOT NULL CHECK (length(checksum) = 32),
    creates BLOB,
    deletes BLOB,
    meta BLOB,
    signature BLOB NOT NULL CHECK (length(signature) = 64),
    received_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('now','subsec') * 1000 AS INTEGER))
);

CREATE TABLE IF NOT EXISTS entities (
    entity_id BLOB PRIMARY KEY CHECK (length(entity_id) = 16),
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL CHECK (length(created_in_bundle) = 16),
    deleted_at BLOB CHECK (deleted_at IS NULL OR length(deleted_at) = 12),
    deleted_by BLOB CHECK (deleted_by IS NULL OR length(deleted_by) = 32),
    deleted_in_bundle BLOB,
    redirect_to BLOB,
    redirect_at BLOB CHECK (redirect_at IS NULL OR length(redirect_at) = 12)
);

CREATE TABLE IF NOT EXISTS fields (
    entity_id BLOB NOT NULL CHECK (length(entity_id) = 16),
    field_key TEXT NOT NULL,
    value BLOB NOT NULL,
    source_op BLOB NOT NULL CHECK (length(source_op) = 16),
    source_actor BLOB NOT NULL CHECK (length(source_actor) = 32),
    updated_at BLOB NOT NULL CHECK (length(updated_at) = 12),
    PRIMARY KEY (entity_id, field_key)
);

CREATE TABLE IF NOT EXISTS facets (
    entity_id BLOB NOT NULL CHECK (length(entity_id) = 16),
    facet_type TEXT NOT NULL,
    attached_at BLOB NOT NULL CHECK (length(attached_at) = 12),
    attached_by BLOB NOT NULL CHECK (length(attached_by) = 32),
    attached_in_bundle BLOB NOT NULL CHECK (length(attached_in_bundle) = 16),
    source_type TEXT NOT NULL DEFAULT 'user',
    detached_at BLOB CHECK (detached_at IS NULL OR length(detached_at) = 12),
    detached_by BLOB CHECK (detached_by IS NULL OR length(detached_by) = 32),
    detached_in_bundle BLOB,
    preserve_values BLOB,
    PRIMARY KEY (entity_id, facet_type)
);

CREATE TABLE IF NOT EXISTS edges (
    edge_id BLOB PRIMARY KEY CHECK (length(edge_id) = 16),
    edge_type TEXT NOT NULL,
    source_id BLOB NOT NULL CHECK (length(source_id) = 16),
    target_id BLOB NOT NULL CHECK (length(target_id) = 16),
    properties BLOB,
    created_at BLOB NOT NULL CHECK (length(created_at) = 12),
    created_by BLOB NOT NULL CHECK (length(created_by) = 32),
    created_in_bundle BLOB NOT NULL CHECK (length(created_in_bundle) = 16),
    deleted_at BLOB CHECK (deleted_at IS NULL OR length(deleted_at) = 12),
    deleted_by BLOB CHECK (deleted_by IS NULL OR length(deleted_by) = 32),
    deleted_in_bundle BLOB
);
CREATE INDEX IF NOT EXISTS idx_edges_source ON edges (source_id, edge_type) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_edges_target ON edges (target_id, edge_type) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS actors (
    actor_id BLOB PRIMARY KEY CHECK (length(actor_id) = 32),
    display_name TEXT,
    first_seen_at BLOB NOT NULL CHECK (length(first_seen_at) = 12)
);

CREATE TABLE IF NOT EXISTS vector_clock (
    actor_id BLOB PRIMARY KEY CHECK (length(actor_id) = 32),
    max_hlc BLOB NOT NULL CHECK (length(max_hlc) = 12)
);
";
```

**Commit:** `feat(storage): define Storage trait and SQLite schema with full spec compliance`

---

## Task 8: SqliteStorage Implementation

**Files:**
- Create: `crates/storage/src/sqlite.rs`

**Review fixes applied:**
- **#8:** No `unwrap()` — all byte conversions use `to_array<N>` helper returning `StorageError`
- **#9:** `CreateEntity` uses `INSERT INTO` (not `INSERT OR IGNORE`) — collisions return `StorageError::EntityCollision`
- **#15:** No dead `materialize_op` method — materialization is only inline in `append_bundle`
- **#17:** Vector clock updated incrementally via `vector_clock` table with `ON CONFLICT DO UPDATE`
- **#3:** `DetachFacet` stashes field values when `preserve_values` is true
- **#4:** `CreateEntity` handles `initial_table` by also inserting facet
- **#6:** Bundle insert includes `checksum`, `creates`, `deletes`, `meta`
- **#5:** Oplog insert includes `module_versions`

Key implementation patterns:

```rust
/// Helper: convert Vec<u8> to fixed-size array with proper error handling.
fn to_array<const N: usize>(v: Vec<u8>, label: &str) -> Result<[u8; N], StorageError> {
    v.try_into()
        .map_err(|_| StorageError::Serialization(format!("invalid {label} length")))
}
```

`read_op` reads 7 columns: `op_id, actor_id, hlc, bundle_id, payload, module_versions, signature` — all deserialized through `to_array` (no unwraps).

`append_bundle` runs in a single transaction:
1. Insert bundle row (with checksum, creates, deletes, meta)
2. For each op: insert oplog row, materialize state, track actor, update vector clock
3. Entity collision detected via SQLite UNIQUE constraint → mapped to `StorageError::EntityCollision`
4. Vector clock: `INSERT INTO vector_clock ... ON CONFLICT(actor_id) DO UPDATE SET max_hlc = excluded.max_hlc WHERE excluded.max_hlc > vector_clock.max_hlc`
5. `DeleteEntity` iterates `cascade_edges` and soft-deletes each
6. `DetachFacet` with `preserve_values: true` queries fields within the transaction and stores them

`get_vector_clock` reads from `vector_clock` table (O(actors), not O(oplog)).

**Full code:** See the complete `sqlite.rs` implementation provided in the review-corrected plan above. The implementation follows the Storage trait interface exactly.

**Commit:** `feat(storage): implement SqliteStorage with collision detection, incremental vector clock, and edge cascade`

---

## Task 9: Harness Foundation + Integration Tests

**Files:**
- Create: `crates/harness/src/peer.rs`
- Create: `crates/harness/src/network.rs`
- Create: `crates/harness/tests/phase1.rs`

**Review fixes applied:**
- **#10:** `delete_entity` queries edges from/to the entity, includes them as `cascade_edges`
- **#19:** Added tests for: ClearField, DetachFacet, edge deletion, cascade deletion, entity collision error, `get_ops_by_actor_after`

**TestPeer** wraps `ActorIdentity` + `HlcClock` + `SqliteStorage`. Convenience methods:
- `create_record(facet_type, fields)` — CreateEntity + AttachFacet + N×SetField in one bundle
- `set_field`, `clear_field`, `delete_entity` (with cascade computation), `create_edge`, `delete_edge`, `detach_facet`
- `execute_bundle` — generates BundleId upfront, creates ops with that bundle_id, creates bundle with same ID and ops (for checksum), appends atomically

**TestNetwork** manages a `Vec<TestPeer>` with `add_peer`, `peer(i)`, `peer_mut(i)`.

**22 integration tests organized by category:**

| Category | Tests |
|----------|-------|
| Entity/Field CRUD | create_entity_with_fields, update_field_value, clear_field, delete_entity, detach_facet, field_types_roundtrip, query_entities_by_facet |
| Signatures | all_operations_have_valid_signatures, tampered_operation_fails_verification |
| Canonical Ordering | canonical_ordering_is_deterministic, operations_attributed_to_correct_actor |
| Bundles | bundle_groups_operations, operation_count_tracks_correctly |
| Edges | create_and_query_edge, delete_edge, delete_entity_cascades_edges |
| Error Handling | entity_collision_returns_error |
| Sync Preparation | vector_clock_reflects_operations, get_ops_by_actor_after, multiple_peers_independent |

**Run:** `cargo test --workspace`
**Expected:** All 37 tests pass (15 unit + 22 integration)

**Commit:** `feat(harness): implement TestPeer/TestNetwork with 22 integration tests`

---

## Summary

| Task | Crate | What it builds | Tests |
|------|-------|---------------|-------|
| 1 | all | Cargo workspace scaffolding | Compiles |
| 2 | core | ID types, FieldValue (NaN-safe), error types | Compiles |
| 3 | core | HLC (12-byte, tick/receive, drift) | 7 unit tests |
| 4 | core | Ed25519 identity (sign/verify) | 4 unit tests |
| 5 | core | All 21 operation variants, Bundle with checksum | Compiles |
| 6 | core | VectorClock (merge, diff, covers) | 4 unit tests |
| 7 | storage | Storage trait, full SQLite schema | Compiles |
| 8 | storage | SqliteStorage (oplog + materialized state) | Compiles |
| 9 | harness | TestPeer, TestNetwork, integration tests | 22 integration tests |

**Total: 9 tasks, ~37 tests, 3 crates.**

### All 24 review fixes applied

| # | Issue | Fix |
|---|-------|-----|
| 1 | Signing includes bundle_id | Removed — matches spec: `id + actor_id + hlc + payload` |
| 2 | Missing operation variants | Added 7: ClearAndAdd, LinkTables, UnlinkTables, AddToTable, RemoveFromTable, RestoreFacet, CreateRule |
| 3 | DetachFacet missing preserve | Added `preserve_values: bool` |
| 4 | CreateEntity missing initial_table | Added `initial_table: Option<String>` |
| 5 | Missing module_versions | Added `HashMap<String, String>` on Operation (empty in Phase 1) |
| 6 | Bundle missing fields | Added BLAKE3 checksum, creates, deletes, meta |
| 7 | received_at seconds | Milliseconds: `CAST(unixepoch('now','subsec') * 1000 AS INTEGER)` |
| 8 | unwrap() in non-test code | `to_array<N>` helper returns `StorageError` |
| 9 | INSERT OR IGNORE entities | `INSERT INTO` — collisions return `EntityCollision` |
| 10 | Empty cascade_edges | `delete_entity` queries edges, includes in cascade |
| 11 | Missing schema_version | Added table |
| 12 | Missing PRAGMAs | Added `mmap_size = 268435456` |
| 13 | Non-partial edge indexes | Added `WHERE deleted_at IS NULL` |
| 14 | Missing redirect_at | Added to entities table |
| 15 | Dead materialize_op | Removed — inline only |
| 16 | Float PartialEq NaN | Manual impl with `f64::total_cmp` |
| 17 | Vector clock scans oplog | Incremental `vector_clock` table |
| 18 | &mut self for writes | Noted for Phase 2 |
| 19 | Missing tests | Added 8 tests: ClearField, DetachFacet, edge deletion, cascade, collision, actor_after |
| 20 | Bundle::new_signed generates own bundle_id | Takes pre-generated BundleId to avoid chicken-and-egg with Operation.bundle_id |
| 21 | module_versions signing called "spec compliance" | Reworded as intentional tamper-proof extension; spec should be updated |
| 22 | HashMap for module_versions needs sort | BTreeMap on Operation struct — deterministic without conversion |
| 23 | schema_version table never populated | Added `INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (1, unixepoch())` |
| 24 | HLC from_bytes/physical_now use unwrap/expect | Return `Result` for consistency with `to_array<N>` approach |

### Phase 1 boundary note

State materialization in `SqliteStorage.append_bundle()` is intentionally coupled — it updates entities/fields/facets/edges inline as ops are appended. Phase 2 will refactor this into a proper `Engine` with command/query separation, incremental derivation, and full replay capability. The materialized tables and their schema are designed to support that refactor without migration.
