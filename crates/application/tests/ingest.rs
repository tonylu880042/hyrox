//! The ingestion use case: raw read -> reader -> athlete -> interpretation -> ACK.
//!
//! Every resolution failure has a defined outcome and none of them may drop the event
//! (CLAUDE.md 31 principle 1).

mod support;

use application::{ingest_read, IngestError, IngestOutcome, LiveSession};
use domain::{
    AthleteState, BindingLedger, ExceptionReason, Instant, Interpreted, ReaderKey, ReaderMode,
    ReaderRegistration, ReaderRegistry, Session, SessionConfig, SessionMode, TagId,
};
use contract::{AckStatus, EdgeEvent, ReceivedEvent};
use support::{Call, FakeStore};

/// A round of one tag yields one outcome. The unwrap is the assertion: anything else means
/// the fan-out over a round changed shape (ADR 0014).
fn sole(ingested: &application::Ingested) -> application::IngestOutcome {
    match ingested.outcomes.as_slice() {
        [outcome] => outcome.clone(),
        other => panic!("one tag in, one outcome out; got {other:?}"),
    }
}


const DEVICE: &str = "a4:cf:12:8b:3d:91";
const CLASS_START: Instant = Instant(1_000_000);

fn read(reader: &str, tag: &str, sequence: i64, at: i64) -> ReceivedEvent {
    let event = EdgeEvent {
        device_id: contract::DeviceId::from_mac_str(DEVICE).expect("device id"),
        reader_id: contract::ReaderId::parse(reader).expect("reader id"),
        boot_id: 7,
        sequence,
        tag_id: vec![tag.to_string()],
        detected_at: at,
        uptime_ms: at - CLASS_START.0,
    };
    // received_at is deliberately later than detected_at: arrival is diagnostics, never
    // timing (CLAUDE.md 17).
    ReceivedEvent::new(event, at + 250)
}

/// One inventory round carrying several tags, all seen at the same instant (ADR 0014).
fn round(reader: &str, tags: &[&str], sequence: i64, at: i64) -> ReceivedEvent {
    let mut e = read(reader, "unused", sequence, at).into_event();
    e.tag_id = tags.iter().map(|t| t.to_string()).collect();
    ReceivedEvent::new(e, at + 250)
}

/// One armed training session, one athlete, a SKIERG entry reader, tag bound.
fn armed_session() -> LiveSession {
    let mut session = Session::new_draft("s1", "THU 19:00", SessionMode::Training);
    session.mark_ready().expect("draft arms");
    session.start().expect("draft arms");

    let mut readers = ReaderRegistry::new();
    readers.register(ReaderRegistration::new(
        ReaderKey::parse(&DEVICE.replace(':', ""), "rfid-01").expect("key"),
        "SKIERG",
        ReaderMode::Entry,
    ));
    readers.register(ReaderRegistration::new(
        ReaderKey::parse(&DEVICE.replace(':', ""), "rfid-02").expect("key"),
        "SKIERG",
        ReaderMode::Exit,
    ));

    let mut bindings = BindingLedger::new();
    bindings
        .bind("s1", &TagId::parse("TAG-A1").unwrap(), "a1", CLASS_START)
        .expect("bind");

    LiveSession::new(session, SessionConfig::new("s1"), CLASS_START)
        .with_athletes(vec![AthleteState::ready("a1", "CHEN YU-TING")])
        .with_readers(readers)
        .with_bindings(bindings)
}

#[tokio::test]
async fn a_known_reader_and_a_bound_tag_produce_an_entry() {
    let store = FakeStore::new();
    let mut state = armed_session();

    let out = ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 1_010_000))
        .await
        .expect("ingest");

    match sole(&out) {
        IngestOutcome::Interpreted { ref athlete_id, event: Interpreted::Entered { ref station, started_timing, .. } } => {
            assert_eq!(athlete_id, "a1");
            assert_eq!(station, "SKIERG");
            // The first valid read after ARMED starts this athlete's clock (CLAUDE.md 11).
            assert!(started_timing);
        }
        other => panic!("expected an entry, got {other:?}"),
    }
    assert_eq!(out.ack.payload().status, AckStatus::Stored);
    assert_eq!(state.athlete("a1").unwrap().started_at, Some(Instant(1_010_000)));
}

#[tokio::test]
async fn the_raw_event_is_committed_before_the_interpretation() {
    let store = FakeStore::new();
    let mut state = armed_session();

    ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 1_010_000))
        .await
        .expect("ingest");

    let calls = store.calls();
    assert!(
        matches!(calls[0], Call::Raw { .. }),
        "raw must be committed first, got {calls:?}"
    );
    assert!(matches!(calls[1], Call::Interpreted { .. }), "got {calls:?}");
}

#[tokio::test]
async fn nothing_is_acknowledged_when_the_raw_commit_fails() {
    let store = FakeStore::failing_raw();
    let mut state = armed_session();

    let err = ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 1_010_000))
        .await
        .expect_err("a failed commit must not produce an ACK");

    assert!(matches!(err, IngestError::Storage(_)));
    // Not interpreted either: the edge still holds the event and will resend it.
    assert!(store.interpreted().is_empty());
    assert_eq!(state.athlete("a1").unwrap().status, domain::AthleteStatus::Ready);
}

#[tokio::test]
async fn a_failed_interpretation_still_returns_the_ack_because_the_read_is_durable() {
    let store = FakeStore::failing_interpreted();
    let mut state = armed_session();

    let err = ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 1_010_000))
        .await
        .expect_err("the interpretation write failed");

    match err {
        IngestError::Interpretation { ack, .. } => {
            assert_eq!(ack.payload().sequence, 1);
        }
        other => panic!("expected an interpretation failure, got {other:?}"),
    }
    assert_eq!(store.raw_count(), 1, "the raw read is durable regardless");
}

#[tokio::test]
async fn an_unregistered_reader_becomes_an_unknown_reader_exception() {
    let store = FakeStore::new();
    let mut state = armed_session();

    let out = ingest_read(&mut state, &store, &read("rfid-99", "TAG-A1", 1, 1_010_000))
        .await
        .expect("ingest");

    assert!(matches!(
        sole(&out),
        IngestOutcome::Interpreted {
            event: Interpreted::Exception { reason: ExceptionReason::UnknownReader, .. },
            ..
        }
    ));
    assert_eq!(store.raw_count(), 1, "an unmapped reader must not lose the read");
    assert_eq!(state.exception_count, 1);
}

#[tokio::test]
async fn a_device_id_that_is_not_canonical_is_an_unknown_reader() {
    let store = FakeStore::new();
    let mut state = armed_session();
    // A device the registry has never heard of: same shape, different board.
    let mut event = read("rfid-01", "TAG-A1", 1, 1_010_000).into_event();
    event.device_id = contract::DeviceId::from_mac_str("00:00:00:00:00:01").unwrap();

    let out = ingest_read(&mut state, &store, &ReceivedEvent::new(event, 1_010_250))
        .await
        .expect("ingest");

    assert!(matches!(
        sole(&out),
        IngestOutcome::Interpreted {
            event: Interpreted::Exception { reason: ExceptionReason::UnknownReader, .. },
            ..
        }
    ));
}

#[tokio::test]
async fn an_unbound_tag_is_a_pending_binding_and_not_an_exception() {
    let store = FakeStore::new();
    let mut state = armed_session();

    let out = ingest_read(&mut state, &store, &read("rfid-01", "TAG-NOBODY", 1, 1_010_000))
        .await
        .expect("ingest");

    match sole(&out) {
        IngestOutcome::PendingBinding { ref tag_id } => assert_eq!(tag_id.as_str(), "TAG-NOBODY"),
        other => panic!("expected a pending binding, got {other:?}"),
    }
    // ADR 0001 D3: the read is kept so it can be claimed once the band is bound...
    assert_eq!(store.raw_count(), 1);
    // ...and it is a to-do for /checkin, not an entry in the exception inbox (D4).
    assert!(store.interpreted().is_empty());
    assert_eq!(state.exception_count, 0);
    assert_eq!(state.pending_tags().len(), 1);
}

#[tokio::test]
async fn repeated_reads_of_an_unbound_tag_list_it_once() {
    let store = FakeStore::new();
    let mut state = armed_session();

    for seq in 1..=3 {
        ingest_read(&mut state, &store, &read("rfid-01", "TAG-NOBODY", seq, 1_010_000 + seq))
            .await
            .expect("ingest");
    }

    assert_eq!(state.pending_tags().len(), 1, "one band, one line on /checkin");
    assert_eq!(store.raw_count(), 3, "every read is still stored");
}

#[tokio::test]
async fn a_tag_bound_to_someone_outside_the_roster_is_an_exception() {
    let store = FakeStore::new();
    let mut state = armed_session();
    // Bound in a different class: a real person, just not in this session (ADR 0001 D4).
    state
        .bindings
        .bind("s2", &TagId::parse("TAG-B9").unwrap(), "b9", CLASS_START)
        .expect("bind");

    let out = ingest_read(&mut state, &store, &read("rfid-01", "TAG-B9", 1, 1_010_000))
        .await
        .expect("ingest");

    match sole(&out) {
        IngestOutcome::Interpreted {
            ref athlete_id,
            event: Interpreted::Exception { reason: ExceptionReason::AthleteNotInSession, .. },
        } => assert_eq!(athlete_id, "b9"),
        other => panic!("expected a roster exception, got {other:?}"),
    }
    assert_eq!(state.exception_count, 1);
}

#[tokio::test]
async fn a_redelivered_event_is_acknowledged_but_interpreted_only_once() {
    let store = FakeStore::new();
    let mut state = armed_session();
    let event = read("rfid-01", "TAG-A1", 1, 1_010_000);

    ingest_read(&mut state, &store, &event).await.expect("first");
    let second = ingest_read(&mut state, &store, &event).await.expect("redelivery");

    // Duplicate delivery is allowed, duplicate processing is not (CLAUDE.md 16).
    assert!(matches!(sole(&second), IngestOutcome::Duplicate));
    assert_eq!(second.ack.payload().status, AckStatus::Duplicate);
    assert_eq!(store.interpreted().len(), 1);
    assert_eq!(state.athlete("a1").unwrap().runs.len(), 1);
}

#[tokio::test]
async fn an_event_arriving_before_the_session_is_armed_is_an_exception() {
    let store = FakeStore::new();
    let mut state = armed_session();
    state.session.complete().expect("complete");

    let out = ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 1_010_000))
        .await
        .expect("ingest");

    assert!(matches!(
        sole(&out),
        IngestOutcome::Interpreted {
            event: Interpreted::Exception { reason: ExceptionReason::SessionNotArmed, .. },
            ..
        }
    ));
    assert_eq!(store.raw_count(), 1, "the read is kept even when it cannot count");
}

#[tokio::test]
async fn an_exception_does_not_advance_the_interpreted_event_count() {
    let store = FakeStore::new();
    let mut state = armed_session();

    ingest_read(&mut state, &store, &read("rfid-99", "TAG-A1", 1, 1_010_000))
        .await
        .expect("ingest");

    // The count gates ARMED -> DRAFT (ADR 0001 D2); an exception is not progress.
    assert_eq!(state.session.interpreted_event_count, 0);
}

#[tokio::test]
async fn entry_then_exit_closes_the_station_run() {
    let store = FakeStore::new();
    let mut state = armed_session();

    ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 1_010_000))
        .await
        .expect("entry");
    ingest_read(&mut state, &store, &read("rfid-02", "TAG-A1", 2, 1_070_000))
        .await
        .expect("exit");

    let a = state.athlete("a1").unwrap();
    assert_eq!(a.runs.len(), 1);
    assert_eq!(a.runs[0].exited_at, Some(Instant(1_070_000)));
    assert_eq!(state.session.interpreted_event_count, 2);
}

#[tokio::test]
async fn official_timing_comes_from_detected_at_not_arrival() {
    let store = FakeStore::new();
    let mut state = armed_session();

    // detected_at 1_010_000, received_at 1_010_250 (CLAUDE.md 11, 17).
    ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 1_010_000))
        .await
        .expect("ingest");

    match &store.interpreted()[0].1 {
        Interpreted::Entered { at, .. } => assert_eq!(*at, Instant(1_010_000)),
        other => panic!("expected an entry, got {other:?}"),
    }
}

#[tokio::test]
async fn a_read_with_an_unusable_tag_is_stored_but_cannot_be_attributed() {
    let store = FakeStore::new();
    let mut state = armed_session();
    let mut event = read("rfid-01", "TAG-A1", 1, 1_010_000).into_event();
    event.tag_id = vec!["   ".to_string()];

    let out = ingest_read(&mut state, &store, &ReceivedEvent::new(event, 1_010_250))
        .await
        .expect("ingest");

    assert!(matches!(sole(&out), IngestOutcome::Unattributable));
    // Still durable: there is nothing to name it, but nothing is discarded either.
    assert_eq!(store.raw_count(), 1);
    assert!(state.pending_tags().is_empty(), "an unusable tag is not a check-in to-do");
}

#[tokio::test]
async fn an_interpretation_that_cannot_be_stored_does_not_advance_memory() {
    // The drift the previous phase documented. `domain::interpret` decided and applied in one
    // step, so a failed write left the in-memory athlete inside a station the event log had
    // never heard of, and the two disagreed until a restart. Deciding is pure, so it can
    // happen early; applying now waits for the write (CLAUDE.md 21, 29).
    let store = FakeStore::failing_interpreted();
    let mut state = armed_session();

    let err = ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 1_010_000))
        .await
        .expect_err("the interpretation write failed");
    assert!(matches!(err, IngestError::Interpretation { .. }));

    let athlete = state.athlete("a1").expect("on the roster");
    assert_eq!(athlete.status, domain::AthleteStatus::Ready, "memory must not run ahead");
    assert_eq!(athlete.station_state, domain::StationState::Outside);
    assert!(athlete.runs.is_empty());
    assert_eq!(athlete.started_at, None);
    assert_eq!(state.session.interpreted_event_count, 0);
    // The raw read is still durable, which is the guarantee that matters (CLAUDE.md 31).
    assert_eq!(store.raw_count(), 1);
}

#[tokio::test]
async fn a_failed_exception_write_does_not_advance_the_inbox_badge() {
    // Same rule on the exception path: an unstored exception must not be counted, or the
    // badge would claim work the operator cannot find (ADR 0001 D4).
    let store = FakeStore::failing_interpreted();
    let mut state = armed_session();

    ingest_read(&mut state, &store, &read("rfid-99", "TAG-A1", 1, 1_010_000))
        .await
        .expect_err("the interpretation write failed");

    assert_eq!(state.exception_count, 0);
}

#[tokio::test]
async fn memory_and_the_log_agree_after_a_recovered_write_failure() {
    // What the ordering buys: replaying the log rebuilds exactly the state memory holds, so
    // the failed read is missing from both rather than from one (CLAUDE.md 21).
    let store = FakeStore::new();
    let mut state = armed_session();
    ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 1_010_000))
        .await
        .expect("stored");

    let replayed = domain::replay(
        "a1",
        "CHEN YU-TING",
        store.interpreted().iter().map(|(_, e)| e).collect::<Vec<_>>(),
    );
    let live = state.athlete("a1").expect("on the roster");
    assert_eq!(replayed.status, live.status);
    assert_eq!(replayed.station_state, live.station_state);
    assert_eq!(replayed.current_station, live.current_station);
    assert_eq!(replayed.started_at, live.started_at);
    assert_eq!(replayed.runs.len(), live.runs.len());
}

// --- UHF inventory rounds (ADR 0014) -------------------------------------------------

#[tokio::test]
async fn every_tag_in_a_round_is_interpreted_for_its_own_athlete() {
    // Two people at the antenna in the same instant. They are different athletes, so the
    // round has to fan out; folding it into one read would silently lose the second.
    let store = FakeStore::new();
    let mut state = armed_session();
    state
        .bindings
        .bind("s1", &TagId::parse("TAG-B2").unwrap(), "a2", CLASS_START)
        .expect("bind the second band");
    state.athletes.push(AthleteState::ready("a2", "LIN WEI"));

    let out = ingest_read(&mut state, &store, &round("rfid-01", &["TAG-A1", "TAG-B2"], 1, 1_010_000))
        .await
        .expect("ingest");

    assert_eq!(out.outcomes.len(), 2, "one outcome per tag, in the order they were read");
    let who: Vec<&str> = out
        .outcomes
        .iter()
        .map(|o| match o {
            IngestOutcome::Interpreted { athlete_id, .. } => athlete_id.as_str(),
            other => panic!("expected an interpretation, got {other:?}"),
        })
        .collect();
    assert_eq!(who, ["a1", "a2"]);

    // Both entered SKIERG, both off the same detection instant (CLAUDE.md 11, 17).
    for id in ["a1", "a2"] {
        assert_eq!(state.athlete(id).unwrap().current_station.as_deref(), Some("SKIERG"));
        assert_eq!(state.athlete(id).unwrap().started_at, Some(Instant(1_010_000)));
    }
    assert_eq!(store.raw_count(), 2, "one raw row per tag");
}

#[tokio::test]
async fn a_round_earns_one_ack_after_every_tag_in_it_is_durable() {
    // The ACK addresses the round, not the tag: `device_id + boot_id + sequence` is what the
    // edge journals and what it releases (CLAUDE.md 15, 16).
    let store = FakeStore::new();
    let mut state = armed_session();
    let event = round("rfid-01", &["TAG-A1", "TAG-NOBODY"], 3, 1_010_000);

    let out = ingest_read(&mut state, &store, &event).await.expect("ingest");

    assert_eq!(out.ack.payload().sequence, 3);
    assert_eq!(out.ack.payload().status, AckStatus::Stored);
    assert_eq!(store.raw_count(), 2, "the unbound tag is stored too, not dropped");
    // An unbound band in the round does not hold up the bound one beside it.
    assert!(matches!(out.outcomes[0], IngestOutcome::Interpreted { .. }));
    assert!(matches!(out.outcomes[1], IngestOutcome::PendingBinding { .. }));
}

#[tokio::test]
async fn a_redelivered_round_is_acked_but_interpreted_once() {
    let store = FakeStore::new();
    let mut state = armed_session();
    let event = round("rfid-01", &["TAG-A1"], 4, 1_010_000);

    ingest_read(&mut state, &store, &event).await.expect("first");
    let second = ingest_read(&mut state, &store, &event).await.expect("redelivery");

    assert!(matches!(sole(&second), IngestOutcome::Duplicate));
    assert_eq!(second.ack.payload().status, AckStatus::Duplicate);
    assert_eq!(store.raw_count(), 1);
    assert_eq!(state.athlete("a1").unwrap().runs.len(), 1);
}

#[tokio::test]
async fn a_resent_round_that_was_only_half_stored_finishes_the_job() {
    // A commit that failed part way through leaves rows but no ACK, so the edge resends the
    // whole round. The tags already there must not be reinterpreted, and the missing one
    // must still land (CLAUDE.md 16, 31).
    let store = FakeStore::new();
    let mut state = armed_session();
    // The first tag of the round arrives on its own -- what a partial commit leaves behind.
    ingest_read(&mut state, &store, &round("rfid-01", &["TAG-A1"], 5, 1_010_000))
        .await
        .expect("the half that got through");
    assert_eq!(store.raw_count(), 1);

    let out = ingest_read(&mut state, &store, &round("rfid-01", &["TAG-A1", "TAG-NOBODY"], 5, 1_010_000))
        .await
        .expect("the resend");

    assert_eq!(out.ack.payload().status, AckStatus::Stored, "the round was not fully stored");
    assert_eq!(store.raw_count(), 2, "only the missing tag is inserted");
}
