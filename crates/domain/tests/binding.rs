//! Member reference and RFID tag binding (CLAUDE.md 7.1, 7.2; ADR 0001 D3).
//!
//! The invariants under test are the ones D3 states outright: one tag binds to one
//! athlete at a time, one athlete holds one tag per session, and swapping a band closes
//! the old binding instead of overwriting it so the audit trail survives (CLAUDE.md 20).

use domain::*;

const T0: i64 = 1_787_734_800_000;

fn at(offset_ms: i64) -> Instant {
    Instant(T0 + offset_ms)
}

fn tag(raw: &str) -> TagId {
    TagId::parse(raw).expect("valid tag id")
}

// --- member reference (CLAUDE.md 7.1) -------------------------------------------------

#[test]
fn member_reference_holds_the_identity_pair_and_the_optional_profile() {
    let m = MemberRef::new("M-1042", "Chen Wei", MembershipStatus::Active);
    assert_eq!(m.member_id, "M-1042");
    assert_eq!(m.display_name, "Chen Wei");
    assert_eq!(m.status, MembershipStatus::Active);
    // Everything the gym app may or may not hold starts absent, never guessed.
    assert_eq!(m.gender, None);
    assert_eq!(m.age, None);
    assert_eq!(m.photo_url, None);
}

#[test]
fn membership_status_is_carried_but_never_gates_timing() {
    // Settled with the user 2026-08-27: if 健身管 returns the member, they may be timed.
    // The status is displayed, not enforced -- there is deliberately no predicate to gate on.
    for status in [
        MembershipStatus::Active,
        MembershipStatus::Suspended,
        MembershipStatus::Expired,
        MembershipStatus::Unknown,
    ] {
        let m = MemberRef::new("M-1042", "Chen Wei", status);
        assert_eq!(
            m.status, status,
            "the source system's answer is carried verbatim"
        );
    }
}

#[test]
fn an_unmapped_membership_status_stays_unknown_rather_than_defaulting_to_active() {
    // The 健身管 contract is still unresolved (CLAUDE.md 28). An unmapped value must stay
    // visibly unknown so nobody later reads it as a confirmed "Active".
    let m = MemberRef::new("M-1042", "Chen Wei", MembershipStatus::Unknown);
    assert_eq!(m.status, MembershipStatus::Unknown);
    assert_ne!(m.status, MembershipStatus::Active);
}

// --- tag id ---------------------------------------------------------------------------

#[test]
fn tag_ids_are_normalised_so_case_cannot_split_one_tag_into_two() {
    assert_eq!(tag("e280117000001234").as_str(), "E280117000001234");
    assert_eq!(tag(" E280117000001234 "), tag("E280117000001234"));
    assert_eq!(TagId::parse("   "), Err(TagIdError::Empty));
}

// --- binding invariants (ADR 0001 D3) -------------------------------------------------

#[test]
fn binding_resolves_a_tag_to_an_athlete_within_a_session() {
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();

    assert_eq!(
        ledger.athlete_for_tag("s1", &tag("E28011700000AAAA")),
        Some("a1")
    );
    assert_eq!(
        ledger.tag_for_athlete("s1", "a1"),
        Some(&tag("E28011700000AAAA"))
    );
}

#[test]
fn a_binding_is_traceable_by_session() {
    // CLAUDE.md 7.2: the same band may be handed to someone else next class.
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();
    ledger
        .unbind("s1", &tag("E28011700000AAAA"), at(1_000))
        .unwrap();
    ledger
        .bind("s2", &tag("E28011700000AAAA"), "a2", at(2_000))
        .unwrap();

    assert_eq!(
        ledger.athlete_for_tag("s2", &tag("E28011700000AAAA")),
        Some("a2")
    );
    assert_eq!(
        ledger.athlete_for_tag("s1", &tag("E28011700000AAAA")),
        None,
        "s1 is closed"
    );
}

#[test]
fn one_tag_cannot_be_bound_to_two_athletes_at_once() {
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();

    assert_eq!(
        ledger.bind("s1", &tag("E28011700000AAAA"), "a2", at(1_000)),
        Err(BindingError::TagAlreadyBound {
            session_id: "s1".into(),
            athlete_id: "a1".into()
        })
    );
    assert_eq!(
        ledger.athlete_for_tag("s1", &tag("E28011700000AAAA")),
        Some("a1")
    );
}

#[test]
fn a_tag_active_in_one_session_cannot_be_bound_in_another() {
    // Two classes running back to back must not both claim the same physical band.
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();

    assert_eq!(
        ledger.bind("s2", &tag("E28011700000AAAA"), "a2", at(1_000)),
        Err(BindingError::TagAlreadyBound {
            session_id: "s1".into(),
            athlete_id: "a1".into()
        })
    );
}

#[test]
fn one_athlete_holds_at_most_one_active_tag_per_session() {
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();

    assert_eq!(
        ledger.bind("s1", &tag("E28011700000BBBB"), "a1", at(1_000)),
        Err(BindingError::AthleteAlreadyBound {
            tag_id: tag("E28011700000AAAA")
        })
    );
    assert_eq!(
        ledger.tag_for_athlete("s1", "a1"),
        Some(&tag("E28011700000AAAA"))
    );
}

#[test]
fn rebinding_closes_the_old_binding_and_opens_a_new_one() {
    // Swapping a band is unbind + bind, two audit records (ADR D3).
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();
    ledger
        .rebind_athlete("s1", "a1", &tag("E28011700000BBBB"), at(5_000))
        .unwrap();

    assert_eq!(
        ledger.tag_for_athlete("s1", "a1"),
        Some(&tag("E28011700000BBBB"))
    );
    assert_eq!(ledger.athlete_for_tag("s1", &tag("E28011700000AAAA")), None);

    let history = ledger.history();
    assert_eq!(
        history.len(),
        2,
        "the old binding is closed, never overwritten"
    );
    assert_eq!(history[0].tag_id, tag("E28011700000AAAA"));
    assert_eq!(history[0].bound_at, at(0));
    assert_eq!(
        history[0].unbound_at,
        Some(at(5_000)),
        "the closed binding keeps its window"
    );
    assert_eq!(history[1].tag_id, tag("E28011700000BBBB"));
    assert_eq!(history[1].unbound_at, None);
}

#[test]
fn a_rejected_rebind_leaves_the_ledger_untouched() {
    // Validate before mutating: a half-applied swap would leave an athlete with no tag.
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();
    ledger
        .bind("s1", &tag("E28011700000BBBB"), "a2", at(1_000))
        .unwrap();

    let err = ledger.rebind_athlete("s1", "a1", &tag("E28011700000BBBB"), at(5_000));
    assert!(matches!(err, Err(BindingError::TagAlreadyBound { .. })));
    assert_eq!(
        ledger.tag_for_athlete("s1", "a1"),
        Some(&tag("E28011700000AAAA"))
    );
    assert_eq!(
        ledger.history().len(),
        2,
        "nothing may be appended or closed"
    );
}

#[test]
fn rebinding_an_athlete_who_has_no_tag_yet_simply_binds() {
    let mut ledger = BindingLedger::new();
    ledger
        .rebind_athlete("s1", "a1", &tag("E28011700000AAAA"), at(0))
        .unwrap();
    assert_eq!(
        ledger.tag_for_athlete("s1", "a1"),
        Some(&tag("E28011700000AAAA"))
    );
    assert_eq!(ledger.history().len(), 1);
}

#[test]
fn binding_the_same_pair_twice_is_idempotent() {
    // The check-in tablet may double-submit; a second identical bind must not fork history.
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(9_000))
        .unwrap();

    assert_eq!(ledger.history().len(), 1);
    assert_eq!(
        ledger.history()[0].bound_at,
        at(0),
        "the original bind time is authoritative"
    );
}

#[test]
fn unbinding_a_tag_that_is_not_bound_is_an_error_not_a_silent_success() {
    let mut ledger = BindingLedger::new();
    assert_eq!(
        ledger.unbind("s1", &tag("E28011700000AAAA"), at(0)),
        Err(BindingError::NotBound)
    );
}

#[test]
fn a_freed_tag_can_be_bound_again() {
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();
    ledger
        .unbind("s1", &tag("E28011700000AAAA"), at(1_000))
        .unwrap();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a2", at(2_000))
        .unwrap();

    assert_eq!(
        ledger.athlete_for_tag("s1", &tag("E28011700000AAAA")),
        Some("a2")
    );
    assert_eq!(ledger.history().len(), 2, "both bindings remain auditable");
}

#[test]
fn active_bindings_exclude_closed_ones() {
    let mut ledger = BindingLedger::new();
    ledger
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();
    ledger
        .bind("s1", &tag("E28011700000BBBB"), "a2", at(0))
        .unwrap();
    ledger
        .unbind("s1", &tag("E28011700000AAAA"), at(1_000))
        .unwrap();

    let active: Vec<&str> = ledger.active().map(|b| b.athlete_id.as_str()).collect();
    assert_eq!(active, ["a2"]);
    assert_eq!(ledger.history().len(), 2);
}

#[test]
fn an_unbound_tag_resolves_to_none_rather_than_panicking() {
    // Unknown tags route to /checkin as pending work, not to a crash (ADR D3).
    let ledger = BindingLedger::new();
    assert_eq!(ledger.athlete_for_tag("s1", &tag("E28011700000AAAA")), None);
    assert_eq!(ledger.tag_for_athlete("s1", "a1"), None);
}

#[test]
fn a_restored_ledger_keeps_its_closed_bindings() {
    // What a restart must not do is drop the history (CLAUDE.md 20, 21): a closed binding is
    // how "who was wearing this band at 10:15" stays answerable after the band moved on.
    let mut original = BindingLedger::new();
    original
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();
    original
        .rebind_athlete("s1", "a1", &tag("E28011700000BBBB"), at(5_000))
        .unwrap();

    let restored = BindingLedger::restore(original.history().to_vec());

    assert_eq!(
        restored.history().len(),
        2,
        "the closed row must come back too"
    );
    assert_eq!(
        restored.athlete_for_tag("s1", &tag("E28011700000BBBB")),
        Some("a1")
    );
    assert_eq!(
        restored.athlete_for_tag("s1", &tag("E28011700000AAAA")),
        None,
        "the band handed back belongs to nobody"
    );
    assert_eq!(restored.active().count(), 1);
}

#[test]
fn a_restored_ledger_still_enforces_one_band_one_wrist() {
    // The invariants are checked against whatever the ledger holds, so they survive the
    // rebuild rather than only applying to bindings made in this process.
    let mut original = BindingLedger::new();
    original
        .bind("s1", &tag("E28011700000AAAA"), "a1", at(0))
        .unwrap();

    let mut restored = BindingLedger::restore(original.history().to_vec());
    let err = restored
        .bind("s1", &tag("E28011700000AAAA"), "a2", at(1_000))
        .expect_err("the band is already on a wrist");

    assert!(matches!(err, BindingError::TagAlreadyBound { .. }));
}
