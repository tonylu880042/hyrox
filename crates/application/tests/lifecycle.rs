//! Session lifecycle use cases (ADR 0001 D2; ADR 0008; CLAUDE.md 20).

mod support;

use application::{
    session::{cancel, complete, mark_ready, pause, reopen, resume, return_to_draft, start},
    LiveSession, OperatorCommand, OperatorError,
};
use domain::{Duration, Instant, Session, SessionConfig, SessionMode, SessionStatus};
use support::{Call, FakeStore};

const NOW: Instant = Instant(2_000_000);
const CLASS_START: Instant = Instant(1_000_000);

fn draft() -> LiveSession {
    LiveSession::new(
        Session::new_draft("s1", "THU 19:00", SessionMode::Training),
        SessionConfig::new("s1"),
        CLASS_START,
    )
}

fn tablet() -> OperatorCommand {
    // Identity is the device name; there is no login (ADR 0001 D1).
    OperatorCommand::new("FRONT DESK TABLET", NOW)
}

fn at(ms: i64) -> OperatorCommand {
    OperatorCommand::new("FRONT DESK TABLET", Instant(ms))
}

/// DRAFT -> RUNNING, which is two transitions now (ADR 0008).
async fn run_class(state: &mut LiveSession, store: &FakeStore) {
    mark_ready(state, store, &tablet()).await.expect("ready");
    start(state, store, &tablet()).await.expect("start");
}

#[tokio::test]
async fn starting_a_class_persists_each_new_status() {
    let store = FakeStore::new();
    let mut state = draft();

    run_class(&mut state, &store).await;

    assert_eq!(state.session.status, SessionStatus::Running);
    assert_eq!(store.saved_sessions()[0].status, SessionStatus::Running);
    // Both transitions were written, not just the last one: a crash between them must not
    // leave a class that is running with nothing on disk saying so.
    let statuses: Vec<String> = store
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Session { status, .. } => Some(status),
            _ => None,
        })
        .collect();
    assert_eq!(statuses, ["READY", "RUNNING"]);
}

#[tokio::test]
async fn a_draft_cannot_be_started_without_passing_through_ready() {
    let store = FakeStore::new();
    let mut state = draft();

    let err = start(&mut state, &store, &tablet())
        .await
        .expect_err("DRAFT is not startable");

    assert!(matches!(err, OperatorError::Session(_)));
    assert!(
        store.saved_sessions().is_empty(),
        "a rejected transition writes nothing"
    );
}

#[tokio::test]
async fn every_lifecycle_change_is_audited_with_the_operator_device() {
    let store = FakeStore::new();
    let mut state = draft();

    run_class(&mut state, &store).await;

    let audits = store.audits();
    assert_eq!(audits[0].action, "SESSION_READY");
    let audit = &audits[1];
    assert_eq!(audit.action, "SESSION_START");
    assert_eq!(audit.operator, "FRONT DESK TABLET");
    assert_eq!(audit.subject, "s1");
    assert_eq!(audit.before.as_deref(), Some("READY"));
    assert_eq!(audit.after.as_deref(), Some("RUNNING"));
}

#[tokio::test]
async fn completing_a_draft_is_rejected_by_the_domain() {
    let store = FakeStore::new();
    let mut state = draft();

    let err = complete(&mut state, &store, &tablet())
        .await
        .expect_err("DRAFT cannot complete");

    assert!(matches!(err, OperatorError::Session(_)));
    assert!(store.saved_sessions().is_empty());
}

#[tokio::test]
async fn starting_a_completed_session_is_refused_because_it_is_a_correction() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;
    complete(&mut state, &store, &tablet())
        .await
        .expect("complete");

    let err = start(&mut state, &store, &tablet())
        .await
        .expect_err("must go through reopen");

    assert!(matches!(err, OperatorError::ReasonRequired));
    assert_eq!(state.session.status, SessionStatus::Completed);
}

// --- pause and resume ------------------------------------------------------------------

#[tokio::test]
async fn pausing_stops_the_class_clock_and_resuming_restarts_it() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;

    // 10s in, pause. 30s of wall time later, resume.
    pause(&mut state, &store, &at(CLASS_START.0 + 10_000))
        .await
        .expect("pause");
    assert_eq!(state.session.status, SessionStatus::Paused);
    assert_eq!(
        state.class_elapsed(Instant(CLASS_START.0 + 25_000)),
        Duration(10_000)
    );

    resume(&mut state, &store, &at(CLASS_START.0 + 40_000))
        .await
        .expect("resume");
    assert_eq!(state.session.status, SessionStatus::Running);
    assert_eq!(
        state.class_elapsed(Instant(CLASS_START.0 + 45_000)),
        Duration(15_000)
    );
}

#[tokio::test]
async fn a_pause_is_persisted_so_a_restart_comes_back_paused() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;
    pause(&mut state, &store, &at(CLASS_START.0 + 10_000))
        .await
        .expect("pause");

    let saved = store.saved_sessions().pop().expect("a saved session");
    assert_eq!(saved.status, SessionStatus::Paused);
    assert_eq!(saved.paused_since, Some(Instant(CLASS_START.0 + 10_000)));
}

#[tokio::test]
async fn a_paused_class_does_not_accept_reads() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;
    pause(&mut state, &store, &at(CLASS_START.0 + 10_000))
        .await
        .expect("pause");

    assert!(!state.session.accepts_events());
}

#[tokio::test]
async fn a_paused_class_can_be_completed_without_resuming() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;
    pause(&mut state, &store, &at(CLASS_START.0 + 10_000))
        .await
        .expect("pause");

    complete(&mut state, &store, &tablet())
        .await
        .expect("complete");

    assert_eq!(state.session.status, SessionStatus::Completed);
}

// --- cancel ----------------------------------------------------------------------------

#[tokio::test]
async fn cancelling_without_a_reason_is_refused() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;

    let err = cancel(&mut state, &store, &tablet())
        .await
        .expect_err("reason required");

    assert!(matches!(err, OperatorError::ReasonRequired));
    assert_eq!(state.session.status, SessionStatus::Running);
}

#[tokio::test]
async fn cancelling_with_a_reason_records_it() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;

    cancel(&mut state, &store, &tablet().with_reason("停電"))
        .await
        .expect("cancel");

    assert_eq!(state.session.status, SessionStatus::Cancelled);
    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "SESSION_CANCEL");
    assert_eq!(audit.reason.as_deref(), Some("停電"));
}

#[tokio::test]
async fn a_cancelled_class_is_not_reopened() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;
    cancel(&mut state, &store, &tablet().with_reason("停電"))
        .await
        .expect("cancel");

    let err = reopen(&mut state, &store, &tablet().with_reason("誤按"))
        .await
        .expect_err("a written-off class is not resurrected");

    assert!(matches!(err, OperatorError::Session(_)));
}

// --- reopen ----------------------------------------------------------------------------

#[tokio::test]
async fn reopening_without_a_reason_is_refused() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;
    complete(&mut state, &store, &tablet())
        .await
        .expect("complete");

    let err = reopen(&mut state, &store, &tablet())
        .await
        .expect_err("reason required");

    assert!(matches!(err, OperatorError::ReasonRequired));
}

#[tokio::test]
async fn a_blank_reason_is_not_a_reason() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;
    complete(&mut state, &store, &tablet())
        .await
        .expect("complete");

    let err = reopen(&mut state, &store, &tablet().with_reason("   "))
        .await
        .expect_err("whitespace explains nothing");

    assert!(matches!(err, OperatorError::ReasonRequired));
}

#[tokio::test]
async fn reopening_with_a_reason_records_it() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;
    complete(&mut state, &store, &tablet())
        .await
        .expect("complete");

    reopen(&mut state, &store, &tablet().with_reason("誤觸"))
        .await
        .expect("reopen");

    assert_eq!(state.session.status, SessionStatus::Running);
    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "SESSION_REOPEN");
    assert_eq!(audit.reason.as_deref(), Some("誤觸"));
    assert_eq!(audit.before.as_deref(), Some("COMPLETED"));
}

// --- back to draft ---------------------------------------------------------------------

#[tokio::test]
async fn an_untouched_session_can_go_back_to_draft() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;

    return_to_draft(&mut state, &store, &tablet())
        .await
        .expect("back to draft");

    assert_eq!(state.session.status, SessionStatus::Draft);
}

#[tokio::test]
async fn a_session_that_has_interpreted_events_cannot_go_back_to_draft() {
    let store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;
    state.session.interpreted_event_count = 1;

    let err = return_to_draft(&mut state, &store, &tablet())
        .await
        .expect_err("results exist");

    assert!(matches!(
        err,
        OperatorError::Session(domain::SessionError::HasInterpretedEvents)
    ));
    assert_eq!(state.session.status, SessionStatus::Running);
}

#[tokio::test]
async fn a_storage_failure_during_pause_leaves_in_memory_session_status_running() {
    let mut store = FakeStore::new();
    let mut state = draft();
    run_class(&mut state, &store).await;

    assert_eq!(state.session.status, SessionStatus::Running);

    store.fail_save_session = true;
    let err = pause(&mut state, &store, &at(CLASS_START.0 + 10_000))
        .await
        .expect_err("pause fails when store fails");
    assert!(matches!(err, OperatorError::Storage(_)));

    assert_eq!(
        state.session.status,
        SessionStatus::Running,
        "session status in memory must stay Running if persistence fails"
    );
}
