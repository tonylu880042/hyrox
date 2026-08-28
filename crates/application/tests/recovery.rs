//! Startup recovery and check-in (CLAUDE.md 21; ADR 0001 D3).

mod support;

use application::{
    checkin::{admit, bind_tag},
    resume_or_start, HubStore, LiveSession, OperatorCommand, OperatorError, Recovery,
    RosterEntry, SessionPlan,
};
use domain::{
    AthleteState, Instant, MemberRef, MembershipStatus, Session, SessionConfig, SessionMode,
    SessionStatus, TagId,
};
use support::{FakeDirectory, FakeStore};

const START: Instant = Instant(1_000_000);

fn plan() -> SessionPlan {
    SessionPlan {
        session: Session::new_draft("s-new", "THU 19:00", SessionMode::Training),
        config: SessionConfig::new("s-new"),
        roster: vec![RosterEntry {
            athlete_id: "a1".into(),
            display_name: "CHEN YU-TING".into(),
        }],
        class_start: START,
    }
}

#[tokio::test]
async fn an_empty_store_starts_and_persists_the_planned_session() {
    let store = FakeStore::new();

    let (state, how) = resume_or_start(&store, plan()).await.expect("start");

    assert_eq!(how, Recovery::Started);
    assert_eq!(state.session.status, SessionStatus::Armed);
    assert_eq!(state.athletes.len(), 1);
    assert_eq!(store.saved_sessions()[0].id, "s-new");
}

#[tokio::test]
async fn an_armed_session_is_resumed_and_its_athletes_rebuilt() {
    let mut existing = Session::new_draft("s-old", "WED 19:00", SessionMode::Training);
    existing.arm().expect("arm");
    existing.interpreted_event_count = 4;
    let mut running = AthleteState::ready("b1", "LIN CHIA-HAO");
    running.status = domain::AthleteStatus::Active;
    let store = FakeStore::new()
        .with_session(existing, Instant(500_000))
        .with_rebuilt_athletes(vec![running]);

    let (state, how) = resume_or_start(&store, plan()).await.expect("resume");

    assert_eq!(how, Recovery::Resumed);
    assert_eq!(state.session.id, "s-old");
    // The class clock keeps its original origin, or a restart would rewind it.
    assert_eq!(state.class_start, Instant(500_000));
    assert_eq!(state.athletes[0].athlete_id, "b1");
}

#[tokio::test]
async fn a_resumed_session_gets_its_exception_badge_back() {
    let mut existing = Session::new_draft("s-old", "WED 19:00", SessionMode::Training);
    existing.arm().expect("arm");
    let store = FakeStore::new().with_session(existing, Instant(500_000));
    store
        .commit_interpreted(application::InterpretedWrite {
            session_id: "s-old",
            athlete_id: "b1",
            raw_event_id: None,
            event: &domain::Interpreted::Exception {
                reason: domain::ExceptionReason::UnknownReader,
                at: Instant(600_000),
            },
        })
        .await
        .expect("record an exception before the restart");

    let (state, _) = resume_or_start(&store, plan()).await.expect("resume");

    // The inbox badge is stored, not remembered: a restart must not hide an open exception
    // (ADR 0001 D4).
    assert_eq!(state.exception_count, 1);
}

#[tokio::test]
async fn a_closed_session_is_not_resumed() {
    let mut closed = Session::new_draft("s-old", "WED 19:00", SessionMode::Training);
    closed.arm().expect("arm");
    closed.close().expect("close");
    let store = FakeStore::new().with_session(closed, Instant(500_000));

    let (state, how) = resume_or_start(&store, plan()).await.expect("start");

    assert_eq!(how, Recovery::Started);
    assert_eq!(state.session.id, "s-new");
}

#[tokio::test]
async fn a_member_with_a_lapsed_membership_is_still_admitted() {
    // Confirmed 2026-08-27: 健身管 status is displayed, never a gate (CLAUDE.md 31).
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, plan()).await.expect("start");
    let member = MemberRef::new("m-9", "WANG SHU-FEN", MembershipStatus::Expired);

    admit(&mut state, &store, &member).await.expect("admit");

    assert!(state.athlete("m-9").is_some());
}

#[tokio::test]
async fn admitting_the_same_member_twice_adds_one_roster_line() {
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, plan()).await.expect("start");
    let member = MemberRef::new("m-9", "WANG SHU-FEN", MembershipStatus::Active);

    admit(&mut state, &store, &member).await.expect("admit");
    admit(&mut state, &store, &member).await.expect("admit again");

    assert_eq!(state.athletes.len(), 2);
}

#[tokio::test]
async fn binding_a_pending_tag_clears_it_from_the_checkin_list() {
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, plan()).await.expect("start");
    let tag = TagId::parse("TAG-A1").unwrap();
    state.note_pending_tag(tag.clone());

    bind_tag(&mut state, &store, &tag, "a1", &OperatorCommand::new("CHECKIN TABLET", START))
        .await
        .expect("bind");

    assert!(state.pending_tags().is_empty());
    assert_eq!(state.bindings.athlete_for_tag("s-new", &tag), Some("a1"));
    assert_eq!(store.audits()[0].action, "TAG_BIND");
}

#[tokio::test]
async fn a_tag_cannot_be_bound_to_someone_who_is_not_in_the_session() {
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, plan()).await.expect("start");
    let tag = TagId::parse("TAG-A1").unwrap();

    let err = bind_tag(
        &mut state,
        &store,
        &tag,
        "nobody",
        &OperatorCommand::new("CHECKIN TABLET", START),
    )
    .await
    .expect_err("unknown athlete");

    assert!(matches!(err, OperatorError::UnknownAthlete(_)));
}

#[tokio::test]
async fn the_member_directory_stub_reports_that_it_is_not_configured() {
    use application::{MemberDirectory, UnconfiguredDirectory};

    // The 健身管 contract is unknown (docs/open-issues.md), so the only honest client is
    // one that says so rather than guessing an endpoint.
    let err = UnconfiguredDirectory.lookup("m-1").await.expect_err("no client yet");

    assert_eq!(err, application::DirectoryError::NotConfigured);
}

#[tokio::test]
async fn a_directory_answering_no_such_member_is_not_a_fault() {
    use application::MemberDirectory;

    let directory = FakeDirectory(vec![MemberRef::new("m-1", "A", MembershipStatus::Active)]);

    assert!(directory.lookup("m-1").await.expect("lookup").is_some());
    // Ok(None) is an answer; only an unreachable directory is an Err.
    assert!(directory.lookup("m-2").await.expect("lookup").is_none());
}

#[tokio::test]
async fn a_resumed_session_keeps_counting_from_its_stored_event_count() {
    let mut existing = Session::new_draft("s-old", "WED 19:00", SessionMode::Training);
    existing.arm().expect("arm");
    existing.interpreted_event_count = 4;
    let store = FakeStore::new().with_session(existing, Instant(500_000));

    let (state, _): (LiveSession, _) = resume_or_start(&store, plan()).await.expect("resume");

    // Gates ARMED -> DRAFT after a restart just as it did before (ADR 0001 D2).
    assert_eq!(state.session.interpreted_event_count, 4);
}
