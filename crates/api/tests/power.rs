//! Power control from the settings screen (M6).
//!
//! The screens have no login: the network boundary is the security (ADR 0001 D1). Powering
//! the machine off is more consequential than anything else on that surface, so it is the
//! one action with a guard of its own -- it asks the same question the nightly window asks
//! (`safe_to_stop`), and refuses while a class is on the floor.

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{anonymous, call, completed, post, running};

const DESK: &str = "FRONT DESK TABLET";

#[tokio::test]
async fn powering_off_is_refused_while_a_class_is_running() {
    let (router, store) = running();

    let (status, body) = call(
        &router,
        post("/api/operator/power", DESK, json!({ "action": "POWEROFF", "reason": "打烊" })),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "CLASS_RUNNING");
    assert!(store.power_actions().is_empty(), "nothing was asked of the machine");
}

#[tokio::test]
async fn powering_off_is_allowed_once_the_class_is_over() {
    let (router, store) = completed();

    let (status, body) = call(
        &router,
        post("/api/operator/power", DESK, json!({ "action": "POWEROFF", "reason": "打烊" })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["action"], "POWEROFF");
    assert_eq!(store.power_actions(), ["POWEROFF"]);
}

/// It stops timing a venue's evening. That is the definition of an action needing a reason
/// (CLAUDE.md 20).
#[tokio::test]
async fn powering_off_needs_a_reason_and_is_audited() {
    let (router, store) = completed();

    let (status, body) =
        call(&router, post("/api/operator/power", DESK, json!({ "action": "POWEROFF" }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "REASON_REQUIRED");

    call(
        &router,
        post("/api/operator/power", DESK, json!({ "action": "REBOOT", "reason": "更新後重開" })),
    )
    .await;
    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "POWER_REBOOT");
    assert_eq!(audit.operator, DESK);
    assert_eq!(audit.reason.as_deref(), Some("更新後重開"));
}

#[tokio::test]
async fn a_power_action_needs_an_operator_name() {
    let (router, _store) = completed();

    let (status, body) = call(
        &router,
        anonymous("POST", "/api/operator/power", json!({ "action": "POWEROFF", "reason": "x" })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "OPERATOR_REQUIRED");
}

#[tokio::test]
async fn something_that_is_not_a_power_action_is_refused() {
    let (router, _store) = completed();

    let (status, body) = call(
        &router,
        post("/api/operator/power", DESK, json!({ "action": "FORMAT", "reason": "x" })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "INVALID_BODY");
}

/// Restarting the hub's own service is the one power action that is safe mid-class: the
/// session is rebuilt by replaying the log, and events published while it was down are
/// delivered afterwards (CLAUDE.md 15, 21). It is how a wedged screen gets fixed without
/// anybody walking to the machine.
#[tokio::test]
async fn restarting_the_service_is_allowed_while_a_class_runs() {
    let (router, store) = running();

    let (status, _body) = call(
        &router,
        post(
            "/api/operator/power",
            DESK,
            json!({ "action": "RESTART_SERVICE", "reason": "畫面卡住" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(store.power_actions(), ["RESTART_SERVICE"]);
}

// --- venue settings (M6 follow-up) ----------------------------------------------------

/// The live screen reads this, and the live screen is a wall with no operator identity.
#[tokio::test]
async fn the_venue_settings_are_readable_without_an_operator_name() {
    let (router, _store) = running();

    let (status, body) = call(&router, support::get("/api/settings")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["live_page_ms"], 10_000, "the shipped default");
}

#[tokio::test]
async fn a_venue_sets_its_own_rotation_and_reads_it_back() {
    let (router, _store) = running();

    let (status, body) =
        call(&router, support::put("/api/operator/settings", DESK, json!({ "live_page_ms": 20000 }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["live_page_ms"], 20_000);

    let (_, read_back) = call(&router, support::get("/api/settings")).await;
    assert_eq!(read_back["live_page_ms"], 20_000);
}

#[tokio::test]
async fn an_unreadable_rotation_is_refused() {
    let (router, _store) = running();

    let (status, body) =
        call(&router, support::put("/api/operator/settings", DESK, json!({ "live_page_ms": 200 }))).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "INVALID_SETTING");
}

#[tokio::test]
async fn changing_a_setting_needs_an_operator_name() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        support::anonymous("PUT", "/api/operator/settings", json!({ "live_page_ms": 20000 })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "OPERATOR_REQUIRED");
}
