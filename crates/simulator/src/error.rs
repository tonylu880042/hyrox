//! Every way the emulated edge can refuse (CLAUDE.md 29).

use contract::{DeviceIdError, ReaderId, ReaderIdError};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    /// A zero or negative window would suppress everything or nothing; neither is a
    /// suppression policy (CLAUDE.md 14).
    #[error("absent_timeout must be a positive number of milliseconds (got {0})")]
    NonPositiveTimeout(i64),
    #[error("journal capacity must be at least one event")]
    ZeroCapacity,
    #[error("journal warning threshold must be 1..=100 percent (got {0})")]
    WarnThreshold(u8),
    #[error("journal reclaim batch must be at least one entry")]
    ZeroReclaimBatch,
    #[error("a device needs at least one reader")]
    NoReaders,
    // Both halves of an edge identity come from `domain` (CLAUDE.md 7.3), so a
    // misconfigured simulator fails on exactly the rule the hub would apply.
    #[error("identity: {0}")]
    Device(#[from] DeviceIdError),
    #[error("identity: {0}")]
    Reader(#[from] ReaderIdError),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum JournalError {
    /// Reported rather than resolved by overwriting: an unacknowledged event is one the
    /// hub has never seen, and losing it is the one outcome the system may not have
    /// (CLAUDE.md 18, 31).
    #[error("journal full: {pending} unacknowledged events occupy all {capacity} slots")]
    Full { pending: usize, capacity: usize },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DeviceError {
    #[error("reader {0} is not attached to this device")]
    UnknownReader(ReaderId),
    #[error(transparent)]
    Journal(#[from] JournalError),
}
