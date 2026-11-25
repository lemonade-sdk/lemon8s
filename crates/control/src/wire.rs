use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

/// Why a wire value was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    Name,
    KeyHash,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Name => write!(
                f,
                "invalid name: 1-{} characters of [A-Za-z0-9{}]",
                Name::MAX_LEN,
                Name::PUNCTUATION
            ),
            WireError::KeyHash => f.write_str("invalid key hash: 64 lowercase hex digits"),
        }
    }
}

impl std::error::Error for WireError {}

/// An identifier: user, deployment, node, model, compartment or reason code. The character set
/// and length rule out free text, so no call field can carry prompt or output content.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Name(pub(crate) String);

impl Name {
    pub const MAX_LEN: usize = 255;
    /// What a name may hold besides ASCII letters and digits; `-` last, for a regex class.
    const PUNCTUATION: &str = "._:/-";

    pub fn new(name: impl Into<String>) -> Result<Self, WireError> {
        let name = name.into();
        let valid = (1..=Self::MAX_LEN).contains(&name.len())
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || Self::PUNCTUATION.as_bytes().contains(&b));
        if valid {
            Ok(Self(name))
        } else {
            Err(WireError::Name)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Name {
    type Error = WireError;

    fn try_from(name: String) -> Result<Self, WireError> {
        Self::new(name)
    }
}

impl From<Name> for String {
    fn from(name: Name) -> String {
        name.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The rule of `Name::new` as the API server checks it, for the custom resources' tags and names.
impl JsonSchema for Name {
    fn schema_name() -> Cow<'static, str> {
        "Name".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "minLength": 1,
            "maxLength": Self::MAX_LEN,
            "pattern": format!("^[A-Za-z0-9{}]+$", Self::PUNCTUATION),
        })
    }
}

/// SHA-256 of a session API key. Control stores and compares only this, never the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct KeyHash(pub [u8; 32]);

impl TryFrom<String> for KeyHash {
    type Error = WireError;

    fn try_from(hex: String) -> Result<Self, WireError> {
        if hex.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(WireError::KeyHash);
        }
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(&hex, &mut bytes).map_err(|_| WireError::KeyHash)?;
        Ok(Self(bytes))
    }
}

impl From<KeyHash> for String {
    fn from(hash: KeyHash) -> String {
        hex::encode(hash.0)
    }
}
