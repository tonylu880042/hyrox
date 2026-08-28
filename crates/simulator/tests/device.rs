//! A single emulated ESP32: identity, readers, counters and reboot (CLAUDE.md 16, 18, 25).

use contract::{AckStatus, ReaderId};
use transport::DeviceWarning;
use simulator::{AbsentTimeout, DeviceConfig, DeviceError, JournalConfig, ReaderConfig, RfOutcome, SimDevice};

const TAG_A: &str = "E280117000001234";
const TAG_B: &str = "E280117000005678";
const T0: i64 = 1_787_734_800_000;

fn reader(id: &str, timeout_ms: i64) -> ReaderConfig {
    ReaderConfig::new(id, AbsentTimeout::from_millis(timeout_ms).unwrap()).unwrap()
}

fn two_reader_device() -> SimDevice {
    let config = DeviceConfig::new("A4:CF:12:8B:3D:91")
        .unwrap()
        .with_reader(reader("rfid-01", 4_000))
        .with_reader(reader("rfid-02", 4_000));
    SimDevice::boot(config, T0).unwrap()
}

fn rid(id: &str) -> ReaderId {
    ReaderId::parse(id).unwrap()
}

#[test]
fn the_device_id_comes_from_the_configured_mac() {
    let d = two_reader_device();
    assert_eq!(d.device_id().as_str(), "esp32-a4cf128b3d91");

    let other = SimDevice::boot(
        DeviceConfig::new("a4:cf:12:8b:3d:92").unwrap().with_reader(reader("rfid-01", 4_000)),
        T0,
    )
    .unwrap();
    assert_ne!(d.device_id(), other.device_id());
}

#[test]
fn a_device_needs_at_least_one_reader_and_a_real_mac() {
    assert!(DeviceConfig::new("nope").is_err());
    let no_readers = DeviceConfig::new("a4cf128b3d91").unwrap();
    assert!(SimDevice::boot(no_readers, T0).is_err());
}

#[test]
fn one_device_serves_several_readers() {
    // CLAUDE.md 7.3: one ESP32 may support more than one Reader, and `reader_id` stays
    // separate from `device_id`.
    let mut d = two_reader_device();
    assert!(matches!(d.rf_read(&rid("rfid-01"), TAG_A, T0), Ok(RfOutcome::Emitted(_))));
    assert!(matches!(d.rf_read(&rid("rfid-02"), TAG_A, T0 + 10), Ok(RfOutcome::Emitted(_))));

    let readers: Vec<String> = d.pending().iter().map(|e| e.reader_id.to_string()).collect();
    assert_eq!(readers, ["rfid-01", "rfid-02"]);
}

#[test]
fn each_reader_suppresses_on_its_own_presence_state() {
    let mut d = two_reader_device();
    d.rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    // The same tag at the same instant on the other antenna is a separate observation.
    assert!(matches!(d.rf_read(&rid("rfid-02"), TAG_A, T0), Ok(RfOutcome::Emitted(_))));
    assert!(matches!(d.rf_read(&rid("rfid-01"), TAG_A, T0 + 500), Ok(RfOutcome::Suppressed)));
}

#[test]
fn several_tags_can_be_in_front_of_one_reader() {
    let mut d = two_reader_device();
    d.rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    d.rf_read(&rid("rfid-01"), TAG_B, T0 + 200).unwrap();
    assert_eq!(d.pending_count(), 2);

    let tags: Vec<String> = d.pending().iter().map(|e| e.tag_id.clone()).collect();
    assert_eq!(tags, [TAG_A, TAG_B]);
}

#[test]
fn an_unknown_reader_is_an_error_not_a_silent_event() {
    let mut d = two_reader_device();
    assert!(matches!(
        d.rf_read(&rid("rfid-99"), TAG_A, T0),
        Err(DeviceError::UnknownReader(_))
    ));
    assert_eq!(d.pending_count(), 0);
}

#[test]
fn a_suppressed_read_consumes_no_sequence_number() {
    // Sequence numbers are the idempotency key's third component (CLAUDE.md 16); a gap
    // would look like a lost event to anyone auditing the journal.
    let mut d = two_reader_device();
    d.rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    d.rf_read(&rid("rfid-01"), TAG_A, T0 + 100).unwrap();
    d.rf_read(&rid("rfid-01"), TAG_B, T0 + 200).unwrap();

    let seqs: Vec<i64> = d.pending().iter().map(|e| e.sequence).collect();
    assert_eq!(seqs, [1, 2]);
}

#[test]
fn detected_at_is_the_moment_of_the_read_and_uptime_is_measured_from_boot() {
    let mut d = two_reader_device();
    d.rf_read(&rid("rfid-01"), TAG_A, T0 + 7_500).unwrap();
    let e = &d.pending()[0];
    assert_eq!(e.detected_at, T0 + 7_500, "official timing (CLAUDE.md 11, 17)");
    assert_eq!(e.uptime_ms, 7_500);
}

#[test]
fn a_reboot_increments_boot_id_restarts_sequence_and_keeps_pending_events() {
    // CLAUDE.md 18: a reboot must preserve pending events. CLAUDE.md 16: `boot_id` is what
    // keeps the restarted sequence from colliding with the previous boot's events.
    let mut d = two_reader_device();
    d.rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    d.rf_read(&rid("rfid-01"), TAG_B, T0 + 100).unwrap();
    assert_eq!(d.boot_id(), 1);

    d.reboot(T0 + 60_000);
    assert_eq!(d.boot_id(), 2);
    assert_eq!(d.pending_count(), 2, "the journal survived the power cut");

    d.rf_read(&rid("rfid-01"), TAG_A, T0 + 61_000).unwrap();
    let after = d.pending().last().unwrap().clone();
    assert_eq!(after.boot_id, 2);
    assert_eq!(after.sequence, 1, "sequence restarts within the new boot");
    assert_eq!(after.uptime_ms, 1_000, "uptime is measured from this boot");

    // The pre-reboot events keep their own identity, so nothing collides.
    let keys: Vec<String> = d.pending().iter().map(|e| e.id().to_string()).collect();
    assert_eq!(keys.len(), 3);
    assert_eq!(
        keys.iter().collect::<std::collections::HashSet<_>>().len(),
        3,
        "three distinct idempotency keys"
    );
}

#[test]
fn a_reboot_makes_the_next_read_a_first_sight() {
    let mut d = two_reader_device();
    d.rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    d.reboot(T0 + 500);
    assert!(matches!(d.rf_read(&rid("rfid-01"), TAG_A, T0 + 600), Ok(RfOutcome::Emitted(_))));
}

#[test]
fn an_offline_device_keeps_recording() {
    // Losing the link must never mean losing a read (CLAUDE.md 31).
    let mut d = two_reader_device();
    d.disconnect();
    d.rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    assert!(d.publish_batch().is_empty(), "nothing goes out while offline");
    assert_eq!(d.pending_count(), 1);

    d.reconnect();
    assert_eq!(d.publish_batch().len(), 1);
}

#[test]
fn publishing_does_not_release_an_event_only_an_ack_does() {
    let mut d = two_reader_device();
    d.rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    let batch = d.publish_batch();
    assert_eq!(d.publish_batch().len(), 1, "still owed an ACK");

    let key = batch[0].id();
    d.acknowledge(&key, AckStatus::Stored);
    assert_eq!(d.pending_count(), 0);
}

#[test]
fn a_duplicate_ack_status_still_releases_the_event() {
    // The hub answers a redelivery with DUPLICATE; that is durable too (CLAUDE.md 16).
    let mut d = two_reader_device();
    d.rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    let key = d.pending()[0].id();
    d.acknowledge(&key, AckStatus::Duplicate);
    assert_eq!(d.pending_count(), 0);
}

#[test]
fn a_full_journal_reports_an_error_and_warns_over_mqtt() {
    let config = DeviceConfig::new("a4cf128b3d91")
        .unwrap()
        .with_reader(reader("rfid-01", 1))
        .with_journal(JournalConfig::new(2, 80, 1).unwrap());
    let mut d = SimDevice::boot(config, T0).unwrap();

    for i in 0..2 {
        d.rf_read(&rid("rfid-01"), TAG_A, T0 + i * 10).unwrap();
    }
    assert!(matches!(
        d.rf_read(&rid("rfid-01"), TAG_A, T0 + 100),
        Err(DeviceError::Journal(_))
    ));

    let status = d.status();
    assert_eq!(status.warning, Some(DeviceWarning::JournalFull));
    assert_eq!(status.pending_events, 2);
    assert_eq!(status.device_id, *d.device_id());
}
