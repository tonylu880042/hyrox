//! Setting up a venue's readers, and the settings screen behind it (M6).
//!
//! Installing a venue means walking the floor with a band, tapping each antenna, and
//! telling the hub what that antenna is. The hub already stores every read it could not
//! attribute (CLAUDE.md 31: nothing is dropped), so the readers that need setting up are
//! the ones it has heard from and does not recognise. That list is derived, never typed:
//! a MAC address copied off a sticker by hand is a typo waiting to happen.

mod support;

use application::{
    register_reader, unregistered_readers, LiveSession, OperatorCommand, OperatorError,
};
use domain::{
    Instant, ReaderKey, ReaderMode, ReaderRegistration, Session, SessionConfig, SessionMode,
};
use support::FakeStore;

const NOW: Instant = Instant(3_000_000);

fn session() -> LiveSession {
    LiveSession::new(
        Session::new_draft("s1", "OPEN 2026", SessionMode::Competition),
        SessionConfig::new("s1"),
        Instant(1_000_000),
    )
}

fn key(reader: &str) -> ReaderKey {
    ReaderKey::parse("a4cf128b3d91", reader).expect("a canonical key")
}

#[tokio::test]
async fn a_reader_the_hub_has_heard_from_and_does_not_know_needs_setting_up() {
    let store = FakeStore::new().with_reader_keys_seen(vec![(
        "a4cf128b3d91".to_string(),
        "rfid-01".to_string(),
        Instant(2_000_000),
        3,
    )]);
    let state = session();

    let found = unregistered_readers(&state.readers, &store)
        .await
        .expect("a list");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].device_id, "a4cf128b3d91");
    assert_eq!(found[0].reader_id, "rfid-01");
    assert_eq!(
        found[0].reads, 3,
        "how many times it has been tapped, so a helper can tell \
                                   which antenna they just touched"
    );
    assert_eq!(found[0].last_seen, Instant(2_000_000));
}

#[tokio::test]
async fn a_reader_that_is_already_set_up_is_not_offered_again() {
    let store = FakeStore::new().with_reader_keys_seen(vec![(
        "a4cf128b3d91".to_string(),
        "rfid-01".to_string(),
        Instant(2_000_000),
        3,
    )]);
    let mut state = session();
    state.readers.register(ReaderRegistration::new(
        key("rfid-01"),
        "SKIERG",
        ReaderMode::Entry,
    ));

    let found = unregistered_readers(&state.readers, &store)
        .await
        .expect("a list");

    assert!(
        found.is_empty(),
        "it has a station; there is nothing left to decide"
    );
}

/// The list is what makes installation possible without typing MAC addresses, so the
/// order has to match the order somebody walks the floor in: most recently tapped first.
#[tokio::test]
async fn the_most_recently_tapped_reader_is_first() {
    let store = FakeStore::new().with_reader_keys_seen(vec![
        (
            "a4cf128b3d91".to_string(),
            "rfid-01".to_string(),
            Instant(2_000_000),
            1,
        ),
        (
            "a4cf128b3d91".to_string(),
            "rfid-09".to_string(),
            Instant(2_900_000),
            1,
        ),
        (
            "a4cf128b3d91".to_string(),
            "rfid-04".to_string(),
            Instant(2_500_000),
            1,
        ),
    ]);

    let found = unregistered_readers(&session().readers, &store)
        .await
        .expect("a list");

    let ids: Vec<&str> = found.iter().map(|r| r.reader_id.as_str()).collect();
    assert_eq!(ids, ["rfid-09", "rfid-04", "rfid-01"]);
}

/// Assigning one is the existing use case; what matters here is that doing so takes it off
/// the list, because that is the loop a helper is in: tap, assign, tap the next one.
#[tokio::test]
async fn assigning_a_station_takes_the_reader_off_the_list() {
    let store = FakeStore::new().with_reader_keys_seen(vec![(
        "a4cf128b3d91".to_string(),
        "rfid-01".to_string(),
        Instant(2_000_000),
        3,
    )]);
    let mut state = session();

    register_reader(
        &mut state,
        &store,
        &ReaderRegistration::new(key("rfid-01"), "SKIERG", ReaderMode::Entry),
        &OperatorCommand::new("SETUP TABLET", NOW),
    )
    .await
    .expect("registered");

    assert!(unregistered_readers(&state.readers, &store)
        .await
        .expect("a list")
        .is_empty());
}

// --- taking one off the wall (ADR 0007 §7, amended) -----------------------------------

#[tokio::test]
async fn a_reader_can_be_unregistered_and_the_store_is_told() {
    let store = FakeStore::new();
    let mut state = session();
    register_reader(
        &mut state,
        &store,
        &ReaderRegistration::new(key("rfid-01"), "SKIERG", ReaderMode::Entry),
        &OperatorCommand::new("SETUP TABLET", NOW),
    )
    .await
    .expect("registered");

    application::unregister_reader(
        &mut state,
        &store,
        &key("rfid-01"),
        &OperatorCommand::new("SETUP TABLET", NOW).with_reason("這支拆掉了"),
    )
    .await
    .expect("removed");

    assert!(state.readers.resolve(&key("rfid-01")).is_err());
    assert_eq!(
        store.deleted_readers(),
        vec![("a4cf128b3d91".to_string(), "rfid-01".to_string())]
    );
}

/// Reads from it stop being attributed, so somebody has to be able to say why. Same rule as
/// voiding an interpretation (CLAUDE.md 20).
#[tokio::test]
async fn unregistering_a_reader_needs_a_reason_and_is_audited() {
    let store = FakeStore::new();
    let mut state = session();
    register_reader(
        &mut state,
        &store,
        &ReaderRegistration::new(key("rfid-01"), "SKIERG", ReaderMode::Entry),
        &OperatorCommand::new("SETUP TABLET", NOW),
    )
    .await
    .expect("registered");

    let refused = application::unregister_reader(
        &mut state,
        &store,
        &key("rfid-01"),
        &OperatorCommand::new("SETUP TABLET", NOW),
    )
    .await;
    assert!(matches!(refused, Err(OperatorError::ReasonRequired)));
    assert!(
        state.readers.resolve(&key("rfid-01")).is_ok(),
        "nothing was removed"
    );

    application::unregister_reader(
        &mut state,
        &store,
        &key("rfid-01"),
        &OperatorCommand::new("SETUP TABLET", NOW).with_reason("這支拆掉了"),
    )
    .await
    .expect("removed");

    let entry = store.audits().pop().expect("an audit record");
    assert_eq!(entry.action, "READER_REMOVE");
    assert_eq!(entry.reason.as_deref(), Some("這支拆掉了"));
    assert_eq!(
        entry.before.as_deref(),
        Some("SKIERG ENTRY"),
        "what it used to mean"
    );
}

#[tokio::test]
async fn unregistering_a_reader_that_was_never_registered_is_a_clear_no() {
    let store = FakeStore::new();
    let mut state = session();

    let answer = application::unregister_reader(
        &mut state,
        &store,
        &key("rfid-99"),
        &OperatorCommand::new("SETUP TABLET", NOW).with_reason("手滑"),
    )
    .await;

    assert!(matches!(answer, Err(OperatorError::UnknownReader { .. })));
}
