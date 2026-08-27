//! The live snapshot the screens render. Serialisation only: every value here is derived
//! from the domain, never computed in the UI (CLAUDE.md 6, 29).

use domain::{AthleteState, AthleteStatus, Instant, Session, StationState};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct CourseStation {
    pub name: String,
    /// Matches the CSS mask class on the screen (`pg-<key>`).
    pub key: String,
    /// Planned work, e.g. "500 M". Display only: the hub cannot measure it (see AthleteView).
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
    /// Stations finished, drives the 8-segment course bar.
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
    pub readers_online: usize,
    /// Drives the mandatory freshness readout (ADR 0001 D5).
    pub last_event_age_ms: Option<i64>,
    pub course: Vec<CourseStation>,
    pub athletes: Vec<AthleteView>,
    pub in_class: usize,
    pub finished: usize,
    pub exceptions: usize,
}

pub fn view(
    bib: usize,
    a: &AthleteState,
    course: &[CourseStation],
    now: Instant,
) -> AthleteView {
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

pub fn snapshot(
    session: &Session,
    athletes: &[AthleteState],
    course: &[CourseStation],
    now: Instant,
    class_start: Instant,
    readers_online: usize,
    exceptions: usize,
) -> Snapshot {
    let last_event_age_ms = athletes
        .iter()
        .filter_map(|a| a.last_event_at)
        .max()
        .map(|t| now.since(t).millis());
    Snapshot {
        session_name: session.name.clone(),
        mode: format!("{:?}", session.mode).to_uppercase(),
        status: format!("{:?}", session.status).to_uppercase(),
        now: now.0,
        class_elapsed_ms: now.since(class_start).millis(),
        readers_online,
        last_event_age_ms,
        course: course.to_vec(),
        athletes: athletes
            .iter()
            .enumerate()
            .map(|(i, a)| view(i + 1, a, course, now))
            .collect(),
        in_class: athletes.len(),
        finished: athletes
            .iter()
            .filter(|a| a.status == AthleteStatus::Finished)
            .count(),
        exceptions,
    }
}
