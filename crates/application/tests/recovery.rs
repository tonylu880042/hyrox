//! Startup recovery and check-in (CLAUDE.md 21; ADR 0001 D3).

mod support;

use application::{
    checkin::{bind_tag, enter, rebind_tag},
    Entrant,
    register_reader, resume_or_start, HubStore, LiveSession, OperatorCommand, OperatorError,
    Recovery, RosterEntry, SessionPlan,
};
use domain::{
    AthleteState, Duration, FinishPolicy, Instant, MemberRef, MembershipStatus, ReaderKey,
    ReaderMode, ReaderRegistration, Session, SessionConfig, SessionMode, SessionStatus, TagId,
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
    assert_eq!(state.session.status, SessionStatus::Running);
    assert_eq!(state.athletes.len(), 1);
    assert_eq!(store.saved_sessions()[0].id, "s-new");
}

#[tokio::test]
async fn an_armed_session_is_resumed_and_its_athletes_rebuilt() {
    let mut existing = Session::new_draft("s-old", "WED 19:00", SessionMode::Training);
    existing.mark_ready().expect("arm");
    existing.start().expect("arm");
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
    existing.mark_ready().expect("arm");
    existing.start().expect("arm");
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
    closed.mark_ready().expect("arm");
    closed.start().expect("arm");
    closed.complete().expect("complete");
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

    enter(&mut state, &store, Entrant::member(&member), &OperatorCommand::new("CHECKIN TABLET", START)).await.expect("enter");

    assert!(state.athlete("m-9").is_some());
}

#[tokio::test]
async fn admitting_the_same_member_twice_adds_one_roster_line() {
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, plan()).await.expect("start");
    let member = MemberRef::new("m-9", "WANG SHU-FEN", MembershipStatus::Active);

    enter(&mut state, &store, Entrant::member(&member), &OperatorCommand::new("CHECKIN TABLET", START)).await.expect("enter");
    enter(&mut state, &store, Entrant::member(&member), &OperatorCommand::new("CHECKIN TABLET", START)).await.expect("enter again");

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
    existing.mark_ready().expect("arm");
    existing.start().expect("arm");
    existing.interpreted_event_count = 4;
    let store = FakeStore::new().with_session(existing, Instant(500_000));

    let (state, _): (LiveSession, _) = resume_or_start(&store, plan()).await.expect("resume");

    // Gates ARMED -> DRAFT after a restart just as it did before (ADR 0001 D2).
    assert_eq!(state.session.interpreted_event_count, 4);
}

// --- what a restart must bring back (ADR 0004) ----------------------------------------

fn armed(id: &str) -> Session {
    let mut session = Session::new_draft(id, "WED 19:00", SessionMode::Training);
    session.mark_ready().expect("arm");
    session.start().expect("arm");
    session
}

fn class_course() -> domain::Course {
    domain::Course::new(
        "HYROX CLASS",
        vec![
            domain::CourseStep::new("SKIERG")
                .with_target(domain::StationTarget::Distance { meters: 500 }),
            domain::CourseStep::new("WALL BALLS")
                .with_target(domain::StationTarget::Repetitions { count: 50 }),
        ],
    )
}

#[tokio::test]
async fn starting_a_session_persists_the_configuration_it_was_armed_with() {
    let store = FakeStore::new();
    let mut plan = plan();
    plan.config = SessionConfig::new("s-new")
        .with_course(class_course())
        .with_finish_policy(FinishPolicy::ClassDuration { limit: Duration(3_600_000) });

    let (state, how) = resume_or_start(&store, plan).await.expect("start");

    assert_eq!(how, Recovery::Started);
    let stored = store.saved_configs();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0], state.config, "what is stored is what is running");
}

#[tokio::test]
async fn a_resumed_session_keeps_the_finish_policy_it_was_armed_with() {
    // The hole this closes: without a stored configuration the resumed class silently
    // adopted whatever the caller passed in, so a crash could change the rule mid-class.
    let store = FakeStore::new().with_session_config(
        armed("s-old"),
        Instant(500_000),
        Some(
            SessionConfig::new("s-old")
                .with_course(class_course())
                .with_finish_policy(FinishPolicy::ClassDuration { limit: Duration(3_600_000) }),
        ),
    );
    // The caller offers a different rule and a different course, as a fresh startup would.
    let mut plan = plan();
    plan.config = SessionConfig::new("s-new").with_finish_policy(FinishPolicy::CoachDecides);

    let (state, how) = resume_or_start(&store, plan).await.expect("resume");

    assert_eq!(how, Recovery::Resumed);
    assert_eq!(
        state.config.finish_policy,
        FinishPolicy::ClassDuration { limit: Duration(3_600_000) },
        "the rule the class started under, not the caller's"
    );
    assert_eq!(state.config.course.as_ref(), Some(&class_course()));
}

#[tokio::test]
async fn a_resumed_session_with_no_stored_configuration_says_so() {
    // A session armed by a build older than ADR 0004. Falling back to the plan is the only
    // thing left to do, but it must not be silent.
    let store = FakeStore::new().with_unconfigured_session(armed("s-old"), Instant(500_000));

    let (state, how) = resume_or_start(&store, plan()).await.expect("resume");

    assert_eq!(how, Recovery::ResumedWithoutStoredConfig);
    assert_eq!(state.session.id, "s-old");
}

#[tokio::test]
async fn a_resumed_session_gets_its_reader_map_back() {
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, plan()).await.expect("start");
    let key = ReaderKey::parse("esp32-a4cf128b3d91", "rfid-01").expect("key");
    let registration = ReaderRegistration::new(key.clone(), "SKIERG", ReaderMode::Entry);
    register_reader(&mut state, &store, &registration, &OperatorCommand::new("OP", START))
        .await
        .expect("register");

    // Restart: only the store survives.
    let (resumed, how) = resume_or_start(&store, plan()).await.expect("resume");

    assert_eq!(how, Recovery::Resumed);
    let found = resumed.readers.resolve(&key).expect("the reader must resolve as before");
    assert_eq!(found.station, "SKIERG");
    assert_eq!(found.mode, ReaderMode::Entry);
}

#[tokio::test]
async fn re_registering_an_unchanged_reader_writes_no_audit_line() {
    // Every startup re-registers the venue's readers. Auditing that would bury the one line
    // that matters -- the evening a reader was actually repointed (CLAUDE.md 20).
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, plan()).await.expect("start");
    let key = ReaderKey::parse("esp32-a4cf128b3d91", "rfid-01").expect("key");
    let cmd = OperatorCommand::new("OP", START);
    let entry = ReaderRegistration::new(key.clone(), "SKIERG", ReaderMode::Entry);

    register_reader(&mut state, &store, &entry, &cmd).await.expect("first");
    register_reader(&mut state, &store, &entry, &cmd).await.expect("again");
    let moved = ReaderRegistration::new(key.clone(), "ROWING", ReaderMode::Entry);
    register_reader(&mut state, &store, &moved, &cmd).await.expect("repointed");

    let actions: Vec<String> = store.audits().iter().map(|a| a.action.clone()).collect();
    assert_eq!(actions, ["READER_REGISTER", "READER_REGISTER"]);
    assert_eq!(store.audits()[1].before.as_deref(), Some("SKIERG Entry"));
    assert_eq!(state.readers.resolve(&key).expect("resolve").station, "ROWING");
}

#[tokio::test]
async fn a_resumed_session_gets_its_bindings_back_including_the_closed_ones() {
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, plan()).await.expect("start");
    let first = TagId::parse("TAG-A1").unwrap();
    let second = TagId::parse("TAG-A2").unwrap();
    bind_tag(&mut state, &store, &first, "a1", &OperatorCommand::new("CHECKIN", START))
        .await
        .expect("bind");
    rebind_tag(
        &mut state,
        &store,
        &second,
        "a1",
        &OperatorCommand::new("CHECKIN", START).with_reason("腳環故障"),
    )
    .await
    .expect("rebind");

    let (resumed, _) = resume_or_start(&store, plan()).await.expect("resume");

    assert_eq!(resumed.bindings.athlete_for_tag("s-new", &second), Some("a1"));
    assert_eq!(
        resumed.bindings.history().len(),
        2,
        "the closed binding is the audit trail and must survive (CLAUDE.md 20)"
    );
    assert!(resumed.bindings.history().iter().any(|b| b.tag_id == first && !b.is_active()));
}

#[tokio::test]
async fn the_checkin_queue_is_rebuilt_from_the_raw_store() {
    // Pending tags used to live only in memory, so a crash lost the check-in queue
    // (ADR 0001 D3). They are derived from the reads instead.
    let store = FakeStore::new();
    let (state, _) = resume_or_start(&store, plan()).await.expect("start");
    store
        .commit_raw(&application::RawRead {
            device_id: "esp32-a4cf128b3d91".into(),
            reader_id: "rfid-01".into(),
            boot_id: 1,
            sequence: 1,
            tag_id: "TAG-NOBODY".into(),
            detected_at: Instant(START.0 + 5_000),
            received_at: Instant(START.0 + 5_250),
        })
        .await
        .expect("raw read of an unclaimed band");
    drop(state);

    let (resumed, _) = resume_or_start(&store, plan()).await.expect("resume");

    assert_eq!(resumed.pending_tags().len(), 1);
    assert_eq!(resumed.pending_tags()[0].as_str(), "TAG-NOBODY");
}

#[tokio::test]
async fn a_tag_bound_before_the_restart_is_not_queued_for_check_in_again() {
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, plan()).await.expect("start");
    store
        .commit_raw(&application::RawRead {
            device_id: "esp32-a4cf128b3d91".into(),
            reader_id: "rfid-01".into(),
            boot_id: 1,
            sequence: 1,
            tag_id: "TAG-A1".into(),
            detected_at: Instant(START.0 + 5_000),
            received_at: Instant(START.0 + 5_250),
        })
        .await
        .expect("raw");
    let tag = TagId::parse("TAG-A1").unwrap();
    bind_tag(&mut state, &store, &tag, "a1", &OperatorCommand::new("CHECKIN", START))
        .await
        .expect("bind");

    let (resumed, _) = resume_or_start(&store, plan()).await.expect("resume");

    assert!(resumed.pending_tags().is_empty(), "a claimed band is not still waiting");
}
