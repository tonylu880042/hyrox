//! Demo data on demand (M6 follow-up).
//!
//! A venue's worth of invented athletes is exactly what you want on a test machine and
//! exactly what you never want on a customer's, so the interesting behaviour here is the
//! gate: a hub that was not set up to offer demo data says so, and the settings screen
//! shows nothing rather than a button that fails.

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{call, completed_session, del, demo_hub, get, post, running, running_session};

const DESK: &str = "FRONT DESK TABLET";

#[tokio::test]
async fn a_hub_without_demo_data_says_so_and_refuses_to_load_any() {
    // `running()` wires no demo capability, which is what a customer's machine looks like.
    let (router, _store) = running();

    let (status, body) = call(&router, get("/api/settings")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["demo_available"], false,
        "the screen has nothing to draw"
    );

    let (status, body) = call(&router, post("/api/operator/demo", DESK, json!({}))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "DEMO_UNAVAILABLE");
}

#[tokio::test]
async fn a_test_machine_loads_and_clears_a_venue() {
    let (router, demo) = demo_hub(completed_session());

    let (status, _body) = call(&router, post("/api/operator/demo", DESK, json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(demo.calls(), ["load"]);

    let (status, _body) = call(&router, del("/api/operator/demo", DESK, json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(demo.calls(), ["load", "clear"]);
}

/// The same guard the power buttons use: a class on the floor is somebody's evening, and
/// loading twelve invented athletes into it would be indistinguishable from a bug.
#[tokio::test]
async fn demo_data_is_refused_while_a_class_is_running() {
    let (router, demo) = demo_hub(running_session());

    let (status, body) = call(&router, post("/api/operator/demo", DESK, json!({}))).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "CLASS_RUNNING");
    assert!(demo.calls().is_empty(), "nothing was loaded");
}

#[tokio::test]
async fn loading_demo_data_is_audited_and_needs_an_operator_name() {
    let (router, _demo) = demo_hub(completed_session());

    let (status, body) = call(
        &router,
        support::anonymous("POST", "/api/operator/demo", json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "OPERATOR_REQUIRED");
}

/// Stopping is allowed at any time. A demo class that has gone wrong is exactly the moment
/// somebody needs the off switch, and stopping invented reads cannot hurt a real one.
#[tokio::test]
async fn clearing_demo_data_is_allowed_even_while_it_is_running() {
    let (router, demo) = demo_hub(running_session());

    let (status, _body) = call(&router, del("/api/operator/demo", DESK, json!({}))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(demo.calls(), ["clear"]);
}
