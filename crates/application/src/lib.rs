//! The application layer: use cases that orchestrate the domain (CLAUDE.md 3, 4).
//!
//! Everything here is a sequence of steps over `domain` types and ports. The rules are in
//! `domain`; the wiring, the ordering and the persistence contracts are here. Nothing above
//! this layer -- HTTP handlers, MQTT callbacks, Tauri commands -- may hold business logic
//! (CLAUDE.md 29), which is exactly what this crate exists to make possible.
//!
//! Dependencies point inward (CLAUDE.md 3): `domain` for the rules, `contract` for the wire
//! contract, and ports for everything else. No axum, no sqlx, no OS-specific API, so the
//! whole crate compiles and tests with no database, no broker and no HTTP (CLAUDE.md 24).
//!
//! ## Known gaps
//!
//! * Being finished by a class-duration rule is derived on each tick, never stored. That is
//!   deliberate (see [`finish`]), but it means a finish is only as durable as the policy.
//! * `Session::interpreted_event_count` is read back from the session row rather than
//!   re-counted from the log, so a crash between writing an interpretation and saving the
//!   session can leave it one behind. It gates only ARMED -> DRAFT (ADR 0001 D2); athlete
//!   state and the exception badge are both re-derived and are unaffected.

pub mod checkin;
pub mod config;
pub mod devices;
pub mod exceptions;
pub mod finish;
pub mod health;
pub mod ingest;
pub mod live;
pub mod live_session;
pub mod operator;
pub mod ports;
pub mod readers;
pub mod recover;
pub mod results;
pub mod session;
pub mod stages;
pub mod templates;

pub use checkin::{enter, Entrant};
pub use config::configure;
pub use devices::{note_device_seen, note_device_status, DeviceHealth, DeviceReport};
pub use finish::{apply_finish_policy, end_class};
pub use health::{health, health_with_version, Blocker, Health};
pub use ingest::{ingest_read, IngestError, IngestOutcome, Ingested};
pub use live::{
    checkin_view, course_view, last_event_age_ms, reader_views, snapshot, AthleteView,
    CheckInAthlete, CheckInView, CourseStation, ReaderView, Snapshot, SplitView,
};
pub use live_session::LiveSession;
pub use operator::{OperatorCommand, OperatorError};
pub use ports::{
    AuditEntry, DirectoryError, HubStore, InterpretedWrite, MemberDirectory, RawCommit, RawRead,
    SeenReader, StoredException, StoredRawRead, UnconfiguredDirectory,
};
pub use readers::{register_reader, unregister_reader, unregistered_readers};
pub mod assets;
pub use assets::{
    delete_venue_asset, save_venue_asset, venue_asset, AssetError, VenueAsset, MAX_ASSET_BYTES,
    VENUE_LOGO,
};
pub mod settings;
pub use recover::{resume_or_start, Recovery, RosterEntry, SessionPlan};
pub use results::{live_results, results, Ordering, ResultRow, SessionResults, SplitRow};
pub use settings::{
    save_venue_setting, venue_settings, SettingError, VenueSettings, DEFAULT_LIVE_PAGE_MS,
    DEFAULT_LIVE_PAGE_SIZE, LIVE_PAGE_LAYOUTS, LIVE_PAGE_MS, LIVE_PAGE_SIZE,
};
pub use stages::{current_expectation, current_stage, stages, StageStatus, StageView};
pub use templates::{
    create_class, delete_template, duplicate_template, list_templates, save_template, NewClass,
    TemplateError,
};
