//! The interface dictionary (roadmap M7).
//!
//! `static/i18n.js` is the one place both screens read their labels from. Three languages
//! maintained by hand drift the moment somebody adds a key to one and forgets the others,
//! and the symptom -- an English label in the middle of a Chinese screen -- is exactly the
//! kind of thing nobody reports. So the parity is asserted here rather than noticed later.

const I18N: &str = include_str!("../static/i18n.js");
const WORKOUT: &str = include_str!("../static/workout.html");
const TRAINING: &str = include_str!("../static/training.html");
const CHECKIN: &str = include_str!("../static/checkin.html");
const LEADERBOARD: &str = include_str!("../static/leaderboard.html");
const RESULT: &str = include_str!("../static/result.html");

/// Every `"key":` at the top level of one language's object.
fn keys_of(language: &str) -> Vec<String> {
    let start = I18N
        .find(&format!("\"{language}\": {{"))
        .unwrap_or_else(|| panic!("{language} is missing from the dictionary"));
    let body = &I18N[start..];
    let mut depth = 0i32;
    let mut keys = Vec::new();
    let bytes: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            '"' if depth == 1 => {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != '"' {
                    j += 1;
                }
                let literal: String = bytes[i + 1..j].iter().collect();
                // A key is a string immediately followed by a colon; a value is not.
                let mut k = j + 1;
                while k < bytes.len() && bytes[k] == ' ' {
                    k += 1;
                }
                if k < bytes.len() && bytes[k] == ':' {
                    keys.push(literal);
                }
                i = j;
            }
            _ => {}
        }
        i += 1;
    }
    keys
}

#[test]
fn every_language_defines_exactly_the_same_keys() {
    let english = keys_of("en");
    assert!(english.len() > 80, "the dictionary looks truncated: {} keys", english.len());

    for language in ["zh-Hant", "zh-Hans"] {
        let mut theirs = keys_of(language);
        let mut ours = english.clone();
        theirs.sort();
        ours.sort();

        let missing: Vec<_> = ours.iter().filter(|k| !theirs.contains(k)).collect();
        let extra: Vec<_> = theirs.iter().filter(|k| !ours.contains(k)).collect();
        assert!(missing.is_empty(), "{language} is missing {missing:?}");
        assert!(extra.is_empty(), "{language} has keys English does not: {extra:?}");
    }
}

#[test]
fn no_language_repeats_a_key() {
    for language in ["zh-Hant", "zh-Hans", "en"] {
        let keys = keys_of(language);
        let mut seen = keys.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), keys.len(), "{language} defines a key twice");
    }
}

/// Every key a screen asks for has to exist, or it renders as the key itself.
#[test]
fn every_key_the_screens_use_is_defined() {
    let defined = keys_of("en");
    for (page, source) in [
        ("workout.html", WORKOUT),
        ("training.html", TRAINING),
        ("checkin.html", CHECKIN),
        ("leaderboard.html", LEADERBOARD),
        ("result.html", RESULT),
    ] {
        for used in used_keys(source) {
            assert!(
                defined.contains(&used),
                "{page} asks for {used:?}, which the dictionary does not define"
            );
        }
    }
}

/// `data-i18n="…"`, its two variants, and `I18N.t("…")`.
///
/// The call marker is the fully qualified `I18N.t(` rather than a short alias, precisely so
/// this scan cannot be fooled: `toast("Deleted.")` ends in `t(` too.
fn used_keys(source: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for marker in [
        "data-i18n=\"",
        "data-i18n-html=\"",
        "data-i18n-placeholder=\"",
        "I18N.t(\"",
    ] {
        let mut rest = source;
        while let Some(at) = rest.find(marker) {
            rest = &rest[at + marker.len()..];
            if let Some(end) = rest.find('"') {
                let key = &rest[..end];
                // A key ending in `.` is a dynamic prefix -- `I18N.t("blk." + kind)`. The
                // scanner cannot resolve those, so the enums behind them are covered
                // exhaustively by `every_enum_value_has_a_translation` instead.
                if !key.is_empty() && !key.ends_with('.') {
                    keys.push(key.to_string());
                }
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

/// Identifiers are contract, not presentation (ADR 0008). Translating a station key would
/// unmap every reader and blank every pictogram at once, so the dictionary must never
/// contain one as a *key* -- exercises are keyed by `Exercise.code`.
#[test]
fn the_dictionary_never_keys_anything_by_station_key() {
    let keys = keys_of("en");
    for station in ["WALL BALLS", "SANDBAG LUNGES", "ROWING", "SLED PUSH", "BURPEE BROAD JUMP"] {
        assert!(
            !keys.iter().any(|k| k.contains(station)),
            "{station:?} is a station key and an identifier; it must not be a dictionary key"
        );
    }
    // The nine exercises are keyed by code, which is what stays stable.
    for code in [
        "ex.RUN", "ex.SKIERG", "ex.ROWERG", "ex.SLED_PUSH", "ex.SLED_PULL",
        "ex.BURPEE_BROAD_JUMP", "ex.FARMERS_CARRY", "ex.SANDBAG_LUNGE", "ex.WALL_BALL",
    ] {
        assert!(keys.iter().any(|k| k == code), "{code} is missing");
    }
}

/// The families the screens look up dynamically -- `I18N.t("blk." + block.block_type)` and
/// friends. The scanner cannot follow those, so every value each Rust enum can serialise is
/// listed here. A variant added to the domain without a label is a failing test, not a
/// screen showing `blk.ZONE_ROTATION` to a coach.
#[test]
fn every_enum_value_has_a_translation() {
    let keys = keys_of("en");
    let families: [(&str, &[&str]); 5] = [
        ("st", &["DRAFT", "READY", "RUNNING", "PAUSED", "COMPLETED", "CANCELLED"]),
        ("stage", &["PENDING", "READY", "ACTIVE", "COMPLETED", "SKIPPED", "DNF"]),
        ("blk", &["SEQUENTIAL", "ROUNDS", "AMRAP", "INTERVAL", "ZONE_ROTATION"]),
        (
            "cat",
            &["FOUNDATIONAL", "ENGINE", "POWER", "COMPLETE", "RACE_SIMULATION", "CUSTOM"],
        ),
        ("unit", &["METER", "KILOMETER", "REPS", "SECOND", "MINUTE", "CALORIE"]),
        // `finish.*` is covered by the markup, which names all three statically.
    ];
    for (prefix, values) in families {
        for value in values {
            let key = format!("{prefix}.{value}");
            assert!(keys.iter().any(|k| *k == key), "{key} is missing from the dictionary");
        }
    }
}

/// `docs/api.md` §6: branch on the `error` code, treat `message` as something for whoever
/// reads a log. Every code the API can answer with needs a line here, or a coach sees the
/// server's English.
#[test]
fn every_api_error_code_has_a_translation() {
    let keys = keys_of("en");
    for code in [
        "OPERATOR_REQUIRED", "INVALID_BODY", "UNKNOWN_SESSION", "UNKNOWN_ATHLETE",
        "UNKNOWN_EVENT", "UNKNOWN_TEMPLATE", "ILLEGAL_TRANSITION", "HAS_INTERPRETED_EVENTS",
        "SESSION_NOT_EDITABLE", "NO_FINISH_RULE", "TAG_ALREADY_BOUND",
        "ATHLETE_ALREADY_BOUND", "NOT_BOUND", "REASON_REQUIRED", "TEMPLATE_NOT_EDITABLE",
        "TEMPLATE_NOT_RUNNABLE", "CLASS_IN_PROGRESS", "STORAGE_FAILED",
    ] {
        let key = format!("err.{code}");
        assert!(keys.iter().any(|k| *k == key), "{key} is missing from the dictionary");
    }
}

/// `app.css` is a captured Tailwind build, not a live compiler: a class written as an
/// arbitrary value -- `h-[48px]`, `max-w-[360px]` -- exists only if it happened to be in
/// the source when that CSS was generated. Writing a new one produces no rule at all, and
/// the element renders at its natural size while the markup still looks correct.
///
/// Found the hard way: a venue logo styled `h-[48px]` came out four times its size and
/// pushed the whole header onto two lines. Inline styles are the answer for anything new.
#[test]
fn no_screen_invents_an_arbitrary_tailwind_class_the_stylesheet_does_not_have() {
    let css = include_str!("../static/app.css");
    let screens: [(&str, &str); 7] = [
        ("training.html", include_str!("../static/training.html")),
        ("leaderboard.html", include_str!("../static/leaderboard.html")),
        ("checkin.html", include_str!("../static/checkin.html")),
        ("workout.html", include_str!("../static/workout.html")),
        ("signup.html", include_str!("../static/signup.html")),
        ("settings.html", include_str!("../static/settings.html")),
        ("result.html", include_str!("../static/result.html")),
    ];

    // Nothing may be added to this list. An arbitrary Tailwind class that app.css does not
    // carry produces no rule at all, so the markup looks right and the element renders at
    // whatever it inherited -- the quietest kind of visual bug there is.
    // Empty, and it should stay that way. The five that were here when this test was
    // written have been converted to inline styles; the projector's per-station clock was
    // rendering at 16px where the design says 46, which on a wall is the difference between
    // a number and a smudge.
    const ALREADY_DEAD: [&str; 0] = [];

    let mut missing: Vec<String> = Vec::new();
    for (name, html) in screens {
        for class in arbitrary_classes(html) {
            // The stylesheet escapes the brackets; either spelling counts as present.
            let escaped = class.replace('[', "\\[").replace(']', "\\]");
            if !css.contains(&class) && !css.contains(&escaped) {
                missing.push(format!("{name}: {class}"));
            }
        }
    }
    missing.sort();
    missing.dedup();
    missing.retain(|m| !ALREADY_DEAD.contains(&m.as_str()));
    assert!(
        missing.is_empty(),
        "classes with no rule in app.css (use an inline style instead): {missing:?}"
    );
}

/// Every `utility-[value]` token that appears inside a `class="..."` attribute. Hand-rolled
/// rather than a regex crate: one shape, twenty lines, no dependency.
fn arbitrary_classes(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (_, attribute) in html.match_indices("class=\"").filter_map(|(i, _)| {
        let rest = &html[i + 7..];
        rest.find('"').map(|end| (i, &rest[..end]))
    }) {
        for token in attribute.split_whitespace() {
            let Some(open) = token.find('[') else { continue };
            if token.ends_with(']')
                && open > 0
                && token[..open].chars().all(|c| c.is_ascii_lowercase() || c == '-')
            {
                found.push(token.to_string());
            }
        }
    }
    found
}
