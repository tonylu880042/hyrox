//! Venue PIN security lock (ADR 0001; M6 follow-up).
//!
//! Protects settings and power operations from unauthorized Wi-Fi users.

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{anonymous, call, post, running};

const TABLET: &str = "COACH TABLET";

#[tokio::test]
async fn default_pin_2018_verifies_successfully() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        anonymous("POST", "/api/operator/pin/verify", json!({ "pin": "2018" })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn incorrect_pin_returns_403_forbidden() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        anonymous("POST", "/api/operator/pin/verify", json!({ "pin": "9999" })),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "PIN_INVALID");
}

#[tokio::test]
async fn changing_pin_requires_operator_name() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        anonymous(
            "POST",
            "/api/operator/pin/change",
            json!({
                "current_pin": "2018",
                "new_pin": "5678"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "OPERATOR_REQUIRED");
}

#[tokio::test]
async fn changing_pin_with_incorrect_current_pin_returns_403() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        post(
            "/api/operator/pin/change",
            TABLET,
            json!({
                "current_pin": "0000",
                "new_pin": "5678"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "PIN_INVALID");
}

#[tokio::test]
async fn changing_pin_with_invalid_format_returns_bad_request() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        post(
            "/api/operator/pin/change",
            TABLET,
            json!({
                "current_pin": "2018",
                "new_pin": "abc"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "INVALID_SETTING");
}

#[tokio::test]
async fn changing_pin_updates_pin_and_audits() {
    let (router, store) = running();

    // Change PIN from 2018 to 8888
    let (status, body) = call(
        &router,
        post(
            "/api/operator/pin/change",
            TABLET,
            json!({
                "current_pin": "2018",
                "new_pin": "8888"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);

    // Old PIN fails
    let (status, _) = call(
        &router,
        anonymous("POST", "/api/operator/pin/verify", json!({ "pin": "2018" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // New PIN succeeds
    let (status, _) = call(
        &router,
        anonymous("POST", "/api/operator/pin/verify", json!({ "pin": "8888" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Audit log recorded with masked PIN
    let audit = store
        .audits()
        .into_iter()
        .find(|a| a.action == "VENUE_SETTING" && a.subject == "security.pin")
        .expect("an audit entry for pin change");
    assert_eq!(audit.operator, TABLET);
    assert_eq!(audit.after.as_deref(), Some("****"));
}
