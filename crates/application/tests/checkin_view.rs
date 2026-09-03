//! The read model behind `/checkin` (ADR 0001 D3).

use application::{checkin_view, LiveSession};
use domain::{AthleteState, BindingLedger, Instant, Session, SessionConfig, SessionMode, TagId};

const START: Instant = Instant(1_000_000);

fn tag(raw: &str) -> TagId {
    TagId::parse(raw).expect("a usable tag id")
}

fn session() -> LiveSession {
    let mut s = Session::new_draft("s1", "THU 19:00", SessionMode::Training);
    s.mark_ready().expect("arm");
    s.start().expect("arm");
    LiveSession::new(s, SessionConfig::new("s1"), START).with_athletes(vec![
        AthleteState::ready("a1", "CHEN YU-TING"),
        AthleteState::ready("a2", "LIN WEI"),
    ])
}

#[test]
fn an_unclaimed_tag_is_listed_in_the_order_it_was_read() {
    let mut state = session();
    state.note_pending_tag(tag("E28011700000AAAA"));
    state.note_pending_tag(tag("E28011700000BBBB"));

    let view = checkin_view(&state);

    assert_eq!(view.pending, ["E28011700000AAAA", "E28011700000BBBB"]);
}

#[test]
fn the_roster_says_who_already_has_a_band() {
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a2", START)
        .expect("bind");
    let state = session().with_bindings(ledger);

    let view = checkin_view(&state);

    assert_eq!(view.athletes.len(), 2);
    assert_eq!(view.athletes[0].bib, 1);
    assert_eq!(view.athletes[0].athlete_id, "a1");
    // Nobody handed them a band yet: the check-in screen's whole work list.
    assert_eq!(view.athletes[0].tag_id, None);
    assert_eq!(view.athletes[1].name, "LIN WEI");
    assert_eq!(view.athletes[1].tag_id.as_deref(), Some("E28011700000AAAA"));
}

/// A band from a class that ran earlier belongs to that class, not to this roster. Showing
/// it against an athlete here would claim a binding this session does not hold.
#[test]
fn a_band_bound_in_another_session_is_not_shown_against_this_roster() {
    let mut ledger = BindingLedger::new();
    ledger
        .bind("other-session", &tag("E28011700000CCCC"), "a1", START)
        .expect("bind");
    let state = session().with_bindings(ledger);

    let view = checkin_view(&state);

    assert_eq!(view.athletes[0].tag_id, None);
}

#[test]
fn a_session_where_nobody_has_scanned_anything_lists_no_pending_tags() {
    let view = checkin_view(&session());

    assert!(view.pending.is_empty());
    assert_eq!(view.athletes.len(), 2);
}
