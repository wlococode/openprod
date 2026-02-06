use serde::{Deserialize, Serialize};

use crate::ids::{BlobHash, EntityId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldValue {
    Null,
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Timestamp(i64),
    EntityRef(EntityId),
    BlobRef(BlobHash),
    Bytes(Vec<u8>),
}

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

    pub fn to_msgpack(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec(self)
    }

    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(bytes)
    }
}
