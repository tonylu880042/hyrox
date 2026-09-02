//! Edge device identity and the reader registry (CLAUDE.md 7.3, 8).
//!
//! The ESP32 knows only `device_id` + `reader_id`; every business meaning is looked up
//! here, and an id the hub does not recognise must produce a named outcome rather than
//! a panic, because unknown readers become operator exceptions upstream (ADR 0001 D4).

use domain::*;

fn device(raw: &str) -> DeviceId {
    DeviceId::parse(raw).expect("canonical device id")
}

fn reader(raw: &str) -> ReaderId {
    ReaderId::parse(raw).expect("valid reader id")
}

fn key(device_raw: &str, reader_raw: &str) -> ReaderKey {
    ReaderKey::new(device(device_raw), reader(reader_raw))
}

// --- device id (CLAUDE.md 7.3) --------------------------------------------------------

#[test]
fn canonical_device_id_round_trips() {
    let d = device("a4cf128b3d91");
    assert_eq!(d.as_str(), "a4cf128b3d91");
}

#[test]
fn device_id_is_derived_from_the_base_mac_not_generated() {
    // The hardware address is the identity; a reflashed board must come back as itself.
    let d = DeviceId::from_mac([0xa4, 0xcf, 0x12, 0x8b, 0x3d, 0x91]);
    assert_eq!(d.as_str(), "a4cf128b3d91");
    assert_eq!(d, device("a4cf128b3d91"), "both routes must agree");
}

#[test]
fn mac_strings_are_normalised_to_the_canonical_form() {
    // The whole point of normalising (ADR 0015): six spellings of one MAC would otherwise
    // become six rows in the reader registry, and the lookup would miss on punctuation
    // alone -- silently, at the venue.
    let expected = device("a4cf128b3d91");
    for raw in ["a4:cf:12:8b:3d:91", "A4-CF-12-8B-3D-91", "A4CF128B3D91", "a4.cf.12.8b.3d.91"] {
        assert_eq!(DeviceId::from_mac_str(raw).unwrap(), expected, "{raw}");
    }
}

#[test]
fn device_ids_are_case_insensitive_but_stored_lowercase() {
    // §8 writes reader ids in upper case while the §16 wire payload is lower case;
    // parsing accepts either and stores one form so lookups cannot miss on case alone.
    assert_eq!(device("A4CF128B3D91").as_str(), "a4cf128b3d91");
}

#[test]
fn malformed_device_ids_are_rejected() {
    let cases = [
        ("", DeviceIdError::WrongLength { found: 0 }),
        ("a4cf128b3d9", DeviceIdError::WrongLength { found: 11 }),
        ("a4cf128b3d911", DeviceIdError::WrongLength { found: 13 }),
        // Separators are for humans and config files; the wire carries the canonical form
        // only, so there is exactly one spelling of one device (ADR 0015).
        ("a4:cf:12:8b:3d:91", DeviceIdError::WrongLength { found: 17 }),
        ("a4cf128b3dzz", DeviceIdError::NotHex),
        // The old prefixed form. Rejected rather than tolerated: two spellings is the thing
        // the canonical form exists to prevent.
        ("esp32-a4cf128b3d91", DeviceIdError::WrongLength { found: 18 }),
    ];
    for (raw, expected) in cases {
        assert_eq!(DeviceId::parse(raw), Err(expected), "{raw:?} must be rejected");
    }
}

#[test]
fn a_uuid_is_not_a_device_id() {
    // Guards CLAUDE.md 7.3 explicitly: random ids must never become hardware identity.
    assert!(DeviceId::parse("550e8400e29b41d4a716446655440000").is_err());
    assert!(DeviceId::parse("550e8400-e29b-41d4-a716-446655440000").is_err());
}

#[test]
fn reader_ids_are_validated_and_kept_separate_from_the_device() {
    assert_eq!(reader("RFID-02").as_str(), "rfid-02");
    assert_eq!(ReaderId::parse(""), Err(ReaderIdError::Empty));
    assert_eq!(ReaderId::parse("rfid 02"), Err(ReaderIdError::InvalidCharacter { found: ' ' }));
}

// --- reader registry (CLAUDE.md 8) ----------------------------------------------------

#[test]
fn registry_maps_device_and_reader_to_station_zone_and_mode() {
    let mut registry = ReaderRegistry::new();
    registry.register(
        ReaderRegistration::new(key("a4cf128b3d91", "rfid-02"), "SKIERG", ReaderMode::Entry)
            .with_zone("MAIN FLOOR"),
    );

    let found = registry.resolve(&key("a4cf128b3d91", "rfid-02")).unwrap();
    assert_eq!(found.station, "SKIERG");
    assert_eq!(found.zone.as_deref(), Some("MAIN FLOOR"));
    assert_eq!(found.mode, ReaderMode::Entry);
}

#[test]
fn one_device_can_host_several_readers() {
    // CLAUDE.md 7.3: reader_id stays separate precisely so this stays possible.
    let mut registry = ReaderRegistry::new();
    let dev = "a4cf128b3d91";
    registry.register(ReaderRegistration::new(key(dev, "rfid-01"), "SKIERG", ReaderMode::Entry));
    registry.register(ReaderRegistration::new(key(dev, "rfid-02"), "SKIERG", ReaderMode::Exit));

    assert_eq!(registry.resolve(&key(dev, "rfid-01")).unwrap().mode, ReaderMode::Entry);
    assert_eq!(registry.resolve(&key(dev, "rfid-02")).unwrap().mode, ReaderMode::Exit);
}

#[test]
fn the_same_reader_id_on_a_different_device_is_a_different_reader() {
    let mut registry = ReaderRegistry::new();
    registry.register(ReaderRegistration::new(
        key("a4cf128b3d91", "rfid-01"),
        "SKIERG",
        ReaderMode::Entry,
    ));
    let other = key("b0b1b2b3b4b5", "rfid-01");
    assert!(registry.resolve(&other).is_err(), "reader_id alone must not identify a reader");
}

#[test]
fn an_unregistered_reader_resolves_to_a_named_unknown_outcome() {
    // Never a panic: an unrecognised reader is an operator exception, not a crash (ADR D4).
    let registry = ReaderRegistry::new();
    let k = key("a4cf128b3d91", "rfid-09");
    assert_eq!(
        registry.resolve(&k),
        Err(UnknownReader {
            device_id: device("a4cf128b3d91"),
            reader_id: reader("rfid-09"),
        })
    );
}

#[test]
fn re_registering_a_reader_returns_the_replaced_mapping() {
    // Reader layout is an open issue (CLAUDE.md 28), so reconfiguration must be allowed --
    // but the previous mapping is handed back rather than silently discarded.
    let mut registry = ReaderRegistry::new();
    let k = key("a4cf128b3d91", "rfid-02");
    assert!(registry
        .register(ReaderRegistration::new(k.clone(), "SKIERG", ReaderMode::Entry))
        .is_none());

    let replaced = registry
        .register(ReaderRegistration::new(k.clone(), "ROWING", ReaderMode::Toggle))
        .expect("the old mapping must be returned");
    assert_eq!(replaced.station, "SKIERG");
    assert_eq!(registry.resolve(&k).unwrap().station, "ROWING");
    assert_eq!(registry.len(), 1, "re-registering must not duplicate the key");
}

#[test]
fn a_registration_yields_the_binding_the_interpreter_consumes() {
    // The registry is the only place business meaning is attached (CLAUDE.md 8).
    let registration = ReaderRegistration::new(
        key("a4cf128b3d91", "rfid-02"),
        "WALL BALLS",
        ReaderMode::Toggle,
    );
    let binding = registration.binding();
    assert_eq!(binding.station, "WALL BALLS");
    assert_eq!(binding.mode, ReaderMode::Toggle);
}

// --- removing one (ADR 0007 §7, amended) ----------------------------------------------

/// A reader can be taken off the wall, or a venue can be reconfigured. Removing the
/// registration is safe for history and always was: `raw_events` keeps the device and
/// reader that produced every read, and an interpretation records the **station**, not the
/// reader. What removal changes is only what happens next.
#[test]
fn a_reader_can_be_removed_and_stops_resolving() {
    let mut registry = ReaderRegistry::new();
    let key = ReaderKey::parse("a4cf128b3d91", "rfid-01").expect("a key");
    registry.register(ReaderRegistration::new(key.clone(), "SKIERG", ReaderMode::Entry));

    let removed = registry.remove(&key).expect("it was there");

    assert_eq!(removed.station, "SKIERG");
    assert!(registry.resolve(&key).is_err(), "a read from it is now an unknown reader");
    assert!(registry.is_empty());
}

/// Removing one that is not there is not a failure to report -- it is the state the caller
/// asked for. But the caller still needs to know nothing happened, so the answer says so.
#[test]
fn removing_a_reader_nobody_registered_says_there_was_nothing_to_remove() {
    let mut registry = ReaderRegistry::new();
    let key = ReaderKey::parse("a4cf128b3d91", "rfid-01").expect("a key");

    assert!(registry.remove(&key).is_none());
}

#[test]
fn removing_one_reader_leaves_the_others_alone() {
    let mut registry = ReaderRegistry::new();
    let entry = ReaderKey::parse("a4cf128b3d91", "rfid-01").expect("a key");
    let exit = ReaderKey::parse("a4cf128b3d91", "rfid-02").expect("a key");
    registry.register(ReaderRegistration::new(entry.clone(), "SKIERG", ReaderMode::Entry));
    registry.register(ReaderRegistration::new(exit.clone(), "SKIERG", ReaderMode::Exit));

    registry.remove(&entry);

    assert!(registry.resolve(&exit).is_ok(), "the exit antenna is untouched");
    assert_eq!(registry.len(), 1);
}
