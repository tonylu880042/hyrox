//! Putting people on the roster, members and walk-ins alike (ADR 0010).

mod support;

use application::{enter, Entrant, LiveSession, OperatorCommand, OperatorError};
use domain::{Instant, MemberRef, MembershipStatus, Session, SessionConfig, SessionMode};
use support::FakeStore;

const NOW: Instant = Instant(2_000_000);

fn session() -> LiveSession {
    LiveSession::new(
        Session::new_draft("s1", "OPEN 2026", SessionMode::Competition),
        SessionConfig::new("s1"),
        Instant(1_000_000),
    )
}

fn desk() -> OperatorCommand {
    OperatorCommand::new("DOOR TABLET", NOW)
}

/// The whole point of ADR 0010: somebody the gym has never seen can be entered with a name
/// and nothing else.
#[tokio::test]
async fn a_walk_in_needs_only_a_name() {
    let store = FakeStore::new();
    let mut state = session();

    let id = enter(&mut state, &store, Entrant::walk_in("陳小明"), &desk())
        .await
        .expect("entered");

    let athlete = state.athlete(&id).expect("on the roster");
    assert_eq!(athlete.display_name, "陳小明");
    assert_eq!(state.athletes.len(), 1);
}

#[tokio::test]
async fn a_walk_in_is_recorded_as_having_no_member_reference() {
    let store = FakeStore::new();
    let mut state = session();

    enter(&mut state, &store, Entrant::walk_in("陳小明"), &desk()).await.unwrap();

    let saved = store.saved_athletes().pop().expect("a stored row");
    assert_eq!(saved.member_id, None, "a walk-in has no member id, and that is the record");
}

#[tokio::test]
async fn a_member_keeps_their_member_id_as_their_athlete_id() {
    let store = FakeStore::new();
    let mut state = session();
    let member = MemberRef {
        member_id: "M-4417".into(),
        display_name: "林佳蓉".into(),
        status: MembershipStatus::Active,
        gender: None,
        age: None,
        photo_url: None,
        height_cm: None,
        weight_kg: None,
    };

    let id = enter(&mut state, &store, Entrant::member(&member), &desk()).await.unwrap();

    assert_eq!(id, "M-4417", "an existing member is still keyed by their member id");
    let saved = store.saved_athletes().pop().expect("a stored row");
    assert_eq!(saved.member_id.as_deref(), Some("M-4417"));
}

#[tokio::test]
async fn bibs_are_handed_out_in_order_when_nobody_asks_for_one() {
    let store = FakeStore::new();
    let mut state = session();

    for name in ["A", "B", "C"] {
        enter(&mut state, &store, Entrant::walk_in(name), &desk()).await.unwrap();
    }

    let bibs: Vec<i64> = store.saved_athletes().into_iter().map(|a| a.bib).collect();
    assert_eq!(bibs, [1, 2, 3]);
}

/// Competition bibs are printed in advance, so the door has to be able to say which one.
#[tokio::test]
async fn a_requested_bib_is_honoured() {
    let store = FakeStore::new();
    let mut state = session();

    enter(&mut state, &store, Entrant::walk_in("A").with_bib(42), &desk()).await.unwrap();

    assert_eq!(store.saved_athletes()[0].bib, 42);
}

#[tokio::test]
async fn a_bib_already_on_somebody_else_is_refused() {
    let store = FakeStore::new();
    let mut state = session();
    enter(&mut state, &store, Entrant::walk_in("A").with_bib(7), &desk()).await.unwrap();

    let err = enter(&mut state, &store, Entrant::walk_in("B").with_bib(7), &desk())
        .await
        .expect_err("two vests with the same number is a timing error waiting to happen");

    assert!(matches!(err, OperatorError::BibTaken(7)));
    assert_eq!(state.athletes.len(), 1);
}

/// The next free bib skips the ones already handed out, rather than colliding with them.
#[tokio::test]
async fn automatic_bibs_step_over_the_ones_already_taken() {
    let store = FakeStore::new();
    let mut state = session();
    enter(&mut state, &store, Entrant::walk_in("A").with_bib(1), &desk()).await.unwrap();
    enter(&mut state, &store, Entrant::walk_in("B").with_bib(2), &desk()).await.unwrap();

    enter(&mut state, &store, Entrant::walk_in("C"), &desk()).await.unwrap();

    assert_eq!(store.saved_athletes()[2].bib, 3);
}

#[tokio::test]
async fn an_entrant_with_no_name_is_refused() {
    let store = FakeStore::new();
    let mut state = session();

    let err = enter(&mut state, &store, Entrant::walk_in("   "), &desk())
        .await
        .expect_err("a blank name on a live screen names nobody");

    assert!(matches!(err, OperatorError::NameRequired));
}

/// The door tablet double-tapped. One person, one roster line -- and the same bib back,
/// so the helper is not told a different number the second time.
#[tokio::test]
async fn admitting_the_same_member_twice_is_idempotent() {
    let store = FakeStore::new();
    let mut state = session();
    let member = MemberRef {
        member_id: "M-1".into(),
        display_name: "林佳蓉".into(),
        status: MembershipStatus::Active,
        gender: None,
        age: None,
        photo_url: None,
        height_cm: None,
        weight_kg: None,
    };

    let first = enter(&mut state, &store, Entrant::member(&member), &desk()).await.unwrap();
    let again = enter(&mut state, &store, Entrant::member(&member), &desk()).await.unwrap();

    assert_eq!(first, again);
    assert_eq!(state.athletes.len(), 1);
}

/// Membership status never gates timing (confirmed 2026-08-27). An expired member walking
/// in on a Saturday is still somebody the clock should run for.
#[tokio::test]
async fn an_expired_membership_does_not_stop_somebody_entering() {
    let store = FakeStore::new();
    let mut state = session();
    let member = MemberRef {
        member_id: "M-9".into(),
        display_name: "王大衛".into(),
        status: MembershipStatus::Expired,
        gender: None,
        age: None,
        photo_url: None,
        height_cm: None,
        weight_kg: None,
    };

    enter(&mut state, &store, Entrant::member(&member), &desk()).await.expect("entered");

    assert_eq!(state.athletes.len(), 1);
}

#[tokio::test]
async fn entering_somebody_is_audited() {
    let store = FakeStore::new();
    let mut state = session();

    enter(&mut state, &store, Entrant::walk_in("陳小明"), &desk()).await.unwrap();

    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "ATHLETE_ENTER");
    assert_eq!(audit.operator, "DOOR TABLET");
}

/// The number on the vest, not the roster position. Before ADR 0010 those were the same
/// thing; now the door can assign one, and a leaderboard showing "2" for somebody wearing
/// 7 is worse than useless on a competition floor.
#[tokio::test]
async fn the_read_models_show_the_bib_that_was_assigned() {
    let store = FakeStore::new();
    let mut state = session();
    enter(&mut state, &store, Entrant::walk_in("A"), &desk()).await.unwrap();
    enter(&mut state, &store, Entrant::walk_in("B").with_bib(7), &desk()).await.unwrap();

    let view = application::checkin_view(&state);
    let bibs: Vec<usize> = view.athletes.iter().map(|a| a.bib).collect();
    assert_eq!(bibs, [1, 7]);

    let results = application::live_results(&state);
    assert_eq!(results.rows.iter().map(|r| r.bib).collect::<Vec<_>>(), [1, 7]);
}
