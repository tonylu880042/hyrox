//! The application layer: use cases that orchestrate the domain (CLAUDE.md 3, 4).
//!
//! Everything here is a sequence of steps over `domain` types and ports. The rules are in
//! `domain`; the wiring, the ordering and the persistence contracts are here. Nothing above
//! this layer -- HTTP handlers, MQTT callbacks, Tauri commands -- may hold business logic
//! (CLAUDE.md 29), which is exactly what this crate exists to make possible.
//!
//! Dependencies point inward (CLAUDE.md 3): `domain` for the rules, `mqtt` for the wire
//! contract, and ports for everything else. No axum, no sqlx, no OS-specific API, so the
//! whole crate compiles and tests with no database, no broker and no HTTP (CLAUDE.md 24).
//!
//! ## Known gaps
//!
//! * The reader registry and the binding ledger are held in memory and supplied at startup:
//!   Phase 1 has no tables for either, so they do not survive a restart the way events do.
//! * A read of an unbound tag is stored and listed for `/checkin`, but binding the tag does
//!   not yet go back and re-interpret the reads that happened before it (ADR 0001 D3 asks
//!   for that). Nothing is lost -- the raw rows are there -- but the claim is manual today.
//! * Being finished by a class-duration rule is derived on each tick, never stored. That is
//!   deliberate (see [`finish`]), but it means a finish is only as durable as the policy.

pub mod checkin;
pub mod finish;
pub mod ingest;
pub mod live;
pub mod live_session;
pub mod operator;
pub mod ports;
pub mod recover;
pub mod session;

pub use finish::{apply_finish_policy, end_class};
pub use ingest::{ingest_read, Ingested, IngestError, IngestOutcome};
pub use live::{course_view, snapshot, AthleteView, CourseStation, Snapshot};
pub use live_session::LiveSession;
pub use operator::{OperatorCommand, OperatorError};
pub use ports::{
    AuditEntry, DirectoryError, HubStore, InterpretedWrite, MemberDirectory, RawCommit, RawRead,
    UnconfiguredDirectory,
};
pub use recover::{resume_or_start, Recovery, RosterEntry, SessionPlan};
