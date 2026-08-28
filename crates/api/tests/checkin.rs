//! `/checkin`: the narrow write surface (ADR 0001 D3).

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{armed, call, get, post};

const DOOR: &str = "DOOR TABLET";

#[tokio::test]
async fn a_band_is_bound_to_an_athlete_and_the_binding_is_audited() {
    let (router, store) = armed();

    let (status, body) = call(
        &router,
        post(
            "/api/checkin/bind",
            DOOR,
            json!({ "tag_id": "E280117000001234", "athlete_id": "a1" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    // Nothing was read before the band was handed out, so nothing was claimed. Reported
    // rather than omitted: an empty list is the normal case, not a missing answer.
    assert!(body["claimed"].as_array().expect("a list").is_empty());

    let bindings = store.saved_bindings();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].athlete_id, "a1");
    assert_eq!(bindings[0].tag_id.as_str(), "E280117000001234");

    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "TAG_BIND");
    assert_eq!(audit.operator, DOOR);
    assert_eq!(audit.after.as_deref(), Some("a1"));
}

/// After binding, the check-in screen must show the athlete as having a band -- that list
/// is the entire work queue of the surface.
#[tokio::test]
async fn the_roster_reflects_a_band_that_has_just_been_bound() {
    let (router, _) = armed();
    call(
        &router,
        post(
            "/api/checkin/bind",
            DOOR,
            json!({ "tag_id": "E280117000001234", "athlete_id": "a2" }),
        ),
    )
    .await;

    let (status, body) = call(&router, get("/api/checkin")).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["athletes"][0]["tag_id"].is_null());
    assert_eq!(body["athletes"][1]["tag_id"], "E280117000001234");
}

/// One band, one wrist (D3). The second attempt is a conflict, not a silent reassignment.
#[tokio::test]
async fn a_band_already_on_somebody_cannot_be_bound_again() {
    let (router, _) = armed();
    call(
        &router,
        post(
            "/api/checkin/bind",
            DOOR,
            json!({ "tag_id": "E280117000001234", "athlete_id": "a1" }),
        ),
    )
    .await;

    let (status, body) = call(
        &router,
        post(
            "/api/checkin/bind",
            DOOR,
            json!({ "tag_id": "E280117000001234", "athlete_id": "a2" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "TAG_ALREADY_BOUND");
}

#[tokio::test]
async fn a_band_cannot_be_bound_to_somebody_off_the_roster() {
    let (router, store) = armed();

    let (status, body) = call(
        &router,
        post(
            "/api/checkin/bind",
            DOOR,
            json!({ "tag_id": "E280117000001234", "athlete_id": "nobody" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "UNKNOWN_ATHLETE");
    assert!(store.saved_bindings().is_empty());
}

/// Swapping a band changes which wristband carries somebody's results, so CLAUDE.md 20
/// wants the reason on the record.
#[tokio::test]
async fn swapping_a_band_needs_a_reason() {
    let (router, _) = armed();
    call(
        &router,
        post(
            "/api/checkin/bind",
            DOOR,
            json!({ "tag_id": "E280117000001234", "athlete_id": "a1" }),
        ),
    )
    .await;

    let (status, body) = call(
        &router,
        post(
            "/api/checkin/rebind",
            DOOR,
            json!({ "tag_id": "E28011700000FFFF", "athlete_id": "a1" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "REASON_REQUIRED");
}

#[tokio::test]
async fn a_swapped_band_closes_the_old_binding_and_keeps_it() {
    let (router, store) = armed();
    call(
        &router,
        post(
            "/api/checkin/bind",
            DOOR,
            json!({ "tag_id": "E280117000001234", "athlete_id": "a1" }),
        ),
    )
    .await;

    let (status, _) = call(
        &router,
        post(
            "/api/checkin/rebind",
            DOOR,
            json!({ "tag_id": "E28011700000FFFF", "athlete_id": "a1",
                    "reason": "設備異常" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let bindings = store.saved_bindings();
    // Both rows survive: dropping the closed one would make "who was wearing this band at
    // 10:15" unanswerable (CLAUDE.md 20).
    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().any(|b| b.unbound_at.is_some()));
    let audit = store
        .audits()
        .into_iter()
        .find(|a| a.action == "TAG_REBIND")
        .expect("a rebind audit");
    assert_eq!(audit.before.as_deref(), Some("E280117000001234"));
    assert_eq!(audit.after.as_deref(), Some("E28011700000FFFF"));
}
