//! The ports the use cases talk to (CLAUDE.md 3).
//!
//! Infrastructure implements these; the application never names an implementation. That is
//! what keeps SQLite, MQTT and 健身管 out of the business rules and lets every use case be
//! tested against in-memory fakes with no database and no broker (CLAUDE.md 24).
//!
//! `async fn` in trait is used deliberately (see ADR 0002): the real adapters are async, a
//! synchronous port would force the hub to block a Tokio worker. The cost is that these
//! traits are not `dyn`-compatible, so the use cases are generic over them.

use domain::{AthleteState, Instant, Interpreted, MemberRef, Session};
use mqtt::CommitOutcome;

/// A raw reader event on its way to the immutable store (CLAUDE.md 16, 19).
///
/// Identifiers stay as the wire spelled them: normalisation into `DeviceId`/`ReaderId` is
/// interpretation, and the raw row must keep what actually arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRead {
    pub device_id: String,
    pub reader_id: String,
    pub boot_id: i64,
    pub sequence: i64,
    pub tag_id: String,
    /// Official timing (CLAUDE.md 11, 17).
    pub detected_at: Instant,
    /// Diagnostics only. Never a source of results.
    pub received_at: Instant,
}

/// What the store did with a raw event it has now durably committed.
///
/// The row id is carried so the interpretation can be linked back to the read it came from;
/// `outcome` distinguishes a first delivery from a redelivery, which is how duplicate
/// business processing is avoided while duplicate delivery stays legal (CLAUDE.md 16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawCommit {
    pub raw_event_id: i64,
    pub outcome: CommitOutcome,
}

/// One interpretation on its way to the store. Borrowed rather than owned: the caller
/// already holds all of it and this crosses the port once.
#[derive(Clone, Copy, Debug)]
pub struct InterpretedWrite<'a> {
    pub session_id: &'a str,
    pub athlete_id: &'a str,
    /// `None` for an event an operator added by hand (CLAUDE.md 20).
    pub raw_event_id: Option<i64>,
    pub event: &'a Interpreted,
}

/// An audit record for anything an operator changed (CLAUDE.md 20; ADR 0001 D1).
///
/// `operator` is the device name, not a person: there is no login, and D1 accepted that
/// trade for zero friction on the gym floor. The field can become an identity later
/// without changing this shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    pub at: Instant,
    pub operator: String,
    /// What was done, e.g. `SESSION_REOPEN`.
    pub action: String,
    /// What it was done to: a session id, an athlete id, a tag.
    pub subject: String,
    /// Required for destructive actions, optional for the rest (ADR 0001 D1).
    pub reason: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// The hub's own database (CLAUDE.md 19, 20, 21), implemented by `crates/storage`.
///
/// One port rather than four: every method writes to or reads from the same local SQLite
/// file, shares one error type, and a split would only multiply generic parameters on the
/// use cases without decoupling anything that can actually be swapped separately.
///
/// The one contract that matters: `commit_raw` returns `Ok` **only** after the durable
/// commit succeeded (CLAUDE.md 15; ADR 0002). A store that buffers and writes later must
/// not return `Ok` yet, because the ACK the hub sends on the strength of it releases the
/// only other copy of the event.
#[allow(async_fn_in_trait)]
pub trait HubStore {
    type Error;

    /// Append a raw event. Must be idempotent on `device_id + boot_id + sequence`:
    /// a redelivery reports `AlreadyStored` and the id of the existing row, never a
    /// second row (CLAUDE.md 16).
    async fn commit_raw(&self, raw: &RawRead) -> Result<RawCommit, Self::Error>;

    async fn commit_interpreted(&self, write: InterpretedWrite<'_>) -> Result<i64, Self::Error>;

    async fn save_session(&self, session: &Session, created_at: Instant)
        -> Result<(), Self::Error>;

    async fn save_athlete(
        &self,
        session_id: &str,
        athlete_id: &str,
        display_name: &str,
        bib: i64,
    ) -> Result<(), Self::Error>;

    /// The session to resume after a restart (CLAUDE.md 21).
    async fn active_session(&self) -> Result<Option<Session>, Self::Error>;

    /// Rebuild every athlete by replaying the non-voided interpreted events (CLAUDE.md 21).
    async fn rebuild_athletes(&self, session_id: &str) -> Result<Vec<AthleteState>, Self::Error>;

    async fn session_created_at(&self, session_id: &str) -> Result<Option<Instant>, Self::Error>;

    /// How many live (non-voided) exceptions the session has recorded, for the operator's
    /// inbox badge after a restart (ADR 0001 D4). Counted in the store rather than tracked
    /// in memory, because the badge must survive the process that produced it.
    async fn exception_count(&self, session_id: &str) -> Result<usize, Self::Error>;

    async fn record_audit(&self, entry: &AuditEntry) -> Result<(), Self::Error>;
}

/// 健身管, the member system of record (CLAUDE.md 7.1).
///
/// Direction settled with the user on 2026-08-27: **the hub calls them**, keyed by a member
/// id read off a QR code, and gets back the basic profile that `MemberRef` already models.
/// The endpoint, the authentication and the payload are still unknown (docs/open-issues.md),
/// so this port is all that exists — an HTTP client written against a guessed contract would
/// have to be thrown away, and would look like knowledge the project does not have.
///
/// `Ok(None)` means the directory answered and does not know that member; an unreachable
/// directory is an `Err`. The difference matters: one is an answer, the other is a fault.
///
/// Nothing here gates timing. Confirmed 2026-08-27: if 健身管 returns the member they may
/// be timed, whatever their `MembershipStatus` says.
#[allow(async_fn_in_trait)]
pub trait MemberDirectory {
    type Error;

    async fn lookup(&self, member_id: &str) -> Result<Option<MemberRef>, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DirectoryError {
    /// No 健身管 client exists yet, because the contract has not been agreed.
    #[error("the 健身管 member directory is not configured")]
    NotConfigured,
}

/// The only implementation that can honestly be shipped today: it always reports that it
/// is not configured. It exists so the wiring, the error path and the call sites are real
/// and testable before the contract arrives, and so nothing has to be invented in the
/// meantime (CLAUDE.md 28).
#[derive(Clone, Copy, Debug, Default)]
pub struct UnconfiguredDirectory;

impl MemberDirectory for UnconfiguredDirectory {
    type Error = DirectoryError;

    async fn lookup(&self, _member_id: &str) -> Result<Option<MemberRef>, DirectoryError> {
        Err(DirectoryError::NotConfigured)
    }
}
