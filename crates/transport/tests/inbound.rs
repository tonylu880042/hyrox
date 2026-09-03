//! Classifying what arrived, with no broker in the build (CLAUDE.md 24).
//!
//! The subscriber loop's whole job is decode → hand off → publish the ACK it was given, so
//! everything it *could* get wrong is decided by [`transport::classify`], and that is pure.

use contract::{AckPayload, AckStatus, DeviceId, EdgeEvent, ReaderId};
use transport::{classify, payload_excerpt, topic, DeviceStatus, DeviceWarning, Inbound};

fn device() -> DeviceId {
    DeviceId::from_mac_str("a4cf128b3d91").unwrap()
}

fn event() -> EdgeEvent {
    EdgeEvent {
        device_id: device(),
        reader_id: ReaderId::parse("rfid-02").unwrap(),
        boot_id: 18,
        sequence: 10_382,
        tag_id: vec!["E280117000001234".to_string()],
        detected_at: 1_787_734_821_382,
        uptime_ms: 382_912,
    }
}

#[test]
fn an_event_on_an_event_topic_decodes_to_an_event() {
    let sent = event();
    match classify(&topic::events(&device()), &sent.encode()) {
        Inbound::Event(received) => assert_eq!(*received, sent),
        other => panic!("expected an event, got {other:?}"),
    }
}

#[test]
fn a_status_on_a_status_topic_decodes_to_a_status() {
    let status = DeviceStatus {
        device_id: device(),
        boot_id: 18,
        pending_events: 8_123,
        journal_capacity: 10_000,
        warning: Some(DeviceWarning::JournalNearlyFull),
    };
    match classify(&topic::status(&device()), &status.encode()) {
        Inbound::Status(received) => assert_eq!(*received, status),
        other => panic!("expected a status, got {other:?}"),
    }
}

#[test]
fn an_ack_on_the_downlink_branch_decodes_to_an_ack() {
    let ack = AckPayload {
        device_id: device(),
        boot_id: 18,
        sequence: 10_382,
        status: AckStatus::Duplicate,
    };
    match classify(&topic::ack(&device()), &ack.encode()) {
        Inbound::Ack(received) => assert_eq!(*received, ack),
        other => panic!("expected an ack, got {other:?}"),
    }
}

/// A broken device must not be able to stop a class (CLAUDE.md 31 principle 1): everything
/// it can send has to come back as a value the loop can carry on from.
#[test]
fn rubbish_on_one_of_our_topics_is_reported_whole_and_never_fails() {
    for payload in [
        b"{not json".to_vec(),
        b"".to_vec(),
        // Structurally valid JSON, but not an event the contract accepts: a negative
        // counter would poison the idempotency key (CLAUDE.md 16).
        br#"{"device_id":"a4cf128b3d91","reader_id":"rfid-02","boot_id":-1,
             "sequence":1,"tag_id":"E28","detected_at":1,"uptime_ms":1}"#
            .to_vec(),
        vec![0xff, 0xfe, 0x00, 0x01],
    ] {
        match classify(&topic::events(&device()), &payload) {
            Inbound::Undecodable {
                topic,
                payload: kept,
                ..
            } => {
                assert_eq!(topic, "hyrox/v1/edge/a4cf128b3d91/events");
                // Kept whole: it is the only evidence of what the device actually sent.
                assert_eq!(kept, payload);
            }
            other => panic!("expected undecodable, got {other:?}"),
        }
    }
}

#[test]
fn a_topic_that_is_not_ours_is_foreign_not_an_error() {
    for name in ["hyrox/v2/edge/a4cf128b3d91/events", "other/app/topic", ""] {
        match classify(name, b"whatever") {
            Inbound::Foreign { topic } => assert_eq!(topic, name),
            other => panic!("expected {name} to be foreign, got {other:?}"),
        }
    }
}

/// The hub logs the excerpt rather than the payload, so a device stuck in a loop cannot
/// drown the operator's log in its own noise.
#[test]
fn an_undecodable_payload_is_logged_bounded_and_printable() {
    let excerpt = payload_excerpt(&vec![b'x'; 4096], 16);
    assert!(excerpt.starts_with("xxxxxxxxxxxxxxxx"));
    assert!(excerpt.contains("4096 bytes total"));
    assert!(excerpt.len() < 64);

    // Not UTF-8, and not a newline that could forge a log line.
    let excerpt = payload_excerpt(&[0xff, b'\n', b'a'], 8);
    assert!(!excerpt.contains('\n'), "{excerpt}");
}
