//! Tag presence / re-arm suppression (CLAUDE.md 14, 24, 25).
//!
//! The contract is emphatic that suppression is **not** a fixed window and in particular
//! not a 60-second one. It is presence-based: first sight emits, continued presence is
//! suppressed at RF level, and only an absence longer than the reader's configured
//! `absent_timeout` re-arms the tag.

use simulator::{AbsentTimeout, PresenceDecision, TagPresence};

const TAG: &str = "E280117000001234";

fn presence(timeout_ms: i64) -> TagPresence {
    TagPresence::new(AbsentTimeout::from_millis(timeout_ms).unwrap())
}

#[test]
fn the_default_timeout_sits_in_the_documented_range() {
    // CLAUDE.md 14 gives a target of 3–5 s and requires the value to be configurable and
    // venue-validated, so this is a named default, not a rule.
    let ms = AbsentTimeout::default().millis();
    assert!((3_000..=5_000).contains(&ms), "default was {ms} ms");
}

#[test]
fn a_timeout_must_be_a_positive_duration() {
    assert!(AbsentTimeout::from_millis(0).is_err());
    assert!(AbsentTimeout::from_millis(-1).is_err());
}

#[test]
fn first_sight_emits() {
    let mut p = presence(4_000);
    assert_eq!(p.observe(TAG, 0), PresenceDecision::Emit);
}

#[test]
fn continuous_presence_suppresses_every_repeat_read() {
    // An athlete standing at the reader is read many times per second. Exactly one event
    // may leave the device.
    let mut p = presence(4_000);
    assert_eq!(p.observe(TAG, 0), PresenceDecision::Emit);
    for t in (100..3_000).step_by(100) {
        assert_eq!(p.observe(TAG, t), PresenceDecision::Suppressed, "at {t} ms");
    }
}

#[test]
fn absence_longer_than_the_timeout_rearms_the_tag() {
    let mut p = presence(4_000);
    assert_eq!(p.observe(TAG, 0), PresenceDecision::Emit);
    assert_eq!(p.observe(TAG, 4_001), PresenceDecision::Emit, "re-armed");
    assert_eq!(p.observe(TAG, 4_100), PresenceDecision::Suppressed);
}

#[test]
fn absence_exactly_equal_to_the_timeout_does_not_yet_rearm() {
    // The boundary is pinned so a venue tuning session changes behaviour by changing the
    // configured value, not by discovering an off-by-one.
    let mut p = presence(4_000);
    p.observe(TAG, 0);
    assert_eq!(p.observe(TAG, 4_000), PresenceDecision::Suppressed);
}

#[test]
fn presence_is_extended_by_every_read_not_by_the_first_one() {
    // Reads 3 s apart under a 4 s timeout are one continuous presence, however long it
    // lasts — the tag never silently re-arms mid-station.
    let mut p = presence(4_000);
    assert_eq!(p.observe(TAG, 0), PresenceDecision::Emit);
    for t in (3_000..=90_000).step_by(3_000) {
        assert_eq!(p.observe(TAG, t), PresenceDecision::Suppressed, "at {t} ms");
    }
}

#[test]
fn there_is_no_sixty_second_window() {
    // The rule CLAUDE.md 14 calls out by name. A tag visible for two minutes emits once;
    // it does not re-emit at 60 s, and it does re-emit after a 5 s gap.
    let mut p = presence(4_000);
    let mut emits = 0;
    for t in (0..=120_000).step_by(500) {
        if p.observe(TAG, t) == PresenceDecision::Emit {
            emits += 1;
        }
    }
    assert_eq!(emits, 1, "continuous presence must emit exactly once");
    assert_eq!(p.observe(TAG, 125_000), PresenceDecision::Emit, "5 s gap re-arms");
}

#[test]
fn station_duration_is_not_a_suppression_window() {
    // A 90 s SkiErg does not mean 90 s of suppression: the athlete leaving the antenna for
    // 5 s and returning is a real new read, and the device must send it.
    let mut p = presence(4_000);
    p.observe(TAG, 0);
    assert_eq!(p.observe(TAG, 5_000), PresenceDecision::Emit);
}

#[test]
fn each_tag_is_tracked_independently() {
    let mut p = presence(4_000);
    let other = "E280117000009999";
    assert_eq!(p.observe(TAG, 0), PresenceDecision::Emit);
    assert_eq!(p.observe(other, 100), PresenceDecision::Emit, "a different athlete");
    assert_eq!(p.observe(TAG, 200), PresenceDecision::Suppressed);
    assert_eq!(p.observe(other, 300), PresenceDecision::Suppressed);
}

#[test]
fn the_timeout_is_per_reader() {
    // CLAUDE.md 14: preferably configurable per Reader. A doorway antenna and a station
    // antenna do not see the same dwell behaviour.
    let mut doorway = presence(3_000);
    let mut station = presence(5_000);
    doorway.observe(TAG, 0);
    station.observe(TAG, 0);

    assert_eq!(doorway.observe(TAG, 4_000), PresenceDecision::Emit);
    assert_eq!(station.observe(TAG, 4_000), PresenceDecision::Suppressed);
}

#[test]
fn a_reboot_clears_presence_because_rf_state_is_volatile() {
    // Presence lives in RAM on the ESP32. After a reboot the device has no idea what it
    // was looking at, so the next read is a first sight.
    let mut p = presence(4_000);
    p.observe(TAG, 0);
    p.forget_all();
    assert_eq!(p.observe(TAG, 100), PresenceDecision::Emit);
}
