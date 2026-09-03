//! The edge → hub wire contract and its idempotency key (CLAUDE.md 16, 17, 24).
//!
//! Nothing here touches a broker: the contract is data, and data is testable on its own.

use contract::{DeviceId, EdgeEvent, EventId, ReaderId, ReceivedEvent, WireError};

/// The exact payload documented in CLAUDE.md 16.
const CANONICAL: &str = r#"{
  "device_id": "a4cf128b3d91",
  "reader_id": "rfid-02",
  "boot_id": 18,
  "sequence": 10382,
  "tag_id": ["E280117000001234"],
  "detected_at": 1787734821382,
  "uptime_ms": 382912
}"#;

fn canonical_event() -> EdgeEvent {
    EdgeEvent::decode(CANONICAL.as_bytes()).expect("the documented payload must decode")
}

#[test]
fn decodes_the_documented_payload() {
    let e = canonical_event();
    assert_eq!(e.device_id.as_str(), "a4cf128b3d91");
    assert_eq!(e.reader_id.as_str(), "rfid-02");
    assert_eq!(e.boot_id, 18);
    assert_eq!(e.sequence, 10382);
    assert_eq!(e.tag_id, ["E280117000001234"]);
    assert_eq!(e.detected_at, 1_787_734_821_382);
    assert_eq!(e.uptime_ms, 382_912);
}

#[test]
fn encodes_exactly_the_documented_field_set() {
    // A silent change to this shape would break every ESP32 in the venue (CLAUDE.md 30),
    // so the field names are asserted, not merely round-tripped.
    let json: serde_json::Value = serde_json::from_slice(&canonical_event().encode()).unwrap();
    let obj = json.as_object().unwrap();
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "boot_id",
            "detected_at",
            "device_id",
            "reader_id",
            "sequence",
            "tag_id",
            "uptime_ms"
        ]
    );
    assert_eq!(obj["device_id"], "a4cf128b3d91");
    assert_eq!(obj["boot_id"], 18);
}

#[test]
fn round_trips_through_json() {
    let e = canonical_event();
    assert_eq!(EdgeEvent::decode(&e.encode()).unwrap(), e);
}

#[test]
fn rejects_a_payload_missing_a_required_field() {
    let bad = r#"{"device_id":"a4cf128b3d91","reader_id":"rfid-02","boot_id":1,
                  "sequence":1,"tag_id":["E2"],"detected_at":1}"#;
    assert!(matches!(
        EdgeEvent::decode(bad.as_bytes()),
        Err(WireError::Malformed(_))
    ));
}

#[test]
fn rejects_counters_that_cannot_come_from_a_real_device() {
    for field in ["boot_id", "sequence", "detected_at", "uptime_ms"] {
        let mut v: serde_json::Value = serde_json::from_str(CANONICAL).unwrap();
        v[field] = serde_json::json!(-1);
        assert!(
            matches!(
                EdgeEvent::decode(v.to_string().as_bytes()),
                Err(WireError::NegativeCounter { .. })
            ),
            "a negative {field} must be rejected, not stored"
        );
    }
}

#[test]
fn rejects_an_empty_tag_id() {
    for empty in [serde_json::json!([""]), serde_json::json!(["E2", "  "])] {
        let mut v: serde_json::Value = serde_json::from_str(CANONICAL).unwrap();
        v["tag_id"] = empty;
        assert!(matches!(
            EdgeEvent::decode(v.to_string().as_bytes()),
            Err(WireError::EmptyField("tag_id"))
        ));
    }
}

#[test]
fn a_round_carries_every_tag_the_reader_saw() {
    // UHF anti-collision reports several tags from one inventory round (ADR 0014). They
    // travel in one message, under one idempotency key, released by one ACK.
    let mut v: serde_json::Value = serde_json::from_str(CANONICAL).unwrap();
    v["tag_id"] = serde_json::json!(["E280117000001234", "E280117000005678"]);
    let e = EdgeEvent::decode(v.to_string().as_bytes()).unwrap();
    assert_eq!(e.tag_id, ["E280117000001234", "E280117000005678"]);
    assert_eq!(
        EventId::of(&e).sequence(),
        10382,
        "the round has one sequence, not one per tag"
    );
}

#[test]
fn rejects_a_round_with_no_tags_in_it() {
    // An inventory round that saw nothing is not an event. Letting it through would burn a
    // sequence number and store a read of nobody.
    let mut v: serde_json::Value = serde_json::from_str(CANONICAL).unwrap();
    v["tag_id"] = serde_json::json!([]);
    assert!(matches!(
        EdgeEvent::decode(v.to_string().as_bytes()),
        Err(WireError::EmptyField("tag_id"))
    ));
}

#[test]
fn rejects_a_bare_string_tag_id() {
    // The single-tag spelling is gone, not silently accepted: firmware that still sends it
    // must fail loudly at integration rather than quietly at the venue (CLAUDE.md 30).
    let mut v: serde_json::Value = serde_json::from_str(CANONICAL).unwrap();
    v["tag_id"] = serde_json::json!("E280117000001234");
    assert!(matches!(
        EdgeEvent::decode(v.to_string().as_bytes()),
        Err(WireError::Malformed(_))
    ));
}

// --- device identity (CLAUDE.md 7.3) -------------------------------------------------

#[test]
fn device_id_is_derived_from_the_base_mac() {
    for mac in ["A4:CF:12:8B:3D:91", "a4-cf-12-8b-3d-91", "a4cf128b3d91"] {
        assert_eq!(
            DeviceId::from_mac_str(mac).unwrap().as_str(),
            "a4cf128b3d91",
            "{mac} must normalise to the canonical device id"
        );
    }
}

#[test]
fn device_id_rejects_something_that_is_not_a_mac() {
    assert!(DeviceId::from_mac_str("not-a-mac").is_err());
    assert!(DeviceId::from_mac_str("a4cf128b3d9").is_err(), "11 nibbles");
    assert!(DeviceId::from_mac_str("").is_err());
}

#[test]
fn device_id_parses_back_from_its_canonical_form() {
    let id = DeviceId::from_mac_str("a4:cf:12:8b:3d:91").unwrap();
    assert_eq!(DeviceId::parse(id.as_str()).unwrap(), id);
    assert!(DeviceId::parse("esp32-zzzz").is_err());
}

#[test]
fn reader_id_is_case_insensitive() {
    // Section 8 writes `RFID-02` in prose and `rfid-02` in the JSON example; folding case
    // stops the two spellings mapping to two different stations.
    assert_eq!(
        ReaderId::parse("RFID-02").unwrap(),
        ReaderId::parse("rfid-02").unwrap()
    );
    assert!(ReaderId::parse("").is_err());
}

// --- idempotency key (CLAUDE.md 16) --------------------------------------------------

#[test]
fn the_key_is_device_plus_boot_plus_sequence() {
    let e = canonical_event();
    let key = EventId::of(&e);
    assert_eq!(key.device_id().as_str(), "a4cf128b3d91");
    assert_eq!(key.boot_id(), 18);
    assert_eq!(key.sequence(), 10382);
    assert_eq!(key.to_string(), "a4cf128b3d91/18/10382");
}

#[test]
fn a_redelivery_carries_the_same_key() {
    let first = canonical_event();
    let mut redelivered = canonical_event();
    // A resend after reconnect may well be relabelled by the broker or arrive later; the
    // fields that identify the *event* are unchanged, so the key must be unchanged too.
    redelivered.uptime_ms += 5_000;
    assert_eq!(EventId::of(&first), EventId::of(&redelivered));
}

#[test]
fn a_reboot_makes_a_repeated_sequence_a_different_event() {
    // `sequence` restarts at reboot, so without `boot_id` the key would collide and a real
    // event would be discarded as a duplicate (CLAUDE.md 31: no lost RFID events).
    let mut a = canonical_event();
    a.boot_id = 18;
    a.sequence = 1;
    let mut b = a.clone();
    b.boot_id = 19;
    assert_ne!(EventId::of(&a), EventId::of(&b));
}

#[test]
fn different_devices_never_share_a_key() {
    let mut a = canonical_event();
    let mut b = canonical_event();
    a.device_id = DeviceId::from_mac_str("a4cf128b3d91").unwrap();
    b.device_id = DeviceId::from_mac_str("a4cf128b3d92").unwrap();
    assert_ne!(EventId::of(&a), EventId::of(&b));
}

// --- official vs diagnostic time (CLAUDE.md 11, 17) ----------------------------------

#[test]
fn official_time_is_detected_at_not_arrival() {
    let e = canonical_event();
    let detected = e.detected_at;
    let late = ReceivedEvent::new(e.clone(), detected + 8_000);
    let prompt = ReceivedEvent::new(e, detected + 3);

    assert_eq!(late.official_time(), detected);
    assert_eq!(prompt.official_time(), late.official_time());
    assert_eq!(late.arrival_lag_ms(), 8_000, "lag is diagnostics only");
}
