use serde::{Deserialize, Deserializer, Serialize, Serializer};
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

        impl Default for $name {
            fn default() -> Self {
                Self::new()
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
        write!(
            f,
            "ActorId({:02x}{:02x}{:02x}{:02x})",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0[..8] {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signature([u8; 64]);

impl Signature {
    pub fn from_bytes(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
        let arr: [u8; 64] = bytes
            .try_into()
            .map_err(|v: Vec<u8>| serde::de::Error::invalid_length(v.len(), &"64 bytes"))?;
        Ok(Self(arr))
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
