//! Session results (CLAUDE.md 22, `/result/{id}`).
//!
//! Rebuilt from the stored interpreted log on every request rather than kept anywhere, so a
//! result cannot disagree with the events behind it and a voided event is reflected the
//! moment it is voided (CLAUDE.md 20, 21). That also means results work for a session the
//! hub is no longer running.
//!
//! ## There is no ranking here, on purpose
//!
//! Ranking needs an ordering, and every ordering worth the name asks who finished first.
//! The competition finish rule is undecided (CLAUDE.md 12, 28), and the training rule that
//! *is* decided -- a class ends when its time is up -- deliberately expects most athletes
//! not to complete the course, so ordering them by elapsed time would rank people who did
//! different amounts of work. Rows therefore come back in bib order and say so
//! ([`SessionResults::ordering`]). Elapsed time, splits, transitions and stations completed
//! are all genuinely derived and are all here; the ordering is the part that would have to
//! be invented.

use crate::live::{course_view, CourseStation};
use crate::ports::HubStore;
use domain::{AthleteState, AthleteStatus, Duration, FinishPolicy, Instant, SessionMode};
use serde::Serialize;

/// One leg: the station, and how long it took to get to it and to do it.
#[derive(Clone, Debug, Serialize)]
pub struct SplitRow {
    pub station: String,
    pub entered_at: i64,
    /// `None` for a station the athlete was still inside when the class ended. No reader
    /// reported them leaving, and inventing an exit would fabricate a split (CLAUDE.md 19).
    pub exited_at: Option<i64>,
    /// Time inside the station -- the Workout Split of CLAUDE.md 23.
    pub work_ms: Option<i64>,
    /// Gap since the previous station finished: ROX Zone time in competition, Transition
    /// Time in training (CLAUDE.md 13). `None` for the first station.
    pub transition_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResultRow {
    pub bib: usize,
    pub athlete_id: String,
    pub name: String,
    pub status: AthleteStatus,
    pub started_at: Option<i64>,
    /// Set only when a finish rule marked them finished. `None` is not "did not finish" --
    /// with an undecided rule it means the question has no answer yet (CLAUDE.md 12).
    pub finished_at: Option<i64>,
    pub elapsed_ms: Option<i64>,
    /// Stations left, not stations entered: a station still in progress is not completed.
    pub stations_completed: usize,
    pub splits: Vec<SplitRow>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionResults {
    pub session_id: String,
    pub session_name: String,
    pub mode: SessionMode,
    pub status: String,
    /// The rule the session was armed under (ADR 0004). Published so a reader of the results
    /// can see what "finished" meant here -- including that it meant nothing.
    pub finish_policy: FinishPolicy,
    pub course: Vec<CourseStation>,
    /// Always `"BIB"`. Named in the payload so no client mistakes row order for a placing.
    pub ordering: &'static str,
    pub rows: Vec<ResultRow>,
}

/// Results for one stored session, or `None` if the store has no such session.
pub async fn results<S: HubStore>(
    store: &S,
    session_id: &str,
) -> Result<Option<SessionResults>, S::Error> {
    let Some(session) = store.session(session_id).await? else {
        return Ok(None);
    };
    let config = store.session_config(session_id).await?;
    let athletes = store.rebuild_athletes(session_id).await?;

    Ok(Some(SessionResults {
        session_id: session.id.clone(),
        session_name: session.name.clone(),
        mode: session.mode,
        status: crate::session::status_name(session.status).to_string(),
        finish_policy: config.as_ref().map(|c| c.finish_policy).unwrap_or_default(),
        course: course_view(config.as_ref().and_then(|c| c.course.as_ref())),
        ordering: "BIB",
        // Bib is the roster position, exactly as the live screen numbers them; the store
        // returns the roster in bib order.
        rows: athletes.iter().enumerate().map(|(i, a)| row(i + 1, a)).collect(),
    }))
}

fn row(bib: usize, a: &AthleteState) -> ResultRow {
    ResultRow {
        bib,
        athlete_id: a.athlete_id.clone(),
        name: a.display_name.clone(),
        status: a.status,
        started_at: a.started_at.map(|t| t.0),
        finished_at: a.finished_at.map(|t| t.0),
        // `elapsed` needs a "now" for an athlete still running; a finished one ignores it.
        // Results of a live class are a snapshot, so their own last event is the honest
        // right-hand side rather than the wall clock, which would keep climbing between
        // two requests that saw the same events.
        elapsed_ms: a
            .elapsed(a.last_event_at.unwrap_or(Instant(0)))
            .map(Duration::millis),
        stations_completed: a.runs.iter().filter(|r| r.exited_at.is_some()).count(),
        splits: a
            .runs
            .iter()
            .map(|r| SplitRow {
                station: r.station.clone(),
                entered_at: r.entered_at.0,
                exited_at: r.exited_at.map(|t| t.0),
                work_ms: r.exited_at.map(|e| e.since(r.entered_at).millis()),
                transition_ms: r.transition_from_prev.map(Duration::millis),
            })
            .collect(),
    }
}
