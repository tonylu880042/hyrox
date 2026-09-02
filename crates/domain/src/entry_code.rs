//! The entry code: six characters that are a walk-in entrant's whole identity (ADR 0011).
//!
//! It is the athlete id, the number on the QR the entrant carries, and the number they type
//! afterwards to find their result. One value doing all three jobs, so nothing can fall out
//! of step.
//!
//! Pure on purpose. Generating one needs randomness, which belongs to the application layer:
//! a domain that reaches for entropy cannot be replayed (CLAUDE.md 21, 29).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Crockford's base 32, minus `U`. No `I`, `L` or `O`, because at arm's length on a phone
/// they are `1`, `1` and `0`; no `U` so a code cannot spell anything unfortunate.
const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const LENGTH: usize = 6;

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EntryCode(String);

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EntryCodeError {
    /// Not six characters once separators are dropped.
    WrongLength(usize),
    /// A character that is not in the alphabet and is not one of the substitutions.
    BadCharacter(char),
}

impl EntryCode {
    pub const ALPHABET: &'static str = ALPHABET;
    pub const LENGTH: usize = LENGTH;

    /// Turns a number into a code. The caller owns the randomness and the collision check:
    /// the roster is what decides whether a code is free, not this function.
    ///
    /// Values wider than 30 bits wrap rather than being rejected -- the caller passes on
    /// whatever its entropy source produced, and refusing it would only push the modulo
    /// somewhere less obvious.
    pub fn encode(value: u64) -> Self {
        let alphabet: Vec<char> = ALPHABET.chars().collect();
        let base = alphabet.len() as u64;
        let mut n = value;
        let mut out = String::with_capacity(LENGTH);
        for _ in 0..LENGTH {
            out.insert(0, alphabet[(n % base) as usize]);
            n /= base;
        }
        Self(out)
    }

    /// Reads a code the way a person typed it: any case, with spaces or dashes, and with
    /// the substitutions they make when reading one off a screen.
    pub fn parse(raw: &str) -> Result<Self, EntryCodeError> {
        let mut out = String::with_capacity(LENGTH);
        for c in raw.chars() {
            if c.is_whitespace() || c == '-' || c == '_' {
                continue;
            }
            let c = match c.to_ascii_uppercase() {
                'O' => '0',
                'I' | 'L' => '1',
                other => other,
            };
            if !ALPHABET.contains(c) {
                return Err(EntryCodeError::BadCharacter(c));
            }
            out.push(c);
        }
        if out.len() != LENGTH {
            return Err(EntryCodeError::WrongLength(out.len()));
        }
        Ok(Self(out))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EntryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for EntryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EntryCode({})", self.0)
    }
}

impl fmt::Display for EntryCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength(n) => write!(f, "an entry code is {LENGTH} characters, got {n}"),
            Self::BadCharacter(c) => write!(f, "{c:?} is not part of an entry code"),
        }
    }
}

impl std::error::Error for EntryCodeError {}

impl TryFrom<String> for EntryCode {
    type Error = EntryCodeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<EntryCode> for String {
    fn from(code: EntryCode) -> Self {
        code.0
    }
}
