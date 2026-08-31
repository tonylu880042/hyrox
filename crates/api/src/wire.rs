//! The wire shapes: what a screen sends, and what it gets back.
//!
//! Only shapes. Every value in a response is either copied from a use case's return type or
//! is a fact about the delivery layer itself (how many sockets are open, how often the hub
//! pushes). Nothing is decided here (CLAUDE.md 29).
//!
//! Domain documents -- `Course`, `FinishPolicy`, `Snapshot`, `SessionResults` -- travel as
//! themselves rather than through a parallel set of DTOs. They are already the published
//! read model, and a second spelling of them would be a second thing to keep in step.

use application::{
    CheckInAthlete, CheckInView, DeviceHealth, ReaderView, SessionResults, Snapshot, StageView,
    StoredException,
};
use domain::{
    Course, DeviceWarning, ExceptionReason, Exercise, FinishPolicy, Instant, Interpreted,
    Expectation, ReaderMode, Session, SessionConfig, SessionMode, TemplateCategory,
    WorkoutBlock, WorkoutTemplate,
};
use serde::{Deserialize, Serialize};

/// The mandatory data-freshness readout (ADR 0001 D5).
///
/// On **every** read response, including the ones that are not live. D5 calls it mandatory
/// because CLAUDE.md 31's first principle is that no event is lost: without this, a still
/// screen means both "nobody is running" and "the link died ten minutes ago", and the venue
/// cannot tell which. Making it part of the envelope rather than an endpoint of its own is
/// what stops a screen from being written that forgets to ask.
#[derive(Clone, Debug, Serialize)]
pub struct Freshness {
    /// The hub's clock when this response was rendered. A client comparing it against its
    /// own can also see that the two disagree.
    pub now: i64,
    /// Age of the newest interpreted event. `None` means no event exists yet -- which is
    /// not zero, and must not be drawn as fresh.
    pub last_event_age_ms: Option<i64>,
    /// Where the live socket is.
    pub websocket_path: &'static str,
    /// How often the hub pushes a snapshot. A screen that has gone noticeably longer than
    /// this without a frame has a dead link, and now has a number to say so with instead of
    /// a guessed timeout.
    pub push_interval_ms: i64,
    /// How many sockets the hub is pushing to right now. The server's half of the liveness
    /// question; the browser knows its own half.
    pub subscribers: usize,
}

/// `GET /api/live` -- the big screen.
#[derive(Debug, Serialize)]
pub struct LiveResponse {
    pub freshness: Freshness,
    pub snapshot: Snapshot,
}

/// `GET /api/coach` -- CLAUDE.md 23, plus the reader warnings that section also asks for.
#[derive(Debug, Serialize)]
pub struct CoachResponse {
    pub freshness: Freshness,
    pub snapshot: Snapshot,
    /// "Device / Reader Warnings" (CLAUDE.md 23), each with its own last-seen age
    /// (ADR 0001 D5).
    pub readers: Vec<ReaderView>,
}

/// `GET /api/session` -- the session and the configuration it is running under.
#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub freshness: Freshness,
    pub session: Session,
    pub config: SessionConfig,
    pub class_elapsed_ms: i64,
    /// Whether `PUT /api/operator/config` would be accepted right now (ADR 0001 D2).
    ///
    /// Answered by `domain::Session`, not by the screen. A UI that greys out its own edit
    /// button using a rule it re-derived would be holding a business rule (CLAUDE.md 6, 29),
    /// and would drift the day the rule changes.
    pub config_editable: bool,
}

/// `GET /api/result/{id}` -- results for any stored session (CLAUDE.md 22).
#[derive(Debug, Serialize)]
pub struct ResultResponse {
    pub freshness: Freshness,
    /// Rows are in bib order and say so; there is no ranking, because the competition
    /// finish rule is undecided (CLAUDE.md 12, 28) and any ordering would imply one.
    pub results: SessionResults,
}

/// `GET /api/operator` -- everything the operator screen keeps on display at once.
#[derive(Debug, Serialize)]
pub struct OverviewResponse {
    pub freshness: Freshness,
    pub session: Session,
    pub config: SessionConfig,
    pub config_editable: bool,
    pub class_elapsed_ms: i64,
    /// Per-reader `last_seen`, which D5 asks `/operator` for specifically.
    pub readers: Vec<ReaderView>,
    pub devices: Vec<DeviceView>,
    /// The exception inbox badge (ADR 0001 D4).
    pub exceptions: usize,
    /// Bands read but not yet claimed. A to-do for `/checkin`, not an error (D3).
    pub pending_tags: usize,
}

/// One edge device as the operator screen shows it (CLAUDE.md 18; ADR 0001 D5).
#[derive(Debug, Serialize)]
pub struct DeviceView {
    pub device_id: String,
    /// How long since the hub heard anything at all from this board.
    pub last_seen_age_ms: i64,
    /// `None` until the device has published a status -- a board can be heard from through
    /// its reads alone, and half a fact beats an invented one.
    pub boot_id: Option<i64>,
    pub pending_events: Option<u64>,
    pub journal_capacity: Option<u64>,
    /// The device's own assessment. The hub relays it and never derives one.
    pub warning: Option<DeviceWarning>,
}

impl DeviceView {
    pub fn of(health: &DeviceHealth, now: Instant) -> Self {
        let report = health.report.as_ref();
        Self {
            device_id: health.device_id.to_string(),
            last_seen_age_ms: now.since(health.last_seen).millis(),
            boot_id: report.map(|r| r.boot_id),
            pending_events: report.map(|r| r.pending_events),
            journal_capacity: report.map(|r| r.journal_capacity),
            warning: health.warning(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ReadersResponse {
    pub freshness: Freshness,
    pub readers: Vec<ReaderView>,
}

/// One item in the exception inbox (ADR 0001 D4).
///
/// Carries the athlete **id** and not their name: the roster lives in the snapshot every
/// operator screen already holds, and joining it there keeps one spelling of a display name.
#[derive(Debug, Serialize)]
pub struct ExceptionView {
    /// What an operator action names. Voiding is done to the interpretation; the raw read
    /// it points at is immutable (CLAUDE.md 19).
    pub interpreted_event_id: i64,
    pub athlete_id: String,
    pub reason: ExceptionReason,
    /// Official timing: when the reader saw it, not when it was stored (CLAUDE.md 17).
    pub at: i64,
    pub age_ms: i64,
    /// `None` for an exception an operator added by hand.
    pub raw_event_id: Option<i64>,
}

impl ExceptionView {
    pub fn of(stored: &StoredException, now: Instant) -> Self {
        Self {
            interpreted_event_id: stored.interpreted_event_id,
            athlete_id: stored.athlete_id.clone(),
            reason: stored.reason.clone(),
            at: stored.at.0,
            age_ms: now.since(stored.at).millis(),
            raw_event_id: stored.raw_event_id,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ExceptionsResponse {
    pub freshness: Freshness,
    pub exceptions: Vec<ExceptionView>,
}

/// `GET /api/checkin` -- the bands waiting, and who still needs one (ADR 0001 D3).
#[derive(Debug, Serialize)]
pub struct CheckInResponse {
    pub freshness: Freshness,
    #[serde(flatten)]
    pub view: CheckInView,
}

/// What a bind or rebind actually did.
#[derive(Debug, Serialize)]
pub struct BindResponse {
    pub freshness: Freshness,
    /// Reads that happened before anyone owned the band and have now been interpreted
    /// (ADR 0001 D3, retroactive claim). Empty is the ordinary case.
    pub claimed: Vec<Interpreted>,
}

/// Athletes a manual class end stopped the clock for (CLAUDE.md 12, as configured).
#[derive(Debug, Serialize)]
pub struct EndClassResponse {
    pub freshness: Freshness,
    pub finished: Vec<String>,
}

/// The body of a write that has nothing to say but its intent.
///
/// `reason` is optional here and required by the use case that needs it -- reopening a
/// closed session, voiding an event, moving an athlete to a different band. Which actions
/// those are is a rule (CLAUDE.md 20; ADR 0001 D1), and rules are not kept in DTOs.
#[derive(Debug, Default, Deserialize)]
pub struct ReasonRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

/// `PUT /api/operator/config` (ADR 0001 D2).
///
/// `finish_policy` is deliberately required. It has a `Default` -- `NotConfigured` -- and
/// letting an omitted field fall into it would let a request quietly remove a class's
/// finish rule, which is precisely the silent substitution ADR 0004 exists to prevent.
#[derive(Debug, Deserialize)]
pub struct ConfigureRequest {
    /// `None` clears the course: a drop-in session with no plan is legal (CLAUDE.md 9.2).
    #[serde(default)]
    pub course: Option<Course>,
    pub finish_policy: FinishPolicy,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /api/operator/readers` (CLAUDE.md 8).
///
/// There is no delete. Nothing in the application layer removes a reader, and inventing
/// removal semantics here would mean deciding what happens to the events already attributed
/// through it. Re-registering the same `(device_id, reader_id)` replaces its mapping, which
/// is what repointing a reader actually is.
#[derive(Debug, Deserialize)]
pub struct RegisterReaderRequest {
    pub device_id: String,
    pub reader_id: String,
    pub station: String,
    #[serde(default)]
    pub zone: Option<String>,
    pub mode: ReaderMode,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /api/checkin/bind` and `/rebind` (ADR 0001 D3).
#[derive(Debug, Deserialize)]
pub struct BindRequest {
    pub tag_id: String,
    pub athlete_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

// --- the workout library (ADR 0008) ---------------------------------------------------------

/// `GET /api/exercises` (workout brief §17).
#[derive(Debug, Serialize)]
pub struct ExercisesResponse {
    pub freshness: Freshness,
    pub exercises: Vec<Exercise>,
}

/// `GET /api/workout-templates` and the answer to every template write.
#[derive(Debug, Serialize)]
pub struct TemplatesResponse {
    pub freshness: Freshness,
    pub templates: Vec<WorkoutTemplate>,
}

/// `GET /api/workout-templates/{id}`.
#[derive(Debug, Serialize)]
pub struct TemplateResponse {
    pub freshness: Freshness,
    pub template: WorkoutTemplate,
}

/// `GET /api/stages` -- every athlete's progress through the snapshot course
/// (workout brief §10).
///
/// Not carried in the pushed snapshot: a full stage list per athlete would multiply the size
/// of every WebSocket frame the big screen receives, and only the coach screen reads it.
#[derive(Debug, Serialize)]
pub struct StagesResponse {
    pub freshness: Freshness,
    pub athletes: Vec<AthleteStages>,
}

#[derive(Debug, Serialize)]
pub struct AthleteStages {
    pub athlete_id: String,
    pub name: String,
    /// Which stage they are on, from 1. `None` before they start and once they are finished.
    pub current_stage: Option<usize>,
    /// How the station they are standing in compares with the plan: EXPECTED, OUT_OF_ORDER,
    /// UNEXPECTED, or UNKNOWN where there is no plan (workout brief §11). `None` between
    /// stations. **Recorded, never enforced** -- nothing disqualifies anybody on this.
    pub expectation: Option<Expectation>,
    pub stages: Vec<StageView>,
}

/// `POST` and `PUT /api/operator/templates`.
///
/// The whole template travels as one document, because that is how it is edited: a builder
/// screen holds the entire plan and saves it. `source` is deliberately absent -- the stored
/// row decides whether a template may be written, never the payload, or the read-only rule
/// on system templates would be bypassed by sending `"source": "COACH"`.
#[derive(Debug, Deserialize)]
pub struct SaveTemplateRequest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub category: TemplateCategory,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub difficulty: Option<String>,
    #[serde(default)]
    pub estimated_duration_minutes: Option<u32>,
    pub blocks: Vec<WorkoutBlock>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl From<SaveTemplateRequest> for WorkoutTemplate {
    fn from(r: SaveTemplateRequest) -> Self {
        let mut t = WorkoutTemplate::new(r.id, r.name, r.category);
        t.description = r.description;
        t.owner_id = r.owner_id;
        t.difficulty = r.difficulty;
        t.estimated_duration_minutes = r.estimated_duration_minutes;
        t.blocks = r.blocks;
        t
    }
}

/// `POST /api/operator/templates/{id}/duplicate` (workout brief §13, scenario A).
#[derive(Debug, Deserialize)]
pub struct DuplicateTemplateRequest {
    pub new_id: String,
    pub name: String,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /api/operator/class` (workout brief §15).
///
/// `finish_policy` is required for the same reason `PUT /config` requires it: letting an
/// omitted field fall into `NotConfigured` would create a class with no rule for when it
/// ends, silently.
#[derive(Debug, Deserialize)]
pub struct CreateClassRequest {
    pub template_id: String,
    pub session_id: String,
    pub name: String,
    #[serde(default = "training")]
    pub mode: SessionMode,
    #[serde(default)]
    pub coach_id: Option<String>,
    #[serde(default)]
    pub scheduled_at: Option<i64>,
    pub finish_policy: FinishPolicy,
    #[serde(default)]
    pub athletes: Vec<NewClassAthlete>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NewClassAthlete {
    pub athlete_id: String,
    pub display_name: String,
}

fn training() -> SessionMode {
    SessionMode::Training
}

/// `POST /api/checkin/entrants` (ADR 0010).
///
/// A competition takes entries from people the gym has never seen, so `member_id` is
/// optional -- its absence *is* the record that this was a walk-in, not a missing field.
#[derive(Debug, Deserialize)]
pub struct EnterRequest {
    pub display_name: String,
    /// `None` for a walk-in. When present it also becomes the athlete id, so a member keeps
    /// one identity across every class they enter.
    #[serde(default)]
    pub member_id: Option<String>,
    /// `None` takes the next free number. Competition bibs are printed in advance, so the
    /// door has to be able to name one.
    #[serde(default)]
    pub bib: Option<i64>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /api/checkin/entrants` -- the new roster line, plus the queue the door is working.
#[derive(Debug, Serialize)]
pub struct EnteredResponse {
    pub freshness: Freshness,
    pub athlete_id: String,
    pub pending: Vec<String>,
    pub athletes: Vec<CheckInAthlete>,
}

/// `GET /api/leaderboard` -- the running session's results, ranked where the finish rule
/// allows it (ADR 0010).
#[derive(Debug, Serialize)]
pub struct LeaderboardResponse {
    pub freshness: Freshness,
    pub results: SessionResults,
}
