//! Editing a session's configuration (ADR 0001 D2).

mod support;

use application::{configure, LiveSession, OperatorCommand, OperatorError};
use domain::{
    Course, CourseStep, Duration, FinishPolicy, Instant, Session, SessionConfig, SessionMode,
    SessionStatus, StationTarget,
};
use support::FakeStore;

const NOW: Instant = Instant(2_000_000);

fn draft() -> LiveSession {
    LiveSession::new(
        Session::new_draft("s1", "THU 19:00", SessionMode::Training),
        SessionConfig::new("s1"),
        Instant(1_000_000),
    )
}

fn tablet() -> OperatorCommand {
    OperatorCommand::new("FRONT DESK TABLET", NOW)
}

/// Repeated stations and per-station targets are both part of the course model
/// (CLAUDE.md 9.2), so the editing path has to carry them.
fn intervals() -> Course {
    Course::new(
        "4 x SKIERG",
        vec![
            CourseStep::new("RUN").with_target(StationTarget::Distance { meters: 400 }),
            CourseStep::new("SKIERG").with_target(StationTarget::Distance { meters: 500 }),
            CourseStep::new("RUN").with_target(StationTarget::Distance { meters: 400 }),
            CourseStep::new("SKIERG").with_target(StationTarget::Duration {
                duration: Duration(90_000),
            }),
        ],
    )
}

#[tokio::test]
async fn a_draft_session_can_be_given_a_course() {
    let store = FakeStore::new();
    let mut state = draft();

    configure(
        &mut state,
        &store,
        Some(intervals()),
        FinishPolicy::CoachDecides,
        &tablet(),
    )
    .await
    .expect("configure");

    let course = state.config.course.as_ref().expect("a course");
    assert_eq!(course.len(), 4);
    assert_eq!(course.occurrences("SKIERG"), 2);
    assert_eq!(state.config.finish_policy, FinishPolicy::CoachDecides);
}

#[tokio::test]
async fn the_new_configuration_is_persisted_under_the_live_session_id() {
    let store = FakeStore::new();
    let mut state = draft();

    configure(
        &mut state,
        &store,
        Some(intervals()),
        FinishPolicy::CoachDecides,
        &tablet(),
    )
    .await
    .expect("configure");

    let stored = store.saved_configs();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].session_id, "s1");
    assert_eq!(stored[0].course.as_ref().expect("course").steps.len(), 4);
}

#[tokio::test]
async fn per_station_targets_survive_the_round_trip() {
    let store = FakeStore::new();
    let mut state = draft();

    configure(
        &mut state,
        &store,
        Some(intervals()),
        FinishPolicy::CoachDecides,
        &tablet(),
    )
    .await
    .expect("configure");

    let saved = store.saved_configs();
    let steps = &saved[0].course.as_ref().expect("course").steps;
    assert_eq!(
        steps[1].target,
        Some(StationTarget::Distance { meters: 500 })
    );
    assert_eq!(
        steps[3].target,
        Some(StationTarget::Duration {
            duration: Duration(90_000)
        })
    );
}

#[tokio::test]
async fn an_armed_session_cannot_be_reconfigured() {
    let store = FakeStore::new();
    let mut state = draft();
    state.session.mark_ready().expect("arm");
    state.session.start().expect("arm");

    let err = configure(
        &mut state,
        &store,
        Some(intervals()),
        FinishPolicy::CoachDecides,
        &tablet(),
    )
    .await
    .expect_err("ARMED is not editable");

    assert!(matches!(
        err,
        OperatorError::NotEditable {
            status: SessionStatus::Running
        }
    ));
    // The class keeps the rule it was armed under (ADR 0004): nothing was written.
    assert!(store.saved_configs().is_empty());
    assert!(state.config.course.is_none());
}

#[tokio::test]
async fn a_closed_session_cannot_be_reconfigured() {
    let store = FakeStore::new();
    let mut state = draft();
    state.session.mark_ready().expect("arm");
    state.session.start().expect("arm");
    state.session.complete().expect("complete");

    let err = configure(
        &mut state,
        &store,
        None,
        FinishPolicy::CoachDecides,
        &tablet(),
    )
    .await
    .expect_err("CLOSED is not editable");

    assert!(matches!(
        err,
        OperatorError::NotEditable {
            status: SessionStatus::Completed
        }
    ));
}

#[tokio::test]
async fn configuring_is_audited_with_the_operator_device() {
    let store = FakeStore::new();
    let mut state = draft();

    configure(
        &mut state,
        &store,
        Some(intervals()),
        FinishPolicy::ClassDuration {
            limit: Duration(3_600_000),
        },
        &tablet(),
    )
    .await
    .expect("configure");

    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "SESSION_CONFIGURE");
    assert_eq!(audit.operator, "FRONT DESK TABLET");
    assert_eq!(audit.subject, "s1");
    assert_eq!(audit.before.as_deref(), Some("NotConfigured, no course"));
    assert!(audit
        .after
        .as_deref()
        .expect("after")
        .contains("4 x SKIERG"));
}
