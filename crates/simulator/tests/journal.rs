//! The persistent edge journal (CLAUDE.md 18, 24).
//!
//! Rules under test: unacknowledged events are never deleted, a reboot preserves pending
//! events, ACK loss is safe, duplicate resend is safe, and space is reclaimed in batches
//! rather than erased on every ACK.

use contract::{DeviceId, EdgeEvent, EventId, ReaderId};
use transport::DeviceWarning;
use simulator::{AckResult, Journal, JournalConfig, JournalError};

fn event(boot_id: i64, sequence: i64) -> EdgeEvent {
    EdgeEvent {
        device_id: DeviceId::from_mac_str("a4cf128b3d91").unwrap(),
        reader_id: ReaderId::parse("rfid-02").unwrap(),
        boot_id,
        sequence,
        tag_id: vec!["E280117000001234".into()],
        detected_at: 1_787_734_821_382 + sequence,
        uptime_ms: sequence,
    }
}

fn journal(capacity: usize) -> Journal {
    Journal::new(JournalConfig::new(capacity, 80, 4).unwrap())
}

#[test]
fn the_default_capacity_meets_the_design_target() {
    // CLAUDE.md 18: minimum design target of 10,000 events per ESP32.
    assert!(JournalConfig::default().capacity >= 10_000);
}

#[test]
fn a_journalled_event_is_pending_until_it_is_acknowledged() {
    let mut j = journal(16);
    j.append(event(1, 1)).unwrap();
    assert_eq!(j.pending_count(), 1);

    assert_eq!(j.ack(&EventId::of(&event(1, 1))), AckResult::Released);
    assert_eq!(j.pending_count(), 0);
}

#[test]
fn pending_events_come_back_in_the_order_they_were_recorded() {
    let mut j = journal(16);
    for seq in 1..=5 {
        j.append(event(1, seq)).unwrap();
    }
    let seqs: Vec<i64> = j.pending().iter().map(|e| e.sequence).collect();
    assert_eq!(seqs, [1, 2, 3, 4, 5]);
}

#[test]
fn an_acknowledged_event_is_not_resent() {
    let mut j = journal(16);
    for seq in 1..=3 {
        j.append(event(1, seq)).unwrap();
    }
    j.ack(&EventId::of(&event(1, 2)));

    let seqs: Vec<i64> = j.pending().iter().map(|e| e.sequence).collect();
    assert_eq!(seqs, [1, 3], "only the gap is closed, the rest still owes an ACK");
}

#[test]
fn a_repeated_ack_is_safe() {
    // The hub acknowledges every delivery including duplicates, so the same ACK can arrive
    // more than once. It must never corrupt the cursor.
    let mut j = journal(16);
    j.append(event(1, 1)).unwrap();
    assert_eq!(j.ack(&EventId::of(&event(1, 1))), AckResult::Released);
    assert_eq!(j.ack(&EventId::of(&event(1, 1))), AckResult::AlreadyReleased);
    assert_eq!(j.pending_count(), 0);
}

#[test]
fn an_ack_for_something_never_recorded_is_ignored() {
    let mut j = journal(16);
    assert_eq!(j.ack(&EventId::of(&event(9, 9))), AckResult::Unknown);
}

#[test]
fn a_lost_ack_leaves_the_event_pending_for_resend() {
    // ACK loss is safe precisely because nothing is released without one (CLAUDE.md 18).
    let mut j = journal(16);
    j.append(event(1, 1)).unwrap();
    // ...ack never arrives...
    assert_eq!(j.pending_count(), 1);
    assert_eq!(j.pending()[0].sequence, 1);
}

#[test]
fn unacknowledged_events_are_never_dropped_to_make_room() {
    // The first priority of the whole system is not losing RFID events (CLAUDE.md 31), so
    // a full journal of unacked events is an error, never a silent overwrite.
    let mut j = journal(3);
    for seq in 1..=3 {
        j.append(event(1, seq)).unwrap();
    }
    assert!(matches!(j.append(event(1, 4)), Err(JournalError::Full { .. })));

    let seqs: Vec<i64> = j.pending().iter().map(|e| e.sequence).collect();
    assert_eq!(seqs, [1, 2, 3], "nothing was sacrificed");
}

#[test]
fn space_is_reclaimed_from_acknowledged_events_when_it_is_needed() {
    // "Do not erase flash after every ACK" — the acked entries stay put until the space is
    // actually wanted, and then a block goes at once (CLAUDE.md 18).
    let mut j = journal(4);
    for seq in 1..=4 {
        j.append(event(1, seq)).unwrap();
    }
    for seq in 1..=2 {
        j.ack(&EventId::of(&event(1, seq)));
    }
    assert_eq!(j.len(), 4, "acked entries are not erased eagerly");

    j.append(event(1, 5)).unwrap();
    assert!(j.len() <= 4);
    let seqs: Vec<i64> = j.pending().iter().map(|e| e.sequence).collect();
    assert_eq!(seqs, [3, 4, 5]);
}

#[test]
fn the_device_warns_before_it_runs_out_of_room() {
    // CLAUDE.md 18: near exhaustion, publish a critical device warning.
    let mut j = journal(4); // warn at 80%
    j.append(event(1, 1)).unwrap();
    assert_eq!(j.warning(), None);

    for seq in 2..=4 {
        j.append(event(1, seq)).unwrap();
    }
    assert_eq!(j.warning(), Some(DeviceWarning::JournalFull));
}

#[test]
fn a_journal_near_the_threshold_warns_without_being_full() {
    let mut j = Journal::new(JournalConfig::new(10, 80, 4).unwrap());
    for seq in 1..=8 {
        j.append(event(1, seq)).unwrap();
    }
    assert_eq!(j.warning(), Some(DeviceWarning::JournalNearlyFull));
}

#[test]
fn a_journal_config_must_be_usable() {
    assert!(JournalConfig::new(0, 80, 4).is_err(), "zero capacity");
    assert!(JournalConfig::new(10, 0, 4).is_err(), "warn threshold out of range");
    assert!(JournalConfig::new(10, 101, 4).is_err());
    assert!(JournalConfig::new(10, 80, 0).is_err(), "zero reclaim batch");
}
