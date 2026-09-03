//! Venue settings: the handful of numbers a site adjusts for itself (M6 follow-up).
//!
//! Not session configuration -- these outlive any class, like the reader map does. The
//! first of them is how long the live screen holds a page before rotating, which depends on
//! the room: a long thin gym reads slower than a studio.
//!
//! Stored, because a venue that has to set it again after every restart will not set it.

mod support;

use application::{save_venue_setting, venue_settings, OperatorCommand, SettingError};
use domain::Instant;
use support::FakeStore;

const NOW: Instant = Instant(5_000_000);

fn desk() -> OperatorCommand {
    OperatorCommand::new("SETTINGS TABLET", NOW)
}

#[tokio::test]
async fn a_venue_that_has_set_nothing_gets_the_default() {
    let store = FakeStore::new();

    let settings = venue_settings(&store).await.expect("settings");

    assert_eq!(
        settings.live_page_ms, 10_000,
        "ten seconds, the value shipped with it"
    );
}

#[tokio::test]
async fn a_stored_value_is_what_the_screen_gets() {
    let store = FakeStore::new();

    save_venue_setting(&store, "live.page_ms", "20000", &desk())
        .await
        .expect("saved");

    assert_eq!(
        venue_settings(&store).await.expect("settings").live_page_ms,
        20_000
    );
}

/// The bounds are sanity, not product policy: a page that flips every 200ms is unreadable,
/// and one that holds for an hour is broken in a way nobody would notice until a class had
/// run its whole length.
#[tokio::test]
async fn an_impossible_rotation_is_refused() {
    let store = FakeStore::new();

    for bad in ["0", "500", "999999", "-5000", "soon", ""] {
        assert!(
            matches!(
                save_venue_setting(&store, "live.page_ms", bad, &desk()).await,
                Err(SettingError::Invalid { .. })
            ),
            "{bad:?} should be refused"
        );
    }
}

#[tokio::test]
async fn a_setting_nobody_defined_is_refused_rather_than_stored() {
    let store = FakeStore::new();

    let saved = save_venue_setting(&store, "live.colour_scheme", "neon", &desk()).await;

    assert!(
        matches!(saved, Err(SettingError::Unknown(_))),
        "an unknown key is a typo, not a feature"
    );
}

/// It changes what a venue's screen does, so it is a write like any other (ADR 0001 D1).
#[tokio::test]
async fn changing_a_setting_is_audited() {
    let store = FakeStore::new();

    save_venue_setting(&store, "live.page_ms", "15000", &desk())
        .await
        .expect("saved");

    let entry = store.audits().pop().expect("an audit record");
    assert_eq!(entry.action, "VENUE_SETTING");
    assert_eq!(entry.subject, "live.page_ms");
    assert_eq!(entry.operator, "SETTINGS TABLET");
    assert_eq!(entry.after.as_deref(), Some("15000"));
}

/// A value stored by an older build, or edited by hand into something absurd, must not take
/// a screen down. The default is what a broken row falls back to.
#[tokio::test]
async fn a_stored_value_that_makes_no_sense_falls_back_to_the_default() {
    let store = FakeStore::new().with_venue_setting("live.page_ms", "banana");

    assert_eq!(
        venue_settings(&store).await.expect("settings").live_page_ms,
        10_000
    );
}

// --- how many fit on a page (M6 follow-up) --------------------------------------------

#[tokio::test]
async fn the_default_page_holds_twelve() {
    let store = FakeStore::new();

    assert_eq!(
        venue_settings(&store)
            .await
            .expect("settings")
            .live_page_size,
        12
    );
}

#[tokio::test]
async fn a_venue_can_pick_a_denser_page() {
    let store = FakeStore::new();

    save_venue_setting(&store, "live.page_size", "30", &desk())
        .await
        .expect("saved");

    assert_eq!(
        venue_settings(&store)
            .await
            .expect("settings")
            .live_page_size,
        30
    );
}

/// The point of offering layouts instead of a number: seven cards leave a ragged row, and
/// nobody chose what shape those cards would be.
#[tokio::test]
async fn a_page_size_nobody_designed_is_refused() {
    let store = FakeStore::new();

    for bad in ["7", "13", "1", "100", "twelve"] {
        assert!(
            matches!(
                save_venue_setting(&store, "live.page_size", bad, &desk()).await,
                Err(SettingError::Invalid { .. })
            ),
            "{bad:?} should be refused"
        );
    }
}

/// Every offered layout is a whole grid: columns times rows is exactly the page size, so
/// the last row is never half empty by design.
#[test]
fn every_offered_layout_fills_its_grid() {
    for (size, cols, rows) in application::LIVE_PAGE_LAYOUTS {
        assert_eq!(cols * rows, size, "{cols}x{rows} does not hold {size}");
    }
}
