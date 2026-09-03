//! Edge device and reader identity (CLAUDE.md 7.3).
//!
//! `device_id` is the device's base MAC and nothing else: a random UUID would be lost on
//! reflash and would make a swapped board indistinguishable from a new one.
//! `reader_id` is a separate type because one device may host several readers.
//!
//! The id is *normalised*, not decorated (ADR 0015). Six spellings of one MAC --
//! `:`/`-`/`.` separators, upper or lower case -- would become six rows in the reader
//! registry, and the lookup would miss on punctuation alone. That failure is silent: the
//! hub reports no error, it just files every read from that reader as UNKNOWN_READER.

use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// A base MAC is six bytes, so the canonical id is twelve hex digits.
const MAC_HEX_LEN: usize = 12;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize)]
#[serde(transparent)]
pub struct DeviceId(String);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceIdError {
    WrongLength { found: usize },
    NotHex,
}

impl fmt::Display for DeviceIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength { found } => {
                write!(f, "device id needs {MAC_HEX_LEN} hex digits, found {found}")
            }
            Self::NotHex => f.write_str("device id must be hexadecimal"),
        }
    }
}

impl std::error::Error for DeviceIdError {}

impl DeviceId {
    /// Parse a canonical `a4cf128b3d91`. Case-insensitive on input because CLAUDE.md writes
    /// reader/device ids in both cases (§8 upper, §16 lower); one stored form keeps a lookup
    /// from missing on case alone. Separators are **not** accepted here -- they belong to
    /// [`Self::from_mac_str`], which is what config files and human input go through.
    pub fn parse(raw: &str) -> Result<Self, DeviceIdError> {
        let lowered = raw.to_ascii_lowercase();
        if lowered.len() != MAC_HEX_LEN {
            return Err(DeviceIdError::WrongLength {
                found: lowered.len(),
            });
        }
        if !lowered.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(DeviceIdError::NotHex);
        }
        Ok(Self(lowered))
    }

    /// The only sanctioned way to mint an id for a device the hub has not seen before.
    pub fn from_mac(mac: [u8; 6]) -> Self {
        let mut s = String::with_capacity(MAC_HEX_LEN);
        for byte in mac {
            s.push(hex_digit(byte >> 4));
            s.push(hex_digit(byte & 0x0f));
        }
        Self(s)
    }

    /// Accepts a MAC as humans and config files write it: `a4:cf:12:8b:3d:91`,
    /// `A4-CF-12-8B-3D-91` or bare hex. Separators are stripped here and never accepted
    /// by `parse`, so the canonical form stays single-valued.
    pub fn from_mac_str(raw: &str) -> Result<Self, DeviceIdError> {
        let hex: String = raw
            .chars()
            .filter(|c| *c != ':' && *c != '-' && *c != '.')
            .flat_map(char::to_lowercase)
            .collect();
        if hex.len() != MAC_HEX_LEN {
            return Err(DeviceIdError::WrongLength { found: hex.len() });
        }
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(DeviceIdError::NotHex);
        }
        Ok(Self(hex))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Deserialisation goes through `parse`, so an id arriving on the wire is validated at the
/// boundary rather than trusted (CLAUDE.md 16). Written by hand rather than derived because
/// the serialised form stays the bare string `#[serde(transparent)]` already produces; only
/// the way back in gains a check.
impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

fn hex_digit(nibble: u8) -> char {
    char::from(if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + nibble - 10
    })
}

/// What an edge device says about its own journal (CLAUDE.md 18).
///
/// Here rather than in `crates/transport` because two layers now need the same vocabulary:
/// the transport decodes it off the status topic, and the application carries it on the
/// operator's reader health view (ADR 0001 D5). `transport` re-exports this type, so the
/// wire spelling is unchanged -- the serialised form is still SCREAMING_SNAKE_CASE.
///
/// The hub never derives one of these. A warning is the device's own assessment; inventing
/// one from a message count would be the hub guessing at firmware internals.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeviceWarning {
    /// Past the configured warning threshold: still recording, but an operator should look.
    JournalNearlyFull,
    /// No reclaimable space left. The next RF read cannot be journalled.
    JournalFull,
}

/// Reader identity, scoped to its device. Kept apart from `DeviceId` so a `reader_id`
/// can never be used on its own to identify hardware (CLAUDE.md 7.3).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize)]
#[serde(transparent)]
pub struct ReaderId(String);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReaderIdError {
    Empty,
    InvalidCharacter { found: char },
}

impl fmt::Display for ReaderIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("reader id must not be empty"),
            Self::InvalidCharacter { found } => {
                write!(
                    f,
                    "reader id must be alphanumeric, '-' or '_' (found {found:?})"
                )
            }
        }
    }
}

impl std::error::Error for ReaderIdError {}

impl ReaderId {
    /// Lower-cased on parse for the same reason as `DeviceId`. Restricted to characters
    /// that survive MQTT topics and URLs unescaped, so an id is never silently re-encoded
    /// between the wire and the registry.
    pub fn parse(raw: &str) -> Result<Self, ReaderIdError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(ReaderIdError::Empty);
        }
        if let Some(bad) = trimmed
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
        {
            return Err(ReaderIdError::InvalidCharacter { found: bad });
        }
        Ok(Self(trimmed.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ReaderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Validated on the way in, for the same reason as [`DeviceId`]: a reader id that cannot be
/// looked up in the registry must fail as a malformed payload, not as a mystery miss later.
impl<'de> Deserialize<'de> for ReaderId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}
