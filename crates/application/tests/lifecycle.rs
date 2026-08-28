//! Session lifecycle use cases (ADR 0001 D2; CLAUDE.md 20).

mod support;

use application::{
    session::{arm, close, reopen, return_to_draft},
    LiveSession, OperatorCommand, OperatorError,
};
use domain::{Instant, Session, SessionConfig, SessionMode, SessionStatus};
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
    // Identity is the device name; there is no login (ADR 0001 D1).
    OperatorCommand::new("FRONT DESK TABLET", NOW)
}

#[tokio::test]
async fn arming_a_draft_persists_the_new_status() {
    let store = FakeStore::new();
    let mut state = draft();

    arm(&mut state, &store, &tablet()).await.expect("arm");

    assert_eq!(state.session.status, SessionStatus::Armed);
    assert_eq!(store.saved_sessions()[0].status, SessionStatus::Armed);
}

#[tokio::test]
async fn every_lifecycle_change_is_audited_with_the_operator_device() {
    let store = FakeStore::new();
    let mut state = draft();

    arm(&mut state, &store, &tablet()).await.expect("arm");

    let audit = &store.audits()[0];
    assert_eq!(audit.action, "SESSION_ARM");
    assert_eq!(audit.operator, "FRONT DESK TABLET");
    assert_eq!(audit.subject, "s1");
    assert_eq!(audit.before.as_deref(), Some("DRAFT"));
    assert_eq!(audit.after.as_deref(), Some("ARMED"));
}

#[tokio::test]
async fn closing_a_draft_is_rejected_by_the_domain() {
    let store = FakeStore::new();
    let mut state = draft();

    let err = close(&mut state, &store, &tablet()).await.expect_err("DRAFT cannot close");

    assert!(matches!(err, OperatorError::Session(_)));
    assert!(store.saved_sessions().is_empty(), "a rejected transition writes nothing");
}

#[tokio::test]
async fn arming_a_closed_session_is_refused_because_it_is_a_correction() {
    let store = FakeStore::new();
    let mut state = draft();
    arm(&mut state, &store, &tablet()).await.expect("arm");
    close(&mut state, &store, &tablet()).await.expect("close");

    let err = arm(&mut state, &store, &tablet()).await.expect_err("must go through reopen");

    assert!(matches!(err, OperatorError::ReasonRequired));
    assert_eq!(state.session.status, SessionStatus::Closed);
}

#[tokio::test]
async fn reopening_without_a_reason_is_refused() {
    let store = FakeStore::new();
    let mut state = draft();
    arm(&mut state, &store, &tablet()).await.expect("arm");
    close(&mut state, &store, &tablet()).await.expect("close");

    let err = reopen(&mut state, &store, &tablet()).await.expect_err("reason required");

    assert!(matches!(err, OperatorError::ReasonRequired));
}

#[tokio::test]
async fn a_blank_reason_is_not_a_reason() {
    let store = FakeStore::new();
    let mut state = draft();
    arm(&mut state, &store, &tablet()).await.expect("arm");
    close(&mut state, &store, &tablet()).await.expect("close");

    let err = reopen(&mut state, &store, &tablet().with_reason("   "))
        .await
        .expect_err("whitespace explains nothing");

    assert!(matches!(err, OperatorError::ReasonRequired));
}

#[tokio::test]
async fn reopening_with_a_reason_records_it() {
    let store = FakeStore::new();
    let mut state = draft();
    arm(&mut state, &store, &tablet()).await.expect("arm");
    close(&mut state, &store, &tablet()).await.expect("close");

    reopen(&mut state, &store, &tablet().with_reason("誤觸"))
        .await
        .expect("reopen");

    assert_eq!(state.session.status, SessionStatus::Armed);
    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "SESSION_REOPEN");
    assert_eq!(audit.reason.as_deref(), Some("誤觸"));
    assert_eq!(audit.before.as_deref(), Some("CLOSED"));
}

#[tokio::test]
async fn an_untouched_session_can_go_back_to_draft() {
    let store = FakeStore::new();
    let mut state = draft();
    arm(&mut state, &store, &tablet()).await.expect("arm");

    return_to_draft(&mut state, &store, &tablet()).await.expect("back to draft");

    assert_eq!(state.session.status, SessionStatus::Draft);
}

#[tokio::test]
async fn a_session_that_has_interpreted_events_cannot_go_back_to_draft() {
    let store = FakeStore::new();
    let mut state = draft();
    arm(&mut state, &store, &tablet()).await.expect("arm");
    state.session.interpreted_event_count = 1;

    let err = return_to_draft(&mut state, &store, &tablet())
        .await
        .expect_err("results exist");

    assert!(matches!(err, OperatorError::Session(domain::SessionError::HasInterpretedEvents)));
    assert_eq!(state.session.status, SessionStatus::Armed);
}
