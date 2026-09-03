//! The entry code: the six characters a walk-in entrant is known by.
//!
//! A mock race takes entries from people the gym has never seen (ADR 0010). They need an
//! identity they can carry on their own phone, read out at the desk when the QR will not
//! scan, and type in afterwards to find their result. Six characters is that identity, and
//! it is the athlete id itself rather than a second number to keep in step.
//!
//! Everything here is pure: generation lives in the application layer, because a domain
//! that reaches for randomness cannot be replayed (CLAUDE.md 21, 29).

use domain::*;

#[test]
fn a_code_is_six_characters_from_the_unambiguous_alphabet() {
    let code = EntryCode::encode(0);
    assert_eq!(code.as_str().len(), 6);
    assert!(code
        .as_str()
        .chars()
        .all(|c| EntryCode::ALPHABET.contains(c)));
}

#[test]
fn the_alphabet_leaves_out_the_letters_people_misread() {
    // Crockford's rule: no I, L, O or U. The first three are misread as 1 and 0 on a phone
    // screen at arm's length; U is dropped so no code spells anything unfortunate.
    for c in ['I', 'L', 'O', 'U'] {
        assert!(
            !EntryCode::ALPHABET.contains(c),
            "{c} should not be in the alphabet"
        );
    }
}

#[test]
fn different_values_give_different_codes() {
    let a = EntryCode::encode(1);
    let b = EntryCode::encode(2);
    let far = EntryCode::encode(987_654_321);
    assert_ne!(a, b);
    assert_ne!(a, far);
    assert_ne!(b, far);
}

#[test]
fn encoding_wraps_rather_than_overflowing() {
    // Six characters hold 30 bits. A larger number must still produce a code, because the
    // caller supplies whatever its source of randomness gave it.
    let code = EntryCode::encode(u64::MAX);
    assert_eq!(code.as_str().len(), 6);
}

#[test]
fn a_code_round_trips_through_parsing() {
    let code = EntryCode::encode(123_456);
    assert_eq!(EntryCode::parse(code.as_str()).expect("valid"), code);
}

#[test]
fn parsing_forgives_how_a_human_types_it() {
    let code = EntryCode::parse("K7QD2M").expect("valid");
    for typed in ["k7qd2m", " K7QD2M ", "K7Q-D2M", "K7Q D2M"] {
        assert_eq!(EntryCode::parse(typed).expect("valid"), code, "{typed:?}");
    }
}

#[test]
fn parsing_maps_the_characters_people_substitute() {
    // Someone reading a code off a phone types O for 0 and I or L for 1. Mapping them is
    // the whole reason the alphabet has holes in it.
    assert_eq!(EntryCode::parse("0AAAAA"), EntryCode::parse("OAAAAA"));
    assert_eq!(EntryCode::parse("1AAAAA"), EntryCode::parse("IAAAAA"));
    assert_eq!(EntryCode::parse("1AAAAA"), EntryCode::parse("LAAAAA"));
}

#[test]
fn parsing_refuses_what_is_not_a_code() {
    for bad in ["", "K7QD2", "K7QD2MX", "K7QD2!", "陳怡君"] {
        assert!(EntryCode::parse(bad).is_err(), "{bad:?} should not parse");
    }
}

#[test]
fn a_parsed_code_is_stored_and_shown_in_upper_case() {
    assert_eq!(
        EntryCode::parse("k7qd2m").expect("valid").as_str(),
        "K7QD2M"
    );
}

#[test]
fn a_member_id_is_not_an_entry_code() {
    // Members keep their 健身管 identity (ADR 0010); only walk-ins are issued a code, so
    // the two must stay distinguishable when one arrives in a URL.
    assert!(EntryCode::parse("M-1042").is_err());
}
