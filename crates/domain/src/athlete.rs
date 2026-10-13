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
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReaderMode {
    Entry,
    Exit,
    Toggle,
    Checkpoint,
    Passage,
    /// One mat that is both the start line and the exit of the station it is registered
    /// against (ADR 0016). What a read means depends on the athlete, never on the mat.
    StartFinish,
    /// One reader whose antenna covers the whole station area (ADR 0017). Arriving enters,
    /// departing exits; it needs the edge to report departures.
    Zone,
}

/// Whether the edge saw a tag appear, or reported it gone after `absent_timeout`
/// (ADR 0017). Every reader reports arrivals; only some report departures.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Presence {
    #[default]
    Arrived,
    Departed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReaderBinding {
    pub station: String,
    pub mode: ReaderMode,
    /// A ZONE visit shorter than this is a pass-by, and a departure followed by a return
    /// within it is the same visit (ADR 0017). Per reader: an erg hides an athlete from an
    /// antenna differently than a sled lane does.
    #[serde(default = "ReaderBinding::default_min_visit")]
    pub min_visit: Duration,
}

impl ReaderBinding {
    /// Far shorter than any station's work, far longer than RF flicker (ADR 0017).
    pub const DEFAULT_MIN_VISIT: Duration = Duration(15_000);

    pub fn new(station: impl Into<String>, mode: ReaderMode) -> Self {
        Self {
            station: station.into(),
            mode,
            min_visit: Self::DEFAULT_MIN_VISIT,
        }
    }

    pub fn with_min_visit(mut self, min_visit: Duration) -> Self {
        self.min_visit = min_visit;
        self
    }

    fn default_min_visit() -> Duration {
        Self::DEFAULT_MIN_VISIT
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The start mat started this athlete's clock (ADR 0016). Also the left-hand side of
    /// the first run, exactly as an exit is for every later one.
    Started {
        at: Instant,
    },
    /// An active athlete crossed the start/finish line without it closing a station
    /// (ADR 0017). The route runs over it, so most crossings mean nothing; the first one
    /// after the last station is the finish under `FinishPolicy::FinishLine`.
    Crossed {
        at: Instant,
    },
    /// A ZONE visit too short to be one -- the route brushed the zone (ADR 0017). Undoes the
    /// entry it follows; the entry stays in the log, auditable.
    VisitWithdrawn {
        station: String,
        at: Instant,
    },
    /// The athlete reappeared in a ZONE shortly after its departure: they never left, the
    /// antenna lost them (ADR 0017). Reopens the visit.
    ExitWithdrawn {
        station: String,
        at: Instant,
    },
    /// A read that means nothing where it was taken: a finished athlete on the mat, or a
    /// departure from a reader that does not track presence (ADR 0016, 0017). Kept so the
    /// read is accounted for, but it is not an exception.
    Passed {
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
    /// The athlete turned up at another station while this one was still open: they left,
    /// but no reader saw when (ADR 0016). `exited_at` stays `None` rather than inventing a
    /// time; the step still counts as done.
    pub exit_missed: bool,
}

impl StationRun {
    /// Done with this station, whether or not the exit was read.
    pub fn is_complete(&self) -> bool {
        self.exited_at.is_some() || self.exit_missed
    }
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
    /// Every crossing of the finish line after the start (ADR 0017). Kept apart from
    /// `last_event_at`: a station departure is published after the athlete has gone, so it
    /// can reach the hub behind a crossing that happened later.
    pub line_crossings: Vec<Instant>,
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
            line_crossings: Vec::new(),
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
    decide_read(state, binding, Presence::Arrived, at, session)
}

/// [`decide`] for a read that may be a departure (ADR 0017).
pub fn decide_read(
    state: &AthleteState,
    binding: &ReaderBinding,
    presence: Presence,
    at: Instant,
    session: &Session,
) -> Interpreted {
    let exception = |reason| Interpreted::Exception { reason, at };

    if !session.accepts_events() {
        return exception(ExceptionReason::SessionNotArmed);
    }
    let inside = |station: &str| {
        state.station_state == StationState::Inside
            && state.current_station.as_deref() == Some(station)
    };
    let passed = || Interpreted::Passed {
        station: binding.station.clone(),
        at,
    };
    if presence == Presence::Departed {
        // Only a zone gives a departure meaning; anywhere else it is recorded (ADR 0017).
        if binding.mode != ReaderMode::Zone
            || state.status == AthleteStatus::Finished
            || !inside(&binding.station)
        {
            return passed();
        }
        let entered = state.runs.last().map(|r| r.entered_at).unwrap_or(at);
        return if at.since(entered) < binding.min_visit {
            Interpreted::VisitWithdrawn {
                station: binding.station.clone(),
                at,
            }
        } else {
            Interpreted::Exited {
                station: binding.station.clone(),
                at,
            }
        };
    }
    if binding.mode == ReaderMode::StartFinish {
        // The route runs over this mat, so most reads of it mean nothing (ADR 0016).
        return match state.status {
            AthleteStatus::Ready => Interpreted::Started { at },
            AthleteStatus::Active if inside(&binding.station) => Interpreted::Exited {
                station: binding.station.clone(),
                at,
            },
            AthleteStatus::Active => Interpreted::Crossed { at },
            AthleteStatus::Finished => passed(),
        };
    }
    if state.status == AthleteStatus::Finished {
        return exception(ExceptionReason::AlreadyFinished);
    }

    let entering = || {
        let entered = Interpreted::Entered {
            station: binding.station.clone(),
            at,
            transition: None,
            started_timing: false,
        };
        // Same derivation as replay, so the live answer and the rebuilt one cannot differ.
        rederive(state, entered).unwrap_or_else(|| exception(ExceptionReason::ImpossibleTransition))
    };
    let exiting = || {
        if !inside(&binding.station) {
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
        // Inside another station, a toggle here means the exit there was missed (ADR 0016).
        ReaderMode::Toggle if inside(&binding.station) => exiting(),
        ReaderMode::Toggle => entering(),
        // Passage/checkpoint readers mark a crossing without owning station occupancy.
        // Modelling them is Milestone 4 work; for now they are recorded, not interpreted.
        ReaderMode::Checkpoint | ReaderMode::Passage => {
            exception(ExceptionReason::ImpossibleTransition)
        }
        // Still in the zone: the same presence, re-reported.
        ReaderMode::Zone if inside(&binding.station) => passed(),
        ReaderMode::Zone => {
            let reopen = Interpreted::ExitWithdrawn {
                station: binding.station.clone(),
                at,
            };
            match state.runs.last() {
                Some(run)
                    if run
                        .exited_at
                        .is_some_and(|out| at.since(out) < binding.min_visit)
                        && rederive(state, reopen.clone()).is_some() =>
                {
                    reopen
                }
                _ => entering(),
            }
        }
        ReaderMode::StartFinish => unreachable!("handled above"),
    }
}

/// The derived fields of an event, recomputed from the history that now precedes it.
///
/// `transition` and `started_timing` are properties of an event's position in the timeline,
/// so a correction earlier in the log changes them (CLAUDE.md 20). Live interpretation goes
/// through here too, which is what keeps a replay identical to the live run (CLAUDE.md 21).
/// `None` means the event no longer makes sense where it sits and must not be projected; it
/// stays in the log, auditable.
pub fn rederive(state: &AthleteState, event: Interpreted) -> Option<Interpreted> {
    match event {
        // A second entry into the station the athlete is already in is a re-read.
        Interpreted::Entered { ref station, .. }
            if state.station_state == StationState::Inside
                && state.current_station.as_deref() == Some(station.as_str()) =>
        {
            None
        }
        Interpreted::Entered { station, at, .. } => Some(Interpreted::Entered {
            // Inside another station the previous exit was missed: nothing to subtract from
            // (ADR 0016). No transition before the first station either.
            transition: match state.station_state {
                StationState::Inside => None,
                StationState::Outside => state.last_exit_at.map(|prev| at.since(prev)),
            },
            // The first valid read after ARMED starts this athlete's clock (CLAUDE.md 11).
            started_timing: state.status == AthleteStatus::Ready,
            station,
            at,
        }),
        // Only the first event can start the clock.
        Interpreted::Started { .. } if state.status != AthleteStatus::Ready => None,
        // A withdrawal only fits the history it was decided against (ADR 0017).
        Interpreted::VisitWithdrawn { ref station, .. }
            if !(state.station_state == StationState::Inside
                && state.current_station.as_deref() == Some(station.as_str())) =>
        {
            None
        }
        Interpreted::ExitWithdrawn { ref station, .. }
            if !(state.station_state == StationState::Outside
                && state
                    .runs
                    .last()
                    .is_some_and(|r| &r.station == station && r.exited_at.is_some())) =>
        {
            None
        }
        other => Some(other),
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
            if state.station_state == StationState::Inside {
                if let Some(open) = state.runs.last_mut() {
                    open.exit_missed = true;
                }
            }
            state.station_state = StationState::Inside;
            state.current_station = Some(station.clone());
            state.runs.push(StationRun {
                station: station.clone(),
                entered_at: *at,
                exited_at: None,
                transition_from_prev: *transition,
                exit_missed: false,
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
        Interpreted::Started { at } => {
            state.status = AthleteStatus::Active;
            state.started_at = Some(*at);
            state.last_exit_at = Some(*at);
            state.last_event_at = Some(*at);
        }
        Interpreted::Crossed { at } => state.line_crossings.push(*at),
        Interpreted::VisitWithdrawn { at, .. } => {
            state.runs.pop();
            match state.runs.last_mut() {
                // The withdrawn entry was what marked this exit missed; undo that too.
                Some(open) if open.exit_missed => {
                    open.exit_missed = false;
                    state.current_station = Some(open.station.clone());
                    state.station_state = StationState::Inside;
                }
                _ => {
                    state.current_station = None;
                    state.station_state = StationState::Outside;
                }
            }
            // A pass-by that started the clock did not start the race.
            if state.runs.is_empty() && state.last_exit_at.is_none() {
                state.status = AthleteStatus::Ready;
                state.started_at = None;
            }
            state.last_event_at = Some(*at);
        }
        Interpreted::ExitWithdrawn { at, .. } => {
            if let Some(run) = state.runs.last_mut() {
                run.exited_at = None;
                // The exit before this visit, recovered from the visit's own run time.
                state.last_exit_at = run
                    .transition_from_prev
                    .map(|d| Instant(run.entered_at.0 - d.millis()));
                state.current_station = Some(run.station.clone());
            }
            state.station_state = StationState::Inside;
            state.last_event_at = Some(*at);
        }
        // Recorded for the operator's exception inbox (ADR 0001 D4); changes no state.
        // A meaningless read changes none either (ADR 0016).
        Interpreted::Exception { .. } | Interpreted::Passed { .. } => {}
    }
}

/// Live path: decide, then apply. Replay uses `apply` alone.
pub fn interpret(
    state: &mut AthleteState,
    binding: &ReaderBinding,
    at: Instant,
    session: &Session,
) -> Interpreted {
    interpret_read(state, binding, Presence::Arrived, at, session)
}

/// [`interpret`] for a read that may be a departure (ADR 0017).
pub fn interpret_read(
    state: &mut AthleteState,
    binding: &ReaderBinding,
    presence: Presence,
    at: Instant,
    session: &Session,
) -> Interpreted {
    let event = decide_read(state, binding, presence, at, session);
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
