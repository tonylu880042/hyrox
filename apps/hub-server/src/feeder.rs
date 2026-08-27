//! Scripted reader events for development. Stands in for MQTT ingestion (Milestone 3):
//! it drives the real domain `interpret()`, so what the screen shows is genuinely derived,
//! not faked at the view layer.
//!
//! ponytail: deterministic script, no jitter or dropouts. The real simulator
//! (CLAUDE.md 25) needs reconnects, duplicates and reboots; that belongs with MQTT.

use crate::state::CourseStation;
use domain::{Instant, ReaderBinding, ReaderMode};

pub struct ScriptedEvent {
    pub athlete: usize,
    pub binding: ReaderBinding,
    pub at: Instant,
}

pub fn course() -> Vec<CourseStation> {
    [
        ("SKIERG", "skierg", "500 M"),
        ("SLED PUSH", "sled_push", "25 M"),
        ("SLED PULL", "sled_pull", "25 M"),
        ("BURPEE BROAD JUMP", "burpee_broad_jump", "40 M"),
        ("ROWING", "rowing", "500 M"),
        ("FARMERS CARRY", "farmers_carry", "100 M"),
        ("SANDBAG LUNGES", "sandbag_lunges", "50 M"),
        ("WALL BALLS", "wall_balls", "50 REPS"),
    ]
    .iter()
    .map(|(n, k, p)| CourseStation {
        name: (*n).into(),
        key: (*k).into(),
        plan: (*p).into(),
    })
    .collect()
}

pub fn athletes() -> Vec<&'static str> {
    vec![
        "CHEN YU-TING", "LIN CHIA-HAO", "WANG SHU-FEN", "HUANG PEI-CHI",
        "TSAI MING-JU", "LEE KUAN-LIN", "WU YA-WEN", "CHANG WEI",
        "HSU MEI-LING", "KUO CHIH-HUNG", "YEH HSIAO-CHUN", "PAN JUN-HAO",
    ]
}

/// One class: each athlete walks the course, staggered at the start, with per-athlete pace.
/// Stations use dedicated ENTRY/EXIT readers here; which readers a venue actually installs
/// is an open issue (CLAUDE.md 28).
pub fn script(class_start: Instant) -> Vec<ScriptedEvent> {
    let course = course();
    let mut out = Vec::new();
    for (i, _) in athletes().iter().enumerate() {
        // Spread the start and vary pace so the screen shows a realistic spread.
        let stagger = (i as i64) * 9_000;
        let pace = 900 + (i as i64 % 5) * 90; // per-mille speed factor
        let mut t = class_start.0 + stagger;
        for station in &course {
            let work = (110_000 * pace) / 1000;
            let transition = (18_000 * pace) / 1000;
            out.push(ScriptedEvent {
                athlete: i,
                binding: ReaderBinding { station: station.name.clone(), mode: ReaderMode::Entry },
                at: Instant(t),
            });
            t += work;
            out.push(ScriptedEvent {
                athlete: i,
                binding: ReaderBinding { station: station.name.clone(), mode: ReaderMode::Exit },
                at: Instant(t),
            });
            t += transition;
        }
    }
    out.sort_by_key(|e| e.at.0);
    out
}
