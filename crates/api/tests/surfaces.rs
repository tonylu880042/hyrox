//! The read/write split, operator identity, and the mandatory freshness readout
//! (ADR 0001 D1, D5; ADR 0007).

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{anonymous, call, get, post, running, NOW};

/// The read-only surfaces of ADR 0001. A write must not be reachable at any of them.
const READ_ONLY_PATHS: [&str; 10] = [
    "/api/live",
    "/api/coach",
    "/api/session",
    "/api/result/s1",
    // The workout library is read here and written under /api/operator (ADR 0008), so
    // these paths must carry no mutating verb either.
    "/api/exercises",
    "/api/workout-templates",
    "/api/workout-templates/t1",
    "/api/stages",
    "/api/health",
    "/api/leaderboard",
];

#[tokio::test]
async fn every_read_only_surface_answers_a_read() {
    let (router, _) = running();

    for path in [
        "/api/live",
        "/api/coach",
        "/api/session",
        "/api/exercises",
        "/api/workout-templates",
        "/api/stages",
        "/api/health",
        "/api/leaderboard",
    ] {
        let (status, _) = call(&router, get(path)).await;
        assert_eq!(status, StatusCode::OK, "GET {path}");
    }
}

/// The structural half of the split: a read-only surface has no mutating route registered,
/// so axum's own method router refuses the verb before any handler of ours runs. The other
/// half is a compile-time fact -- `crate::read`'s state type has no write method -- and is
/// checked by the code compiling at all.
#[tokio::test]
async fn a_read_only_surface_has_no_write_route() {
    let (router, _) = running();

    for path in READ_ONLY_PATHS {
        for method in ["POST", "PUT", "DELETE"] {
            let (status, _) = call(&router, anonymous(method, path, json!({}))).await;
            assert_eq!(
                status,
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {path} must not be routable"
            );
        }
    }
}

/// A check-in tablet is handed to whoever is on the door. Its surface may bind bands and
/// must not be able to touch the session's clock (ADR 0001).
#[tokio::test]
async fn the_narrow_write_surface_cannot_control_the_session() {
    let (router, _) = running();

    for path in ["/api/checkin/session/complete", "/api/checkin/config"] {
        let (status, _) = call(&router, post(path, "DOOR TABLET", json!({}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path} must not exist");
    }
}

/// ADR 0001 D1: there is no login, so the device name is the whole audit identity of
/// CLAUDE.md 20. A write without one is refused rather than attributed to nobody.
#[tokio::test]
async fn a_write_without_an_operator_identity_is_refused() {
    let (router, store) = running();

    let writes = [
        ("POST", "/api/operator/session/complete"),
        ("POST", "/api/operator/session/reopen"),
        ("POST", "/api/operator/session/start"),
        ("POST", "/api/operator/session/draft"),
        ("POST", "/api/operator/session/end-class"),
        ("POST", "/api/operator/readers"),
        ("PUT", "/api/operator/config"),
        ("POST", "/api/operator/exceptions/1/void"),
        ("POST", "/api/operator/templates"),
        ("PUT", "/api/operator/templates/t1"),
        ("DELETE", "/api/operator/templates/t1"),
        ("POST", "/api/operator/templates/t1/duplicate"),
        ("POST", "/api/operator/class"),
        ("POST", "/api/checkin/entrants"),
        ("POST", "/api/checkin/bind"),
        ("POST", "/api/checkin/rebind"),
    ];

    for (method, path) in writes {
        let (status, body) = call(&router, anonymous(method, path, json!({}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {path}");
        assert_eq!(body["error"], "OPERATOR_REQUIRED", "{method} {path}");
    }

    // Nothing happened, and above all nothing was written to the trail under a blank name.
    assert!(store.audits().is_empty());
    assert!(store.saved_sessions().is_empty());
}

/// A header that is present but blank is not an identity. It would satisfy "the field is
/// there" and tell a later reader of the audit trail exactly nothing.
#[tokio::test]
async fn a_blank_operator_name_is_not_an_identity() {
    let (router, store) = running();

    let (status, body) = call(
        &router,
        post("/api/operator/session/complete", "   ", json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "OPERATOR_REQUIRED");
    assert!(store.audits().is_empty());
}

/// D5 is mandatory and safety-critical: without it a frozen screen and an empty gym look
/// the same. Every read surface therefore carries the readout, not just the live one.
#[tokio::test]
async fn every_read_surface_reports_its_freshness() {
    let (router, _) = running();

    for path in ["/api/live", "/api/coach", "/api/session", "/api/operator"] {
        let (status, body) = call(&router, get(path)).await;
        assert_eq!(status, StatusCode::OK, "GET {path}");
        let freshness = &body["freshness"];
        assert_eq!(freshness["now"], NOW.0, "{path} reports the hub's clock");
        assert_eq!(freshness["websocket_path"], "/ws", "{path}");
        assert_eq!(freshness["push_interval_ms"], 250, "{path}");
        // Nobody is listening yet, and the API says so rather than implying a live link.
        assert_eq!(freshness["subscribers"], 0, "{path}");
        // No event has happened. `null` is not zero and a screen must not draw it as fresh.
        assert!(freshness["last_event_age_ms"].is_null(), "{path}");
    }
}

#[tokio::test]
async fn the_check_in_surface_reports_its_freshness_too() {
    let (router, _) = running();

    let (status, body) = call(&router, get("/api/checkin")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["freshness"]["now"], NOW.0);
    assert_eq!(body["athletes"].as_array().expect("a roster").len(), 2);
    assert!(body["pending"].as_array().expect("a queue").is_empty());
}

/// The competition finish rule is undecided (CLAUDE.md 12, 28), so results are published
/// with no ranking and say what order they are in.
#[tokio::test]
async fn results_are_published_without_a_ranking() {
    let (router, _) = running();
    // `/result/{id}` reads from the store, not from memory, so the session has to have
    // reached it: closing the class is what puts it there.
    let (status, _) = call(
        &router,
        post("/api/operator/session/complete", "DESK", json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = call(&router, get("/api/result/s1")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["results"]["ordering"], "BIB");
    assert_eq!(body["results"]["finish_policy"]["kind"], "NOT_CONFIGURED");
    assert!(body["results"].get("ranking").is_none());
    assert!(body["freshness"]["websocket_path"].is_string());
}

#[tokio::test]
async fn results_for_a_session_the_hub_never_stored_are_a_404() {
    let (router, _) = running();

    let (status, body) = call(&router, get("/api/result/no-such-session")).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "UNKNOWN_SESSION");
}

/// A malformed body is a client error with this API's own shape, not axum's default.
#[tokio::test]
async fn a_body_that_will_not_parse_is_a_client_error() {
    let (router, _) = running();

    let (status, body) = call(
        &router,
        post("/api/checkin/bind", "DOOR TABLET", json!({ "tag_id": "" })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "INVALID_BODY");
}
