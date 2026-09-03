//! The live read model (CLAUDE.md 22, 23; ADR 0001 D5).
//!
//! One derived, serialisable view of a running session, shared by every screen: `/live` now,
//! `/coach` and the competition screen later. It lives here rather than in the HTTP binary
//! because what a coach is shown -- which station, which split, how stale the data is -- is
//! a product decision, and a second screen must not have to re-derive it (CLAUDE.md 6, 29).
//!
//! Nothing is computed here that the domain has not already decided. Every field is a
//! projection of `AthleteState`, the session, and the course.

use crate::live_session::LiveSession;
use crate::session::status_name;
use domain::{
    AthleteState, AthleteStatus, Course, DeviceWarning, Instant, ReaderMode, Session, StationRun,
    StationState, StationTarget,
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct CourseStation {
    pub name: String,
    /// A stable slug for the station, which the screens use to pick an icon.
    pub key: String,
    /// Planned work, e.g. "500 M". Display only: the hub learns entry and exit times from
    /// RFID and nothing about what happened inside the station (ADR 0001, 2026-08-27).
    pub plan: String,
}

/// One completed or in-progress leg, as a coach reads it (CLAUDE.md 23).
///
/// `work_ms` is the Workout Split; `transition_ms` is the ROX / Transition Time
/// (CLAUDE.md 13). Whether a leg is a "run" is a question about the course's station names,
/// not about this type -- the hub learns entry and exit times and nothing else (ADR 0001,
/// 2026-08-27 addendum), so it publishes every leg and lets the plan say which is which.
#[derive(Clone, Debug, Serialize)]
pub struct SplitView {
    pub station: String,
    pub entered_at: i64,
    pub exited_at: Option<i64>,
    pub work_ms: Option<i64>,
    pub transition_ms: Option<i64>,
}

/// One configured reader and how long ago its device was last heard from (ADR 0001 D5).
#[derive(Clone, Debug, Serialize)]
pub struct ReaderView {
    pub device_id: String,
    pub reader_id: String,
    pub station: String,
    pub zone: Option<String>,
    pub mode: ReaderMode,
    /// `None` means the hub has heard nothing from this device since it started. That is
    /// not the same as zero, and a screen must not draw it as fresh.
    pub last_seen_age_ms: Option<i64>,
    /// The device's own journal warning, if it has published one (CLAUDE.md 18).
    pub warning: Option<DeviceWarning>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AthleteView {
    pub bib: usize,
    pub name: String,
    pub status: AthleteStatus,
    pub station_state: StationState,
    pub station: Option<String>,
    pub station_key: Option<String>,
    pub station_index: Option<usize>,
    pub next_station: Option<String>,
    /// Stations finished, drives the course bar.
    pub completed: usize,
    /// Time in the current station, or in transition when OUTSIDE (CLAUDE.md 13, 23).
    pub leg_ms: Option<i64>,
    pub elapsed_ms: Option<i64>,
    pub plan: Option<String>,
    /// What the last event was, e.g. `"EXITED SKIERG"` (CLAUDE.md 23, "Last Event").
    /// Derived from the athlete's own runs, so it says what the log says.
    pub last_event: Option<String>,
    pub last_event_at: Option<i64>,
    /// How stale this athlete's own line is (ADR 0001 D5). The header carries the session's
    /// freshness; this one answers "and is *this* person's row moving?".
    pub last_event_age_ms: Option<i64>,
    /// Every leg so far: Workout Split and Transition / ROX per station (CLAUDE.md 23).
    pub splits: Vec<SplitView>,
    /// The most recent completed transition, which is the number a coach watches between
    /// stations (CLAUDE.md 13).
    pub last_transition_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub session_name: String,
    pub mode: String,
    pub status: String,
    pub now: i64,
    pub class_elapsed_ms: i64,
    /// Readers the hub can attribute a read to. A reader missing from here is a read that
    /// would become an UNKNOWN_READER exception.
    pub readers_online: usize,
    /// Drives the mandatory freshness readout (ADR 0001 D5): a still screen must be
    /// distinguishable from a dead link.
    pub last_event_age_ms: Option<i64>,
    pub course: Vec<CourseStation>,
    pub athletes: Vec<AthleteView>,
    pub in_class: usize,
    pub finished: usize,
    /// Exception inbox badge (ADR 0001 D4).
    pub exceptions: usize,
    /// Tags read but not yet claimed on `/checkin` (ADR 0001 D3). Not exceptions.
    pub pending_tags: usize,
}

/// The course as the screens want it: names, icon keys and a printable plan.
pub fn course_view(course: Option<&Course>) -> Vec<CourseStation> {
    course
        .map(|c| {
            c.steps
                .iter()
                .map(|s| CourseStation {
                    key: slug(&s.station),
                    name: s.station.clone(),
                    plan: s.target.as_ref().map(target_label).unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn view(bib: usize, a: &AthleteState, course: &[CourseStation], now: Instant) -> AthleteView {
    let station_index = a
        .current_station
        .as_ref()
        .and_then(|s| course.iter().position(|c| &c.name == s));
    let completed = a.runs.iter().filter(|r| r.exited_at.is_some()).count();
    let next_station = if a.station_state == StationState::Outside {
        course.get(completed).map(|c| c.name.clone())
    } else {
        None
    };
    let splits: Vec<SplitView> = a.runs.iter().map(split_view).collect();
    AthleteView {
        bib,
        name: a.display_name.clone(),
        status: a.status,
        station_state: a.station_state,
        station: a.current_station.clone(),
        station_key: station_index.map(|i| course[i].key.clone()),
        station_index,
        next_station,
        completed,
        leg_ms: a.current_leg(now).map(|d| d.millis()),
        elapsed_ms: a.elapsed(now).map(|d| d.millis()),
        plan: station_index.map(|i| course[i].plan.clone()),
        last_event: last_event_label(a),
        last_event_at: a.last_event_at.map(|t| t.0),
        last_event_age_ms: a.last_event_at.map(|t| now.since(t).millis()),
        last_transition_ms: splits.iter().rev().find_map(|s| s.transition_ms),
        splits,
    }
}

fn split_view(run: &StationRun) -> SplitView {
    SplitView {
        station: run.station.clone(),
        entered_at: run.entered_at.0,
        exited_at: run.exited_at.map(|t| t.0),
        work_ms: run.exited_at.map(|e| e.since(run.entered_at).millis()),
        transition_ms: run.transition_from_prev.map(|d| d.millis()),
    }
}

/// The last thing that happened to this athlete, in the words the log would use.
///
/// Read off the runs rather than stored, because `AthleteState` is itself a fold of the
/// interpreted events (CLAUDE.md 21) -- keeping a separate copy of the last one would be a
/// second thing to keep in step. Exceptions do not appear: they change no state and are the
/// inbox's business (ADR 0001 D4), not the athlete row's.
fn last_event_label(a: &AthleteState) -> Option<String> {
    let last = a.runs.last()?;
    Some(match last.exited_at {
        Some(_) => format!("EXITED {}", last.station),
        None => format!("ENTERED {}", last.station),
    })
}

/// One roster line as `/checkin` reads it (ADR 0001 D3).
#[derive(Clone, Debug, Serialize)]
pub struct CheckInAthlete {
    /// Roster position, numbered exactly as the live screen numbers it.
    pub bib: usize,
    pub athlete_id: String,
    pub name: String,
    /// The band in force for this athlete **in this session**. `None` is the work: someone
    /// on the roster with no band yet.
    pub tag_id: Option<String>,
}

/// What the narrow write surface needs to do its one job: which bands are waiting, and who
/// has not got one (ADR 0001 D3).
#[derive(Clone, Debug, Serialize)]
pub struct CheckInView {
    /// Tags a reader has seen that belong to nobody, oldest first. Not errors -- they are
    /// the to-do list, which is why they never reach the exception inbox (D4).
    pub pending: Vec<String>,
    pub athletes: Vec<CheckInAthlete>,
}

/// The check-in queue and the roster beside it.
///
/// Derived here rather than in an HTTP handler for the same reason [`snapshot`] is: which
/// band counts as "in force for this athlete" is a question about the binding ledger, and
/// a second check-in screen must not have to answer it again (CLAUDE.md 29).
pub fn checkin_view(state: &LiveSession) -> CheckInView {
    CheckInView {
        pending: state.pending_tags().iter().map(|t| t.to_string()).collect(),
        athletes: state
            .athletes
            .iter()
            .enumerate()
            .map(|(i, a)| CheckInAthlete {
                // The number actually on the vest, which the door may have assigned
                // (ADR 0010). Roster position only as a fallback, for a session seeded
                // before bibs were assignable.
                bib: state.bib_of(&a.athlete_id).unwrap_or(i as i64 + 1) as usize,
                athlete_id: a.athlete_id.clone(),
                name: a.display_name.clone(),
                // Scoped to this session on purpose: a band bound in another class is on
                // somebody else's wrist, and drawing it here would claim a binding this
                // session does not hold.
                tag_id: state
                    .bindings
                    .tag_for_athlete(&state.session.id, &a.athlete_id)
                    .map(|t| t.to_string()),
            })
            .collect(),
    }
}

/// Every configured reader with its device's freshness (ADR 0001 D5).
///
/// Served on its own rather than folded into [`snapshot`]: reader health changes on the
/// scale of a status message, not of a race, and the live screen pushes a snapshot four
/// times a second (CLAUDE.md 23).
pub fn reader_views(state: &LiveSession, now: Instant) -> Vec<ReaderView> {
    state
        .readers
        .iter()
        .map(|r| {
            let health = state.device(&r.key.device_id);
            ReaderView {
                device_id: r.key.device_id.to_string(),
                reader_id: r.key.reader_id.to_string(),
                station: r.station.clone(),
                zone: r.zone.clone(),
                mode: r.mode,
                last_seen_age_ms: health.map(|h| now.since(h.last_seen).millis()),
                warning: health.and_then(|h| h.warning()),
            }
        })
        .collect()
}

/// How long ago the newest interpreted event in this session was **detected**, or `None`
/// when nothing has happened yet (ADR 0001 D5).
///
/// The mandatory freshness reading, defined once. Every screen shows it, so a still screen
/// and a dead link are never the same picture -- and every screen must be showing the same
/// number, which it cannot be if each one derives its own.
///
/// `None` is not zero and must not be drawn as fresh: it means no event exists to be stale.
pub fn last_event_age_ms(state: &LiveSession, now: Instant) -> Option<i64> {
    state
        .athletes
        .iter()
        .filter_map(|a| a.last_event_at)
        .max()
        .map(|t| now.since(t).millis())
}

pub fn snapshot(state: &LiveSession, now: Instant) -> Snapshot {
    let course = course_view(state.config.course.as_ref());
    let last_event_age_ms = last_event_age_ms(state, now);
    Snapshot {
        session_name: state.session.name.clone(),
        mode: mode_name(&state.session).to_string(),
        status: status_name(state.session.status).to_string(),
        now: now.0,
        class_elapsed_ms: state.class_elapsed(now).millis(),
        readers_online: state.readers.len(),
        last_event_age_ms,
        athletes: state
            .athletes
            .iter()
            .enumerate()
            .map(|(i, a)| {
                view(
                    state.bib_of(&a.athlete_id).unwrap_or(i as i64 + 1) as usize,
                    a,
                    &course,
                    now,
                )
            })
            .collect(),
        in_class: state.athletes.len(),
        finished: state
            .athletes
            .iter()
            .filter(|a| a.status == AthleteStatus::Finished)
            .count(),
        exceptions: state.exception_count,
        pending_tags: state.pending_tags().len(),
        course,
    }
}

fn mode_name(session: &Session) -> &'static str {
    match session.mode {
        domain::SessionMode::Competition => "COMPETITION",
        domain::SessionMode::Training => "TRAINING",
    }
}

/// "SLED PUSH" -> "sled_push". Punctuation and spaces collapse to `_` so a station name can
/// be renamed in configuration without breaking a screen's asset lookup.
fn slug(station: &str) -> String {
    station
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// The plan a screen shows beside a station, e.g. "800 M".
///
/// The trailing token is a unit the screens translate at the point of display
/// (`PLAN_UNIT` in `design/live/build_screens.py`, `targetUnit` in `workout.html`). Adding a
/// new suffix here means adding it there, or a coach sees an English one.
fn target_label(target: &StationTarget) -> String {
    match target {
        StationTarget::Distance { meters } => format!("{meters} M"),
        StationTarget::Repetitions { count } => format!("{count} REPS"),
        StationTarget::Calories { count } => format!("{count} CAL"),
        StationTarget::Duration { duration } => {
            let total = duration.millis().max(0) / 1000;
            format!("{}:{:02} MIN", total / 60, total % 60)
        }
    }
}
