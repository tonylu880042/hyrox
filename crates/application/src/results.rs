//! Session results (CLAUDE.md 22, `/result/{id}`).
//!
//! Rebuilt from the stored interpreted log on every request rather than kept anywhere, so a
//! result cannot disagree with the events behind it and a voided event is reflected the
//! moment it is voided (CLAUDE.md 20, 21). That also means results work for a session the
//! hub is no longer running.
//!
//! ## Ranking follows the finish rule, and only that rule
//!
//! A ranking needs an ordering, and every ordering worth the name asks who finished first.
//! Under `CourseComplete` that has an answer -- finishing is completing the course, timed at
//! the last station's exit (settled 2026-08-28) -- so competitors are placed by it.
//!
//! Under every other rule they are not. A class that ends when its time is up stops everyone
//! at the same moment having done different amounts of work, so ordering them by elapsed
//! time would rank people who did different things; and where no rule is configured, nobody
//! has finished anything. Those come back in bib order with no placings, and the payload
//! says which it is ([`SessionResults::ordering`]) so no client mistakes row order for a
//! placing.

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

/// How the rows are sorted, named in the payload so row order is never mistaken for a
/// placing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Ordering {
    /// Roster order. No placings.
    Bib,
    /// Finishers first, in the order they finished.
    FinishTime,
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
    /// Where they placed, from 1. `None` where the session is not ranked at all, and also
    /// for a competitor who has not finished -- which is not the same as last.
    pub place: Option<usize>,
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
    pub ordering: Ordering,
    pub rows: Vec<ResultRow>,
}

/// Results for the session the hub is running now, without touching the store.
///
/// The leaderboard screen polls this during a race, and everything it needs is already in
/// memory: the roster is rebuilt from the interpreted log at start-up and advanced by
/// ingestion since. Reading the store instead would add a round trip per poll and, worse,
/// would answer 404 for a session that is plainly on the floor.
pub fn live_results(state: &crate::live_session::LiveSession) -> SessionResults {
    let policy = state.config.finish_policy;
    let bib_of = |id: &str| state.bib_of(id).unwrap_or(0) as usize;
    let (ordering, rows) = rank(policy, &state.athletes, &bib_of);
    SessionResults {
        session_id: state.session.id.clone(),
        session_name: state.session.name.clone(),
        mode: state.session.mode,
        status: crate::session::status_name(state.session.status).to_string(),
        finish_policy: policy,
        course: course_view(state.config.course.as_ref()),
        ordering,
        rows,
    }
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
    let policy = config.as_ref().map(|c| c.finish_policy).unwrap_or_default();
    let bibs = store.athlete_bibs(session_id).await?;
    let bib_of = |id: &str| {
        bibs.iter().find(|(a, _)| a == id).map(|(_, b)| *b as usize).unwrap_or(0)
    };
    let (ordering, rows) = rank(policy, &athletes, &bib_of);

    Ok(Some(SessionResults {
        session_id: session.id.clone(),
        session_name: session.name.clone(),
        mode: session.mode,
        status: crate::session::status_name(session.status).to_string(),
        finish_policy: policy,
        course: course_view(config.as_ref().and_then(|c| c.course.as_ref())),
        ordering,
        rows,
    }))
}

/// Places the finishers, or leaves the roster in bib order.
///
/// Split out because the *decision* -- is this session rankable at all -- is the whole of
/// the product judgement here, and it is one line: only a course-completion rule says who
/// finished first.
fn rank(
    policy: FinishPolicy,
    athletes: &[AthleteState],
    bib_of: &dyn Fn(&str) -> usize,
) -> (Ordering, Vec<ResultRow>) {
    let mut rows: Vec<ResultRow> = athletes.iter().map(|a| row(bib_of(&a.athlete_id), a)).collect();

    if policy != FinishPolicy::CourseComplete {
        return (Ordering::Bib, rows);
    }

    // Finishers first by finish time, then everyone still out on the course in bib order.
    rows.sort_by_key(|r| (r.finished_at.is_none(), r.finished_at, r.bib));

    // Standard competition ranking: a shared time shares a place and the next place skips,
    // so the number beside a name is how many people were ahead of them, plus one.
    let mut previous: Option<i64> = None;
    let mut place = 0;
    for (index, row) in rows.iter_mut().enumerate() {
        let Some(at) = row.finished_at else { break };
        if previous != Some(at) {
            place = index + 1;
            previous = Some(at);
        }
        row.place = Some(place);
    }
    (Ordering::FinishTime, rows)
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
        place: None,
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
