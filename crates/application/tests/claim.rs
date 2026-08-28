//! Retroactive claim: binding a band re-interprets the reads that arrived before it
//! (ADR 0001 D3).
//!
//! The promise D3 makes is that an athlete who scanned in before anyone handed them a bound
//! band does not lose the time. The raw reads were always kept, so nothing was lost -- but
//! claiming them was manual. The property that matters is equivalence: a late binding must
//! produce exactly what an early one would have.

mod support;

use application::{
    checkin::{bind_tag, rebind_tag},
    ingest_read, IngestOutcome, LiveSession, OperatorCommand, OperatorError,
};
use domain::{
    AthleteState, BindingLedger, ExceptionReason, Instant, Interpreted, ReaderKey, ReaderMode,
    ReaderRegistration, ReaderRegistry, Session, SessionConfig, SessionMode, TagId,
};
use contract::{EdgeEvent, ReceivedEvent};
use support::FakeStore;

const DEVICE: &str = "a4:cf:12:8b:3d:91";
const CLASS_START: Instant = Instant(1_000_000);

fn read(reader: &str, tag: &str, sequence: i64, at: i64) -> ReceivedEvent {
    let event = EdgeEvent {
        device_id: contract::DeviceId::from_mac_str(DEVICE).expect("device id"),
        reader_id: contract::ReaderId::parse(reader).expect("reader id"),
        boot_id: 7,
        sequence,
        tag_id: tag.to_string(),
        detected_at: at,
        uptime_ms: at - CLASS_START.0,
    };
    ReceivedEvent::new(event, at + 250)
}

fn readers() -> ReaderRegistry {
    let device = format!("esp32-{}", DEVICE.replace(':', ""));
    let mut registry = ReaderRegistry::new();
    for (reader, mode) in [("rfid-01", ReaderMode::Entry), ("rfid-02", ReaderMode::Exit)] {
        registry.register(ReaderRegistration::new(
            ReaderKey::parse(&device, reader).expect("key"),
            "SKIERG",
            mode,
        ));
    }
    registry
}

/// One armed training session with a roster of one. `bound` decides whether the band is
/// already on the wrist when the reads arrive.
fn session(bound: bool) -> LiveSession {
    let mut s = Session::new_draft("s1", "THU 19:00", SessionMode::Training);
    s.arm().expect("draft arms");

    let mut bindings = BindingLedger::new();
    if bound {
        bindings
            .bind("s1", &TagId::parse("TAG-A1").unwrap(), "a1", CLASS_START)
            .expect("bind");
    }

    LiveSession::new(s, SessionConfig::new("s1"), CLASS_START)
        .with_athletes(vec![AthleteState::ready("a1", "CHEN YU-TING")])
        .with_readers(readers())
        .with_bindings(bindings)
}

/// SKIERG in, SKIERG out, SKIERG in again: enough for a transition to be computed, which is
/// the value most sensitive to replay order (CLAUDE.md 13).
async fn run_the_class(state: &mut LiveSession, store: &FakeStore) {
    for (reader, seq, at) in
        [("rfid-01", 1, 1_010_000), ("rfid-02", 2, 1_070_000), ("rfid-01", 3, 1_090_000)]
    {
        ingest_read(state, store, &read(reader, "TAG-A1", seq, at))
            .await
            .expect("ingest");
    }
}

#[tokio::test]
async fn binding_a_band_claims_the_reads_that_arrived_before_it() {
    let store = FakeStore::new();
    let mut state = session(false);
    run_the_class(&mut state, &store).await;

    // Nothing interpreted yet: the reads are durable and the band is on the check-in list.
    assert_eq!(store.raw_count(), 3);
    assert!(store.interpreted().is_empty());
    assert_eq!(state.pending_tags().len(), 1);

    let claimed = bind_tag(
        &mut state,
        &store,
        &TagId::parse("TAG-A1").unwrap(),
        "a1",
        &OperatorCommand::new("CHECKIN TABLET", Instant(1_100_000)),
    )
    .await
    .expect("bind");

    assert_eq!(claimed.len(), 3, "every stored read is claimed");
    let athlete = state.athlete("a1").expect("on the roster");
    assert_eq!(athlete.status, domain::AthleteStatus::Active);
    // The clock starts at the first read, not at the moment of binding (CLAUDE.md 11).
    assert_eq!(athlete.started_at, Some(Instant(1_010_000)));
    assert_eq!(athlete.runs.len(), 2);
    assert!(state.pending_tags().is_empty());
}

#[tokio::test]
async fn a_late_binding_produces_exactly_what_an_early_one_would_have() {
    // The equivalence D3 promises. Anything less means the time is technically preserved
    // but arithmetically different, which on a results screen is the same as losing it.
    let early_store = FakeStore::new();
    let mut early = session(true);
    run_the_class(&mut early, &early_store).await;

    let late_store = FakeStore::new();
    let mut late = session(false);
    run_the_class(&mut late, &late_store).await;
    bind_tag(
        &mut late,
        &late_store,
        &TagId::parse("TAG-A1").unwrap(),
        "a1",
        &OperatorCommand::new("CHECKIN TABLET", Instant(1_100_000)),
    )
    .await
    .expect("bind");

    assert_eq!(
        late_store.interpreted_events(),
        early_store.interpreted_events(),
        "the same reads must mean the same thing whenever the band was claimed"
    );

    let (a, b) = (early.athlete("a1").unwrap(), late.athlete("a1").unwrap());
    assert_eq!(a.status, b.status);
    assert_eq!(a.station_state, b.station_state);
    assert_eq!(a.current_station, b.current_station);
    assert_eq!(a.started_at, b.started_at);
    assert_eq!(a.last_exit_at, b.last_exit_at);
    assert_eq!(a.runs.len(), b.runs.len());
    for (x, y) in a.runs.iter().zip(&b.runs) {
        assert_eq!(x.station, y.station);
        assert_eq!(x.entered_at, y.entered_at);
        assert_eq!(x.exited_at, y.exited_at);
        // The transition is derived from the previous exit, so it only comes out right if
        // the claim replayed in detected_at order (CLAUDE.md 13).
        assert_eq!(x.transition_from_prev, y.transition_from_prev);
    }
    assert_eq!(early.session.interpreted_event_count, late.session.interpreted_event_count);
}

#[tokio::test]
async fn claiming_twice_does_not_interpret_a_read_twice() {
    let store = FakeStore::new();
    let mut state = session(false);
    run_the_class(&mut state, &store).await;
    let tag = TagId::parse("TAG-A1").unwrap();
    bind_tag(&mut state, &store, &tag, "a1", &OperatorCommand::new("CHECKIN", Instant(1_100_000)))
        .await
        .expect("bind");

    // Re-running the claim, as a repeated tap on /checkin would.
    let again = rebind_tag(
        &mut state,
        &store,
        &tag,
        "a1",
        &OperatorCommand::new("CHECKIN", Instant(1_110_000)).with_reason("誤觸"),
    )
    .await
    .expect("same band, same athlete");

    assert!(again.is_empty(), "a claimed read is claimed once");
    assert_eq!(store.interpreted().len(), 3);
    assert_eq!(state.athlete("a1").unwrap().runs.len(), 2);
}

#[tokio::test]
async fn a_claimed_read_from_an_unregistered_reader_is_still_an_exception() {
    // Claiming goes through the same reader resolution as the live path, so a venue
    // mis-configuration surfaces in the inbox rather than being quietly accepted.
    let store = FakeStore::new();
    let mut state = session(false);
    ingest_read(&mut state, &store, &read("rfid-99", "TAG-A1", 1, 1_010_000))
        .await
        .expect("ingest");

    let claimed = bind_tag(
        &mut state,
        &store,
        &TagId::parse("TAG-A1").unwrap(),
        "a1",
        &OperatorCommand::new("CHECKIN", Instant(1_100_000)),
    )
    .await
    .expect("bind");

    assert!(matches!(
        claimed[0],
        Interpreted::Exception { reason: ExceptionReason::UnknownReader, .. }
    ));
    assert_eq!(state.exception_count, 1);
    assert_eq!(state.athlete("a1").unwrap().status, domain::AthleteStatus::Ready);
}

#[tokio::test]
async fn a_band_bound_before_the_class_has_nothing_to_claim() {
    let store = FakeStore::new();
    let mut state = session(false);

    let claimed = bind_tag(
        &mut state,
        &store,
        &TagId::parse("TAG-A1").unwrap(),
        "a1",
        &OperatorCommand::new("CHECKIN", CLASS_START),
    )
    .await
    .expect("bind");

    assert!(claimed.is_empty());
    assert!(store.interpreted().is_empty());
}

#[tokio::test]
async fn swapping_a_band_needs_a_reason() {
    // Moving someone's results onto a different band changes recorded data (CLAUDE.md 20).
    let store = FakeStore::new();
    let mut state = session(true);

    let err = rebind_tag(
        &mut state,
        &store,
        &TagId::parse("TAG-A9").unwrap(),
        "a1",
        &OperatorCommand::new("CHECKIN", Instant(1_100_000)),
    )
    .await
    .expect_err("no reason given");

    assert!(matches!(err, OperatorError::ReasonRequired));
}

#[tokio::test]
async fn swapping_a_band_keeps_what_the_old_one_already_recorded() {
    let store = FakeStore::new();
    let mut state = session(true);
    run_the_class(&mut state, &store).await;

    rebind_tag(
        &mut state,
        &store,
        &TagId::parse("TAG-A9").unwrap(),
        "a1",
        &OperatorCommand::new("CHECKIN", Instant(1_100_000)).with_reason("腳環故障"),
    )
    .await
    .expect("swap");

    // Two ledger rows, one closed, both auditable (ADR 0001 D3).
    assert_eq!(state.bindings.history().len(), 2);
    assert_eq!(state.bindings.active().count(), 1);
    assert_eq!(store.audits().last().expect("audit").action, "TAG_REBIND");
    // The old band's stations are untouched: a swap is not a re-run.
    assert_eq!(state.athlete("a1").unwrap().runs.len(), 2);
    assert_eq!(store.interpreted().len(), 3);
}

#[tokio::test]
async fn the_new_band_takes_over_from_the_swap_onwards() {
    let store = FakeStore::new();
    let mut state = session(true);
    run_the_class(&mut state, &store).await;
    rebind_tag(
        &mut state,
        &store,
        &TagId::parse("TAG-A9").unwrap(),
        "a1",
        &OperatorCommand::new("CHECKIN", Instant(1_100_000)).with_reason("腳環故障"),
    )
    .await
    .expect("swap");

    let out = ingest_read(&mut state, &store, &read("rfid-02", "TAG-A9", 4, 1_150_000))
        .await
        .expect("ingest on the new band");

    match out.outcome {
        IngestOutcome::Interpreted { ref athlete_id, event: Interpreted::Exited { .. } } => {
            assert_eq!(athlete_id, "a1")
        }
        other => panic!("expected the new band to close the station run, got {other:?}"),
    }
}
