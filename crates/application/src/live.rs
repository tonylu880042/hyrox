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
    AthleteState, AthleteStatus, Course, Instant, Session, StationState, StationTarget,
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
    }
}

pub fn snapshot(state: &LiveSession, now: Instant) -> Snapshot {
    let course = course_view(state.config.course.as_ref());
    let last_event_age_ms = state
        .athletes
        .iter()
        .filter_map(|a| a.last_event_at)
        .max()
        .map(|t| now.since(t).millis());
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
            .map(|(i, a)| view(i + 1, a, &course, now))
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
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect()
}

fn target_label(target: &StationTarget) -> String {
    match target {
        StationTarget::Distance { meters } => format!("{meters} M"),
        StationTarget::Repetitions { count } => format!("{count} REPS"),
        StationTarget::Duration { duration } => {
            let total = duration.millis().max(0) / 1000;
            format!("{}:{:02} MIN", total / 60, total % 60)
        }
    }
}
