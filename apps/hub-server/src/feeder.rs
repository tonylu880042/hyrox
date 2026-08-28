//! Scripted edge events for development. Stands in for MQTT ingestion (Milestone 3).
//!
//! It publishes what an ESP32 publishes -- device, reader, tag, boot, sequence, detected_at
//! (CLAUDE.md 16) -- and nothing else. The hub resolves the reader and the tag itself, so
//! the whole ingestion pipeline is exercised rather than short-circuited.
//!
//! ponytail: deterministic script, no jitter, no dropouts, no duplicates. The real
//! simulator (CLAUDE.md 25) covers reconnects, resends and reboots; that arrives with MQTT.

use domain::{
    BindingLedger, Course, CourseStep, Instant, ReaderKey, ReaderMode, ReaderRegistration,
    ReaderRegistry, StationTarget, TagId,
};
use mqtt::EdgeEvent;

/// One dev collector, standing in for the venue's ESP32s.
const DEVICE_MAC: &str = "a4:cf:12:8b:3d:91";
const BOOT_ID: i64 = 1;

pub struct ScriptedRead {
    pub event: EdgeEvent,
    pub at: Instant,
}

pub fn course() -> Course {
    Course::new(
        "HYROX CLASS",
        [
            ("SKIERG", StationTarget::Distance { meters: 500 }),
            ("SLED PUSH", StationTarget::Distance { meters: 25 }),
            ("SLED PULL", StationTarget::Distance { meters: 25 }),
            ("BURPEE BROAD JUMP", StationTarget::Distance { meters: 40 }),
            ("ROWING", StationTarget::Distance { meters: 500 }),
            ("FARMERS CARRY", StationTarget::Distance { meters: 100 }),
            ("SANDBAG LUNGES", StationTarget::Distance { meters: 50 }),
            ("WALL BALLS", StationTarget::Repetitions { count: 50 }),
        ]
        .into_iter()
        .map(|(name, target)| CourseStep::new(name).with_target(target))
        .collect(),
    )
}

pub fn athletes() -> Vec<&'static str> {
    vec![
        "CHEN YU-TING", "LIN CHIA-HAO", "WANG SHU-FEN", "HUANG PEI-CHI",
        "TSAI MING-JU", "LEE KUAN-LIN", "WU YA-WEN", "CHANG WEI",
        "HSU MEI-LING", "KUO CHIH-HUNG", "YEH HSIAO-CHUN", "PAN JUN-HAO",
    ]
}

pub fn athlete_id(index: usize) -> String {
    format!("a{}", index + 1)
}

fn tag_for(athlete_id: &str) -> String {
    format!("TAG-{}", athlete_id.to_ascii_uppercase())
}

/// Two readers per station, ENTRY and EXIT. Which readers a venue actually installs is an
/// open issue (CLAUDE.md 28), so this is one plausible layout, not a decision.
fn reader_id(station: &str, mode: ReaderMode) -> String {
    let slug: String = station
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect();
    let role = match mode {
        ReaderMode::Entry => "entry",
        _ => "exit",
    };
    format!("rfid-{slug}-{role}")
}

fn device_id() -> String {
    format!("esp32-{}", DEVICE_MAC.replace(':', ""))
}

/// The reader map the hub interprets reads through (CLAUDE.md 8). Held in memory: Phase 1
/// has no table for reader configuration yet.
pub fn readers() -> ReaderRegistry {
    let mut registry = ReaderRegistry::new();
    for step in &course().steps {
        for mode in [ReaderMode::Entry, ReaderMode::Exit] {
            let key = ReaderKey::parse(&device_id(), &reader_id(&step.station, mode))
                .expect("dev reader ids are canonical");
            registry.register(ReaderRegistration::new(key, &step.station, mode));
        }
    }
    registry
}

/// Bands handed out at check-in (ADR 0001 D3), pre-bound for the dev script.
pub fn bindings(session_id: &str, at: Instant) -> BindingLedger {
    let mut ledger = BindingLedger::new();
    for i in 0..athletes().len() {
        let id = athlete_id(i);
        let tag = TagId::parse(&tag_for(&id)).expect("dev tag ids are non-empty");
        ledger.bind(session_id, &tag, &id, at).expect("dev bindings do not collide");
    }
    ledger
}

/// One class: each athlete walks the course, staggered at the start, with per-athlete pace.
pub fn script(class_start: Instant) -> Vec<ScriptedRead> {
    let course = course();
    let mut reads = Vec::new();
    for (i, _) in athletes().iter().enumerate() {
        // Spread the start and vary pace so the screen shows a realistic spread.
        let stagger = (i as i64) * 9_000;
        let pace = 900 + (i as i64 % 5) * 90; // per-mille speed factor
        let mut t = class_start.0 + stagger;
        let tag = tag_for(&athlete_id(i));
        for step in &course.steps {
            let work = (110_000 * pace) / 1000;
            let transition = (18_000 * pace) / 1000;
            reads.push((t, step.station.clone(), ReaderMode::Entry, tag.clone()));
            t += work;
            reads.push((t, step.station.clone(), ReaderMode::Exit, tag.clone()));
            t += transition;
        }
    }
    reads.sort_by_key(|(t, _, _, _)| *t);

    // Sequence numbers are assigned in publication order, as an edge collector would: the
    // idempotency key is device + boot + sequence (CLAUDE.md 16).
    reads
        .into_iter()
        .enumerate()
        .map(|(seq, (t, station, mode, tag_id))| ScriptedRead {
            at: Instant(t),
            event: EdgeEvent {
                device_id: mqtt::DeviceId::from_mac(DEVICE_MAC).expect("dev MAC"),
                reader_id: mqtt::ReaderId::new(&reader_id(&station, mode)).expect("dev reader"),
                boot_id: BOOT_ID,
                sequence: seq as i64 + 1,
                tag_id,
                detected_at: t,
                uptime_ms: t - class_start.0,
            },
        })
        .collect()
}
