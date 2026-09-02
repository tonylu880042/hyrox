//! The development venue: a course, a reader map, a set of bands, and a class script.
//!
//! It describes what happens in front of the antennas -- which tag is read at which reader,
//! and when -- and nothing else. Turning that into wire events is the emulated collector's
//! job (`crate::sim`, over `crates/simulator`), so boot ids, sequence numbers and journal
//! behaviour come from the device model rather than being invented here (CLAUDE.md 16, 25).
//!
//! ponytail: deterministic script, no jitter. The dropouts, resends, reboots and duplicates
//! live in `crates/simulator` and its tests; the dev screen does not need them to be useful.

use domain::{
    Course, CourseStep, Instant, ReaderId, ReaderKey, ReaderMode, ReaderRegistration,
    StationTarget, TagId,
};

/// One dev collector, standing in for the venue's ESP32s. Its base MAC is its identity
/// (CLAUDE.md 7.3).
pub const DEVICE_MAC: &str = "a4:cf:12:8b:3d:91";

/// One tag presented to one reader at one moment. The moment is the *detection* time, which
/// is the only timestamp a result may be computed from (CLAUDE.md 11, 17).
///
/// Only the emulated collector reads the script, so a build without `dev-simulator` has no
/// caller for it. That is what the feature is for, not code to delete.
#[cfg_attr(not(feature = "dev-simulator"), allow(dead_code))]
pub struct ScriptedRead {
    pub at: Instant,
    pub reader_id: ReaderId,
    pub tag_id: String,
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
    DEVICE_MAC.replace(':', "")
}

/// The reader map the hub interprets reads through (CLAUDE.md 8). Handed to the registration
/// use case at startup, which persists it: after the first run these come back from the
/// store instead (ADR 0004).
pub fn readers() -> Vec<ReaderRegistration> {
    let mut out = Vec::new();
    for step in &course().steps {
        for mode in [ReaderMode::Entry, ReaderMode::Exit] {
            let key = ReaderKey::parse(&device_id(), &reader_id(&step.station, mode))
                .expect("dev reader ids are canonical");
            out.push(ReaderRegistration::new(key, &step.station, mode));
        }
    }
    out
}

/// Bands handed out at check-in (ADR 0001 D3), bound through the check-in use case so they
/// are stored and survive a restart like real ones.
pub fn bands() -> Vec<(TagId, String)> {
    (0..athletes().len())
        .map(|i| {
            let id = athlete_id(i);
            let tag = TagId::parse(&tag_for(&id)).expect("dev tag ids are non-empty");
            (tag, id)
        })
        .collect()
}

/// One class: each athlete walks the course, staggered at the start, with per-athlete pace.
#[cfg_attr(not(feature = "dev-simulator"), allow(dead_code))]
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

    reads
        .into_iter()
        .map(|(t, station, mode, tag_id)| ScriptedRead {
            at: Instant(t),
            reader_id: ReaderId::parse(&reader_id(&station, mode)).expect("dev reader"),
            tag_id,
        })
        .collect()
}
