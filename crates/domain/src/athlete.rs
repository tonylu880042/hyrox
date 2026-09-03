//! Athlete state and reader-event interpretation (CLAUDE.md 8, 10, 11, 13).

use crate::session::Session;
use crate::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AthleteStatus {
    Ready,
    Active,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum StationState {
    Outside,
    Inside,
}

/// Interpretation belongs to the hub, never to the ESP32 (CLAUDE.md 8).
///
/// `Deserialize` as well as `Serialize`: registering a reader is an operator action over
/// HTTP (ADR 0007), so the mode has to survive the trip in as well as out.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ReaderMode {
    Entry,
    Exit,
    Toggle,
    Checkpoint,
    Passage,
}

#[derive(Clone, Debug)]
pub struct ReaderBinding {
    pub station: String,
    pub mode: ReaderMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExceptionReason {
    SessionNotArmed,
    /// e.g. an EXIT while OUTSIDE, or a TOGGLE for a station the athlete is not inside.
    ImpossibleTransition,
    AlreadyFinished,
    /// The (device_id, reader_id) pair resolved to nothing in the reader registry
    /// (CLAUDE.md 8). Recorded rather than dropped so the read survives a venue
    /// mis-configuration and can be re-attributed once the mapping is fixed.
    UnknownReader,
    /// The tag is bound, but to somebody this session's roster does not contain -- a band
    /// from another class, or an athlete removed from the roster (ADR 0001 D4). The read is
    /// attributed to whoever holds the tag so an operator can see who to go and find.
    AthleteNotInSession,
}

/// What the hub decided a raw read means. The raw event is stored regardless (CLAUDE.md 19).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Interpreted {
    Entered {
        station: String,
        at: Instant,
        /// Gap since the previous station finished (CLAUDE.md 13). None for the first station.
        transition: Option<Duration>,
        /// True when this read also started the athlete's clock (CLAUDE.md 11).
        started_timing: bool,
    },
    Exited {
        station: String,
        at: Instant,
    },
    /// Recorded, never dropped. Surfaces in the operator's exception inbox (ADR D4).
    Exception {
        reason: ExceptionReason,
        at: Instant,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct StationRun {
    pub station: String,
    pub entered_at: Instant,
    pub exited_at: Option<Instant>,
    pub transition_from_prev: Option<Duration>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AthleteState {
    pub athlete_id: String,
    pub display_name: String,
    pub status: AthleteStatus,
    pub station_state: StationState,
    pub current_station: Option<String>,
    pub started_at: Option<Instant>,
    pub last_event_at: Option<Instant>,
    /// When the previous station was finished; the left-hand side of a transition (CLAUDE.md 13).
    pub last_exit_at: Option<Instant>,
    /// When this athlete stopped. Set by `finish`; without it every clock on the live
    /// screen would keep counting after the class ended.
    pub finished_at: Option<Instant>,
    pub runs: Vec<StationRun>,
}

impl AthleteState {
    pub fn ready(athlete_id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            athlete_id: athlete_id.into(),
            display_name: display_name.into(),
            status: AthleteStatus::Ready,
            station_state: StationState::Outside,
            current_station: None,
            started_at: None,
            last_event_at: None,
            last_exit_at: None,
            finished_at: None,
            runs: Vec::new(),
        }
    }

    /// Total time on course. A finished athlete's clock stops at their finish, so the
    /// screen shows a result rather than a number that keeps climbing.
    pub fn elapsed(&self, now: Instant) -> Option<Duration> {
        let end = self.finished_at.unwrap_or(now);
        self.started_at.map(|s| end.since(s))
    }

    /// Time inside the current station, or in transition if OUTSIDE. Frozen once finished,
    /// for the same reason as `elapsed`.
    pub fn current_leg(&self, now: Instant) -> Option<Duration> {
        let end = self.finished_at.unwrap_or(now);
        match self.station_state {
            StationState::Inside => self.runs.last().map(|r| end.since(r.entered_at)),
            StationState::Outside => self.last_exit_at.map(|e| end.since(e)),
        }
    }
}

/// Decide what a reader read means. Pure: takes `&AthleteState`, mutates nothing.
///
/// `at` is always `detected_at`, never arrival time (CLAUDE.md 11, 17). Never fails --
/// an uninterpretable read becomes an Exception so the event is preserved (CLAUDE.md 31).
pub fn decide(
    state: &AthleteState,
    binding: &ReaderBinding,
    at: Instant,
    session: &Session,
) -> Interpreted {
    let exception = |reason| Interpreted::Exception { reason, at };

    if !session.accepts_events() {
        return exception(ExceptionReason::SessionNotArmed);
    }
    if state.status == AthleteStatus::Finished {
        return exception(ExceptionReason::AlreadyFinished);
    }

    // The first valid read after ARMED starts this athlete's clock (CLAUDE.md 11).
    let started_timing = state.status == AthleteStatus::Ready;

    let entering = || {
        if state.station_state == StationState::Inside {
            return exception(ExceptionReason::ImpossibleTransition);
        }
        Interpreted::Entered {
            station: binding.station.clone(),
            at,
            // No transition before the first station: nothing to subtract from.
            transition: state.last_exit_at.map(|prev| at.since(prev)),
            started_timing,
        }
    };
    let exiting = || {
        let inside_this = state.station_state == StationState::Inside
            && state.current_station.as_deref() == Some(binding.station.as_str());
        if !inside_this {
            return exception(ExceptionReason::ImpossibleTransition);
        }
        Interpreted::Exited {
            station: binding.station.clone(),
            at,
        }
    };

    match binding.mode {
        ReaderMode::Entry => entering(),
        ReaderMode::Exit => exiting(),
        // Uses athlete state, never scan parity (CLAUDE.md 10.2).
        ReaderMode::Toggle => match state.station_state {
            StationState::Outside => entering(),
            StationState::Inside => exiting(),
        },
        // Passage/checkpoint readers mark a crossing without owning station occupancy.
        // Modelling them is Milestone 4 work; for now they are recorded, not interpreted.
        ReaderMode::Checkpoint | ReaderMode::Passage => {
            exception(ExceptionReason::ImpossibleTransition)
        }
    }
}

/// Fold one interpreted event into the athlete's state.
///
/// This is the ONLY way state advances, so replaying the stored interpreted events in
/// `detected_at` order rebuilds state exactly (CLAUDE.md 21). Rebuilding must go through
/// interpreted events rather than re-deciding from raw ones, because operator corrections
/// live at the interpreted layer and re-deciding would silently discard them (CLAUDE.md 20).
pub fn apply(state: &mut AthleteState, event: &Interpreted) {
    match event {
        Interpreted::Entered {
            station,
            at,
            transition,
            started_timing,
        } => {
            if *started_timing {
                state.status = AthleteStatus::Active;
                state.started_at = Some(*at);
            }
            state.station_state = StationState::Inside;
            state.current_station = Some(station.clone());
            state.runs.push(StationRun {
                station: station.clone(),
                entered_at: *at,
                exited_at: None,
                transition_from_prev: *transition,
            });
            state.last_event_at = Some(*at);
        }
        Interpreted::Exited { station, at } => {
            if let Some(run) = state.runs.iter_mut().rev().find(|r| &r.station == station) {
                run.exited_at = Some(*at);
            }
            state.station_state = StationState::Outside;
            state.current_station = None;
            state.last_exit_at = Some(*at);
            state.last_event_at = Some(*at);
        }
        // Recorded for the operator's exception inbox (ADR 0001 D4); changes no state.
        Interpreted::Exception { .. } => {}
    }
}

/// Live path: decide, then apply. Replay uses `apply` alone.
pub fn interpret(
    state: &mut AthleteState,
    binding: &ReaderBinding,
    at: Instant,
    session: &Session,
) -> Interpreted {
    let event = decide(state, binding, at, session);
    apply(state, &event);
    event
}

/// Rebuild an athlete from stored interpreted events. Order must be `detected_at` ascending;
/// voided events must already be filtered out by the caller.
pub fn replay<'a>(
    athlete_id: impl Into<String>,
    display_name: impl Into<String>,
    events: impl IntoIterator<Item = &'a Interpreted>,
) -> AthleteState {
    let mut state = AthleteState::ready(athlete_id, display_name);
    for e in events {
        apply(&mut state, e);
    }
    state
}
