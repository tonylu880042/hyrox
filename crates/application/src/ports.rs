//! The ports the use cases talk to (CLAUDE.md 3).
//!
//! Infrastructure implements these; the application never names an implementation. That is
//! what keeps SQLite, MQTT and 健身管 out of the business rules and lets every use case be
//! tested against in-memory fakes with no database and no broker (CLAUDE.md 24).
//!
//! Asynchronous by design (see ADR 0002): the real adapters are async, and a synchronous
//! port would force the hub to block a Tokio worker. The cost is that these traits are not
//! `dyn`-compatible, so the use cases are generic over them.
//!
//! The methods are written as `-> impl Future<..> + Send` rather than as `async fn`, which
//! is the same thing with one promise added: the returned future is `Send`. An `async fn`
//! in a trait leaves that unknown to generic callers, and every use case in this crate is
//! generic over the port -- so an HTTP handler awaiting one could not be proved safe to run
//! on a multi-threaded executor, and `crates/api` would not compile (ADR 0007). Adapters
//! still write plain `async fn`; only the promise is new.

use contract::CommitOutcome;
use domain::{
    AthleteState, BindingLedger, ExceptionReason, ExerciseLibrary, Instant, Interpreted,
    MemberRef, PhysicalStation, ReaderRegistration, ReaderRegistry, Session, SessionConfig,
    StationMap, TagBinding, WorkoutTemplate,
};
use std::future::Future;

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

/// A raw read fetched back out of the immutable store, with the row id an interpretation
/// has to point at (CLAUDE.md 19).
///
/// Identifiers come back as the wire spelled them, exactly as they went in: resolving them
/// is interpretation, and interpretation is not the raw store's job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRawRead {
    pub raw_event_id: i64,
    pub device_id: String,
    pub reader_id: String,
    /// Official timing (CLAUDE.md 11, 17). Claiming replays in this order.
    pub detected_at: Instant,
}

/// One live exception, as the operator's inbox lists it (ADR 0001 D4).
///
/// `interpreted_event_id` is what an operator action names: voiding is done to the
/// interpretation, never to the raw read, which stays immutable (CLAUDE.md 19, 20).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredException {
    pub interpreted_event_id: i64,
    pub athlete_id: String,
    pub reason: ExceptionReason,
    /// Official timing (CLAUDE.md 11, 17): when the reader saw it, not when it was stored.
    pub at: Instant,
    /// The raw read behind it, when there is one. `None` for an exception an operator added.
    pub raw_event_id: Option<i64>,
}

/// A `(device_id, reader_id)` the hub has heard from, whether or not it is configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenReader {
    pub device_id: String,
    pub reader_id: String,
    /// Official timing, as everywhere else: when the reader saw a tag (CLAUDE.md 17).
    pub last_seen: Instant,
    pub reads: i64,
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
pub trait HubStore {
    type Error;

    /// Append a raw event. Must be idempotent on `device_id + boot_id + sequence`:
    /// a redelivery reports `AlreadyStored` and the id of the existing row, never a
    /// second row (CLAUDE.md 16).
    fn commit_raw(
        &self,
        raw: &RawRead,
    ) -> impl Future<Output = Result<RawCommit, Self::Error>> + Send;

    fn commit_interpreted(
        &self,
        write: InterpretedWrite<'_>,
    ) -> impl Future<Output = Result<i64, Self::Error>> + Send;

    fn save_session(
        &self,
        session: &Session,
        created_at: Instant,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Put one person on a session's roster.
    ///
    /// `member_id` is `None` for a walk-in (ADR 0010): an athlete is identified by
    /// `athlete_id`, and the 健身管 reference is provenance rather than a precondition.
    /// Keyed on `(session_id, athlete_id)`, so a door tablet's double tap is one row.
    fn save_athlete(
        &self,
        session_id: &str,
        athlete_id: &str,
        display_name: &str,
        bib: i64,
        member_id: Option<&str>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Records a finish the finish rule decided, rather than a read (migration 0010).
    ///
    /// A ClassDuration finish has no event behind it, so replay cannot rebuild it: without
    /// this the class comes back from a restart still running. `None` writes NULL -- nobody
    /// was finished by a rule.
    fn save_athlete_finish(
        &self,
        session_id: &str,
        athlete_id: &str,
        finished_at: Option<Instant>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// The session to resume after a restart (CLAUDE.md 21).
    fn active_session(&self) -> impl Future<Output = Result<Option<Session>, Self::Error>> + Send;

    /// One session by id, whether or not it is the active one.
    ///
    /// Results outlive the class that produced them: `/result/{id}` has to be able to name
    /// a session the hub is no longer running (CLAUDE.md 22).
    fn session(
        &self,
        session_id: &str,
    ) -> impl Future<Output = Result<Option<Session>, Self::Error>> + Send;

    /// Store the course and the policies a session was armed with (ADR 0004).
    ///
    /// Written once, when the session is armed. Editing configuration is only legal in
    /// DRAFT (ADR 0001 D2), so a running class cannot have its finish rule changed underneath
    /// it -- which is the whole point of storing it.
    fn save_session_config(
        &self,
        config: &SessionConfig,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// The configuration a session was armed with, or `None` for a session stored before
    /// configuration was persisted. `None` is not an error, but it is not a default either:
    /// the caller must decide, and say so (see [`crate::Recovery`]).
    fn session_config(
        &self,
        session_id: &str,
    ) -> impl Future<Output = Result<Option<SessionConfig>, Self::Error>> + Send;

    /// Register or reconfigure one reader (CLAUDE.md 8). Keyed on `(device_id, reader_id)`,
    /// so re-registering a reader replaces its mapping rather than adding a second one.
    fn save_reader(
        &self,
        registration: &ReaderRegistration,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// The venue's reader map. Readers belong to the building, not to a session, so this
    /// is not scoped by session id.
    fn readers(&self) -> impl Future<Output = Result<ReaderRegistry, Self::Error>> + Send;

    /// Append or close one binding row (CLAUDE.md 7.2; ADR 0001 D3).
    ///
    /// Append-only, exactly like the domain ledger: an implementation may stamp
    /// `unbound_at` on a row it already holds, and may never rewrite who held the tag.
    fn save_binding(
        &self,
        binding: &TagBinding,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Every binding ever made, closed ones included, oldest first. Dropping the closed
    /// ones would leave "who was wearing this band at 10:15" unanswerable (CLAUDE.md 20).
    fn bindings(&self) -> impl Future<Output = Result<BindingLedger, Self::Error>> + Send;

    /// Distinct tag ids seen by any reader since `since`, oldest first.
    ///
    /// The check-in queue is derived from this rather than remembered, so a crash cannot
    /// lose it (ADR 0001 D3): a tag that was read and is still unbound is still waiting.
    fn raw_tags_since(
        &self,
        since: Instant,
    ) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send;

    /// Every `(device_id, reader_id)` the hub has ever heard from, with when it was last
    /// heard and how many reads it has produced.
    ///
    /// This is what makes installing a venue possible without anyone copying a MAC address
    /// off a sticker: tap the antenna, and the reader that just appeared is the one to
    /// assign. Derived from `raw_events` rather than remembered, because a read the hub
    /// could not attribute is still stored (CLAUDE.md 31).
    fn reader_keys_seen(
        &self,
    ) -> impl Future<Output = Result<Vec<SeenReader>, Self::Error>> + Send;

    /// Stored reads of one tag that no interpretation points at yet, oldest first
    /// (ADR 0001 D3, retroactive claim).
    ///
    /// "No interpretation points at it" is what makes claiming idempotent: a read claimed
    /// once is never replayed into a second interpreted event, however often a band is
    /// rebound. Tag comparison ignores case, because the raw row keeps the wire spelling
    /// while `TagId` upper-cases.
    fn unclaimed_reads_for_tag(
        &self,
        tag_id: &str,
        since: Instant,
    ) -> impl Future<Output = Result<Vec<StoredRawRead>, Self::Error>> + Send;

    /// `(athlete_id, bib)` for one session's roster.
    ///
    /// Separate from `rebuild_athletes` because a bib is not replayed: `AthleteState` is
    /// derived from the interpreted log, and the number on somebody's vest is a roster fact
    /// the door assigned (ADR 0010). Results for a session the hub is no longer running
    /// have to read it back from here rather than counting rows.
    fn athlete_bibs(
        &self,
        session_id: &str,
    ) -> impl Future<Output = Result<Vec<(String, i64)>, Self::Error>> + Send;

    /// Rebuild every athlete by replaying the non-voided interpreted events (CLAUDE.md 21).
    fn rebuild_athletes(
        &self,
        session_id: &str,
    ) -> impl Future<Output = Result<Vec<AthleteState>, Self::Error>> + Send;

    fn session_created_at(
        &self,
        session_id: &str,
    ) -> impl Future<Output = Result<Option<Instant>, Self::Error>> + Send;

    /// How many live (non-voided) exceptions the session has recorded, for the operator's
    /// inbox badge after a restart (ADR 0001 D4). Counted in the store rather than tracked
    /// in memory, because the badge must survive the process that produced it.
    fn exception_count(
        &self,
        session_id: &str,
    ) -> impl Future<Output = Result<usize, Self::Error>> + Send;

    /// The live exceptions themselves, oldest first, for the operator's inbox (ADR 0001 D4).
    /// Voided ones are excluded: clearing one in the inbox clears it from the list as well
    /// as from the badge.
    fn exceptions(
        &self,
        session_id: &str,
    ) -> impl Future<Output = Result<Vec<StoredException>, Self::Error>> + Send;

    /// Void one interpretation (CLAUDE.md 20; ADR 0001 D4). Reports whether a row was
    /// actually voided, so an operator who named an id that does not exist is told so
    /// rather than shown a success.
    ///
    /// The interpretation is marked voided, never deleted, and the raw read it points at is
    /// not touched at all (CLAUDE.md 19). A voided event must disappear from every replay,
    /// which is what makes the derived values recomputable after the fact.
    /// Marks one exception as looked at and left alone (ADR 0001 D4; migration 0011).
    ///
    /// Not a void: the interpretation stays in the log and in every replay. False means no
    /// such open exception.
    fn acknowledge_interpreted(
        &self,
        interpreted_event_id: i64,
        at: Instant,
        operator: &str,
        reason: Option<&str>,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;

    fn void_interpreted(
        &self,
        interpreted_event_id: i64,
        at: Instant,
        operator: &str,
        reason: &str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;

    /// Copy the whole database to `path`, while the hub keeps running (ADR 0012).
    ///
    /// A port rather than a detail of the SQLite adapter, because the operator surface is
    /// what decides *when* a backup is taken -- the nightly window asks the hub, and the
    /// hub is the only process allowed to touch the file (ADR 0009). The implementation
    /// must produce a consistent snapshot without anyone copying `-wal` by hand.
    fn backup_to(
        &self,
        path: &std::path::Path,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Removes one reader's mapping. The reads it already produced are untouched: they
    /// live in `raw_events`, which is immutable (CLAUDE.md 19).
    fn delete_reader(
        &self,
        device_id: &str,
        reader_id: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Every venue setting that has been chosen, as raw key/value pairs (M6 follow-up).
    /// Parsing and defaulting belong above this: a store returns what is stored.
    fn venue_settings(
        &self,
    ) -> impl Future<Output = Result<Vec<(String, String)>, Self::Error>> + Send;

    /// Stores one, replacing any previous value for the same key.
    fn save_venue_setting(
        &self,
        key: &str,
        value: &str,
        at: Instant,
        by: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// One of the venue's images, or `None` if nobody uploaded it (M6 follow-up).
    fn venue_asset(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<crate::assets::VenueAsset>, Self::Error>> + Send;

    fn save_venue_asset(
        &self,
        key: &str,
        media_type: &str,
        bytes: &[u8],
        at: Instant,
        by: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn delete_venue_asset(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn record_audit(
        &self,
        entry: &AuditEntry,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    // --- the workout library (ADR 0008) --------------------------------------------------
    //
    // Venue configuration, not session data: templates, exercises and machines outlive any
    // one class, so none of these is scoped by session id.

    /// Insert or replace one template, keyed on its id. Replacing is how an edit is saved;
    /// the version the template carries is what distinguishes one save from the next.
    fn save_template(
        &self,
        template: &WorkoutTemplate,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn template(
        &self,
        template_id: &str,
    ) -> impl Future<Output = Result<Option<WorkoutTemplate>, Self::Error>> + Send;

    /// Every template, system ones included, in listing order.
    fn templates(&self) -> impl Future<Output = Result<Vec<WorkoutTemplate>, Self::Error>> + Send;

    /// Reports whether a row was actually removed, so a coach who named an id that does not
    /// exist is told so rather than shown a success.
    ///
    /// A real delete, not a void: a template is a plan, not a record of something that
    /// happened. Deleting one cannot touch a class that already ran, because the class runs
    /// off its own snapshot (ADR 0004, 0008).
    fn delete_template(
        &self,
        template_id: &str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;

    fn save_exercise(
        &self,
        exercise: &domain::Exercise,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn exercises(&self) -> impl Future<Output = Result<ExerciseLibrary, Self::Error>> + Send;

    fn save_station(
        &self,
        station: &PhysicalStation,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn stations(&self) -> impl Future<Output = Result<StationMap, Self::Error>> + Send;
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
