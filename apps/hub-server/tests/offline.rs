//! The screens must not depend on anything outside the hub (CLAUDE.md 31; roadmap M8).
//!
//! A venue that loses its uplink keeps timing correctly -- ingestion, storage and the
//! WebSocket are all local. What used to break was the *picture*: the stylesheet and the
//! fonts came from a CDN, so the data stayed right while the screen became unreadable
//! markup. That is the worst of both, and it is the kind of regression nobody reports
//! from a developer machine that always has internet.

const TRAINING: &str = include_str!("../static/training.html");
const WORKOUT: &str = include_str!("../static/workout.html");
const CHECKIN: &str = include_str!("../static/checkin.html");
const LEADERBOARD: &str = include_str!("../static/leaderboard.html");
const RESULT: &str = include_str!("../static/result.html");
const APP_CSS: &str = include_str!("../static/app.css");
const FONTS_CSS: &str = include_str!("../static/fonts.css");

/// Hosts a page may not reach at run time. `fonts.googleapis.com` and `cdn.tailwindcss.com`
/// are the two that were actually there; the rest are the neighbours most likely to be
/// pasted in next.
const OFF_SITE: [&str; 6] = [
    "cdn.tailwindcss.com",
    "fonts.googleapis.com",
    "fonts.gstatic.com",
    "unpkg.com",
    "cdn.jsdelivr.net",
    "cdnjs.cloudflare.com",
];

#[test]
fn no_screen_loads_anything_from_a_cdn() {
    for (name, page) in [
        ("training.html", TRAINING),
        ("workout.html", WORKOUT),
        ("checkin.html", CHECKIN),
        ("leaderboard.html", LEADERBOARD),
        ("result.html", RESULT),
    ] {
        for host in OFF_SITE {
            assert!(
                !page.contains(host),
                "{name} references {host}; with no uplink that page renders unstyled"
            );
        }
    }
}

/// The stylesheet is generated from the CDN's own output, so it must not smuggle the
/// remote font URLs back in through `@font-face`.
#[test]
fn the_stylesheets_reference_only_local_files() {
    for (name, css) in [("app.css", APP_CSS), ("fonts.css", FONTS_CSS)] {
        for host in OFF_SITE {
            assert!(!css.contains(host), "{name} still points at {host}");
        }
        assert!(
            !css.contains("url(http"),
            "{name} loads something over the network"
        );
    }
}

/// Every `url(fonts/...)` in the generated stylesheet has to resolve, or a face silently
/// falls back and the screen changes shape.
#[test]
fn every_font_the_stylesheet_asks_for_is_embedded() {
    let embedded: Vec<&str> = hub_server_fonts();
    let mut asked = 0;
    for chunk in FONTS_CSS.split("url(fonts/").skip(1) {
        let file = chunk.split(')').next().expect("a closing paren");
        assert!(
            embedded.contains(&file),
            "fonts.css asks for {file}, which is not embedded"
        );
        asked += 1;
    }
    assert!(asked > 0, "fonts.css declares no local font at all");
}

/// The binary's font table, read from the generated module rather than duplicated here.
fn hub_server_fonts() -> Vec<&'static str> {
    include_str!("../src/fonts.rs")
        .split('"')
        .filter(|s| s.ends_with(".woff2") && !s.contains('/'))
        .collect()
}
