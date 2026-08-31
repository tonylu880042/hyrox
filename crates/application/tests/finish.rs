//! Applying the finish policy (CLAUDE.md 12; docs/open-issues.md).

mod support;

use application::{apply_finish_policy, end_class, LiveSession, OperatorCommand, OperatorError};
use domain::{
    AthleteState, AthleteStatus, Duration, FinishPolicy, Instant, Session, SessionConfig,
    SessionMode, SessionStatus,
};
use support::FakeStore;

const START: Instant = Instant(1_000_000);
const LIMIT: Duration = Duration(60 * 60 * 1000); // a one-hour class

fn class(mode: SessionMode, policy: FinishPolicy) -> LiveSession {
    let mut session = Session::new_draft("s1", "THU 19:00", mode);
    session.mark_ready().expect("arm");
    session.start().expect("arm");
    let mut running = AthleteState::ready("a1", "RUNNING");
    running.status = AthleteStatus::Active;
    running.started_at = Some(START);
    LiveSession::new(session, SessionConfig::new("s1").with_finish_policy(policy), START)
        .with_athletes(vec![running, AthleteState::ready("a2", "NEVER SCANNED IN")])
}

#[tokio::test]
async fn a_class_ends_when_its_time_is_up() {
    let mut state = class(SessionMode::Training, FinishPolicy::ClassDuration { limit: LIMIT });

    let finished = apply_finish_policy(&mut state, Instant(START.0 + LIMIT.millis()));

    assert_eq!(finished, vec!["a1".to_string()]);
    assert_eq!(state.athlete("a1").unwrap().status, AthleteStatus::Finished);
}

#[tokio::test]
async fn before_the_limit_nobody_is_finished() {
    let mut state = class(SessionMode::Training, FinishPolicy::ClassDuration { limit: LIMIT });

    let finished = apply_finish_policy(&mut state, Instant(START.0 + LIMIT.millis() - 1));

    assert!(finished.is_empty());
    assert_eq!(state.athlete("a1").unwrap().status, AthleteStatus::Active);
}

#[tokio::test]
async fn an_athlete_who_never_scanned_in_does_not_finish() {
    let mut state = class(SessionMode::Training, FinishPolicy::ClassDuration { limit: LIMIT });

    apply_finish_policy(&mut state, Instant(START.0 + LIMIT.millis()));

    // The class ended; this one simply did not take part (docs/open-issues.md, 2026-08-27).
    assert_eq!(state.athlete("a2").unwrap().status, AthleteStatus::Ready);
}

#[tokio::test]
async fn applying_the_policy_twice_finishes_nobody_new() {
    let mut state = class(SessionMode::Training, FinishPolicy::ClassDuration { limit: LIMIT });
    let at = Instant(START.0 + LIMIT.millis());

    apply_finish_policy(&mut state, at);
    let again = apply_finish_policy(&mut state, Instant(at.0 + 5_000));

    assert!(again.is_empty(), "a finish is derived once, not re-announced every tick");
}

#[tokio::test]
async fn an_undecided_rule_finishes_nobody_ever() {
    // Competition's rule is NotConfigured, which evaluates to Undetermined. Treating that
    // as "not finished" or as "finished" would both be inventing it (CLAUDE.md 12, 28).
    let mut state = class(SessionMode::Competition, FinishPolicy::NotConfigured);

    let finished = apply_finish_policy(&mut state, Instant(START.0 + 10 * LIMIT.millis()));

    assert!(finished.is_empty());
    assert_eq!(state.athlete("a1").unwrap().status, AthleteStatus::Active);
}

#[tokio::test]
async fn coach_decides_never_finishes_anyone_automatically() {
    let mut state = class(SessionMode::Training, FinishPolicy::CoachDecides);

    let finished = apply_finish_policy(&mut state, Instant(START.0 + 10 * LIMIT.millis()));

    assert!(finished.is_empty());
}

#[tokio::test]
async fn the_coach_can_end_the_class_by_hand() {
    let store = FakeStore::new();
    let mut state = class(SessionMode::Training, FinishPolicy::CoachDecides);
    let cmd = OperatorCommand::new("COACH TABLET", Instant(START.0 + 3_000_000));

    let finished = end_class(&mut state, &store, &cmd).await.expect("end class");

    assert_eq!(finished, vec!["a1".to_string()]);
    assert_eq!(state.session.status, SessionStatus::Completed);
    assert_eq!(store.audits()[0].action, "CLASS_END");
}

#[tokio::test]
async fn ending_a_class_by_hand_is_refused_when_no_finish_rule_exists() {
    let store = FakeStore::new();
    let mut state = class(SessionMode::Competition, FinishPolicy::NotConfigured);
    let cmd = OperatorCommand::new("COACH TABLET", Instant(START.0 + 3_000_000));

    let err = end_class(&mut state, &store, &cmd).await.expect_err("no rule to apply");

    assert!(matches!(err, OperatorError::NoFinishRule));
    assert_eq!(state.athlete("a1").unwrap().status, AthleteStatus::Active);
    assert_eq!(state.session.status, SessionStatus::Running);
}

#[tokio::test]
async fn an_athlete_caught_inside_a_station_keeps_that_run_open() {
    let mut state = class(SessionMode::Training, FinishPolicy::ClassDuration { limit: LIMIT });
    {
        let a = state.athlete_mut("a1").unwrap();
        domain::apply(
            a,
            &domain::Interpreted::Entered {
                station: "SKIERG".into(),
                at: Instant(START.0 + 1000),
                transition: None,
                started_timing: false,
            },
        );
    }

    apply_finish_policy(&mut state, Instant(START.0 + LIMIT.millis()));

    let a = state.athlete("a1").unwrap();
    assert_eq!(a.status, AthleteStatus::Finished);
    // No reader reported them leaving; inventing an exit would fabricate a split.
    assert_eq!(a.runs[0].exited_at, None);
}
