//! Session lifecycle over HTTP: DRAFT -> ARMED -> CLOSED, and back (ADR 0001 D2).
//!
//! Every refusal here is a domain invariant saying no. None of them may be a 500: an
//! operator has to be able to tell a rule from an outage.

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{armed, call, draft, get, post, put};

const DESK: &str = "FRONT DESK TABLET";

#[tokio::test]
async fn a_draft_session_arms() {
    let (router, store) = draft();

    let (status, body) = call(&router, post("/api/operator/session/arm", DESK, json!({}))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["session"]["status"], "ARMED");
    // Persisted before it was claimed, and audited under the device that did it.
    assert_eq!(store.saved_sessions()[0].status, domain::SessionStatus::Armed);
    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "SESSION_ARM");
    assert_eq!(audit.operator, DESK);
}

#[tokio::test]
async fn an_armed_session_closes() {
    let (router, store) = armed();

    let (status, body) = call(&router, post("/api/operator/session/close", DESK, json!({}))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["session"]["status"], "CLOSED");
    assert_eq!(store.audits().pop().expect("an audit").action, "SESSION_CLOSE");
}

/// A DRAFT session was never accepting events, so there is nothing to close. 409, because
/// the request is well formed and the world is simply not where the client thought.
#[tokio::test]
async fn a_draft_session_cannot_be_closed() {
    let (router, store) = draft();

    let (status, body) = call(&router, post("/api/operator/session/close", DESK, json!({}))).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "ILLEGAL_TRANSITION");
    assert!(store.audits().is_empty());
}

/// CLOSED -> ARMED is deliberately allowed (D2): a mis-tap on a busy floor must not force a
/// new session. It is a correction, so CLAUDE.md 20 wants the reason on the record.
#[tokio::test]
async fn reopening_a_closed_session_needs_a_reason() {
    let (router, store) = armed();
    call(&router, post("/api/operator/session/close", DESK, json!({}))).await;

    let (status, body) = call(&router, post("/api/operator/session/reopen", DESK, json!({}))).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "REASON_REQUIRED");
    assert!(!store.audits().iter().any(|a| a.action == "SESSION_REOPEN"));
}

#[tokio::test]
async fn a_closed_session_reopens_with_a_reason() {
    let (router, store) = armed();
    call(&router, post("/api/operator/session/close", DESK, json!({}))).await;

    let (status, body) = call(
        &router,
        post("/api/operator/session/reopen", DESK, json!({ "reason": "誤觸" })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["session"]["status"], "ARMED");
    let audit = store
        .audits()
        .into_iter()
        .find(|a| a.action == "SESSION_REOPEN")
        .expect("a reopen audit");
    assert_eq!(audit.reason.as_deref(), Some("誤觸"));
    assert_eq!(audit.operator, DESK);
}

/// `arm` refuses a CLOSED session outright: bringing one back is a correction and goes
/// through reopen, which insists on a reason.
#[tokio::test]
async fn arm_does_not_double_as_reopen() {
    let (router, _) = armed();
    call(&router, post("/api/operator/session/close", DESK, json!({}))).await;

    let (status, body) = call(&router, post("/api/operator/session/arm", DESK, json!({}))).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "REASON_REQUIRED");
}

#[tokio::test]
async fn an_armed_session_with_nothing_interpreted_returns_to_draft() {
    let (router, _) = armed();

    let (status, body) = call(&router, post("/api/operator/session/draft", DESK, json!({}))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["session"]["status"], "DRAFT");
    assert_eq!(body["config_editable"], true);
}

/// ARMED -> DRAFT is only legal while nothing has been interpreted (D2). Once a read has
/// been folded in, going back would orphan it.
#[tokio::test]
async fn an_armed_session_that_has_interpreted_events_cannot_return_to_draft() {
    let (store, mut state) = (
        std::sync::Arc::new(support::FakeStore::new()),
        support::armed_session(),
    );
    state.session.interpreted_event_count = 1;
    let (router, _) = support::hub(state, store);

    let (status, body) = call(&router, post("/api/operator/session/draft", DESK, json!({}))).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "HAS_INTERPRETED_EVENTS");
}

/// Configuration is DRAFT-only (D2), which is what makes a resumed session trustworthy
/// (ADR 0004): a running class keeps the rule it was armed under.
#[tokio::test]
async fn a_draft_session_can_be_given_a_course_and_a_finish_rule() {
    let (router, store) = draft();

    let (status, body) = call(
        &router,
        put(
            "/api/operator/config",
            DESK,
            json!({
                "course": {
                    "name": "HYROX CLASS",
                    "steps": [
                        { "station": "RUN", "target": { "kind": "DISTANCE", "meters": 400 } },
                        { "station": "SKIERG", "target": { "kind": "DISTANCE", "meters": 500 } }
                    ]
                },
                "finish_policy": { "kind": "CLASS_DURATION", "limit": 3600000 }
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["config"]["course"]["steps"].as_array().expect("steps").len(), 2);
    assert_eq!(body["config"]["finish_policy"]["kind"], "CLASS_DURATION");
    assert_eq!(store.saved_configs().len(), 1);
}

#[tokio::test]
async fn an_armed_session_cannot_be_reconfigured() {
    let (router, store) = armed();

    let (status, body) = call(
        &router,
        put(
            "/api/operator/config",
            DESK,
            json!({ "finish_policy": { "kind": "COACH_DECIDES" } }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "SESSION_NOT_EDITABLE");
    assert!(store.saved_configs().is_empty());
    // And the read surface says so up front, so a screen need not re-derive the rule.
    let (_, session) = call(&router, get("/api/session")).await;
    assert_eq!(session["config_editable"], false);
}

/// The finish policy is required in the body. It has a `Default` -- `NotConfigured` -- and
/// an omitted field falling into it would silently remove a class's finish rule.
#[tokio::test]
async fn configuring_without_a_finish_policy_is_refused() {
    let (router, store) = draft();

    let (status, body) = call(
        &router,
        put("/api/operator/config", DESK, json!({ "course": null })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "INVALID_BODY");
    assert!(store.saved_configs().is_empty());
}

/// Competition's finish rule is undecided (CLAUDE.md 12, 28). A button that stopped every
/// competitor's clock would be exactly the invented rule the project forbids.
#[tokio::test]
async fn a_class_with_no_finish_rule_cannot_be_ended_by_hand() {
    let (router, _) = armed();

    let (status, body) = call(
        &router,
        post("/api/operator/session/end-class", DESK, json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "NO_FINISH_RULE");
}

#[tokio::test]
async fn a_class_with_a_finish_rule_can_be_ended_by_hand() {
    let (router, store) = draft();
    call(
        &router,
        put(
            "/api/operator/config",
            DESK,
            json!({ "finish_policy": { "kind": "COACH_DECIDES" } }),
        ),
    )
    .await;
    call(&router, post("/api/operator/session/arm", DESK, json!({}))).await;

    let (status, body) = call(
        &router,
        post("/api/operator/session/end-class", DESK, json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    // Nobody had started, so nobody's clock was stopped -- and the answer says so rather
    // than claiming finishers.
    assert!(body["finished"].as_array().expect("a list").is_empty());
    assert!(store.audits().iter().any(|a| a.action == "CLASS_END"));
}

#[tokio::test]
async fn a_reader_is_registered_and_then_reported_with_its_freshness() {
    let (router, store) = armed();

    let (status, body) = call(
        &router,
        post(
            "/api/operator/readers",
            DESK,
            json!({
                "device_id": "esp32-a4cf128b3d91",
                "reader_id": "rfid-01",
                "station": "SKIERG",
                "zone": "STATION ROW",
                "mode": "ENTRY"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let readers = body["readers"].as_array().expect("readers");
    assert_eq!(readers.len(), 1);
    assert_eq!(readers[0]["station"], "SKIERG");
    assert_eq!(readers[0]["mode"], "ENTRY");
    // The hub has heard nothing from this board yet. `null`, not zero: an unheard device
    // must not be drawn as fresh (ADR 0001 D5).
    assert!(readers[0]["last_seen_age_ms"].is_null());
    assert_eq!(store.saved_readers().len(), 1);
}

#[tokio::test]
async fn a_reader_key_the_hub_cannot_parse_is_a_client_error() {
    let (router, store) = armed();

    let (status, body) = call(
        &router,
        post(
            "/api/operator/readers",
            DESK,
            json!({ "device_id": "not-a-mac", "reader_id": "rfid-01",
                    "station": "SKIERG", "mode": "ENTRY" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "INVALID_BODY");
    assert!(store.saved_readers().is_empty());
}

/// There is deliberately no removal use case, so there is deliberately no removal route
/// (CLAUDE.md 28): what becomes of events already attributed through a reader is a product
/// rule nobody has made.
#[tokio::test]
async fn there_is_no_route_that_deletes_a_reader() {
    let (router, _) = armed();

    let (status, _) = call(
        &router,
        support::anonymous("DELETE", "/api/operator/readers", json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

/// The inbox (D4). Voiding is destructive, so it needs a reason; naming an id that is not
/// there is a 404, never a silent success.
#[tokio::test]
async fn voiding_an_exception_needs_a_reason_and_a_real_event() {
    let (router, store) = armed();
    let id = store.seed_interpreted(
        "a1",
        domain::Interpreted::Exception {
            reason: domain::ExceptionReason::UnknownReader,
            at: support::START,
        },
    );

    let (status, body) = call(&router, get("/api/operator/exceptions")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["exceptions"][0]["reason"], "UNKNOWN_READER");
    assert_eq!(body["exceptions"][0]["interpreted_event_id"], id);

    let path = format!("/api/operator/exceptions/{id}/void");
    let (status, body) = call(&router, post(&path, DESK, json!({}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "REASON_REQUIRED");

    let (status, body) = call(
        &router,
        post("/api/operator/exceptions/999/void", DESK, json!({ "reason": "誤刷" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "UNKNOWN_EVENT");

    let (status, body) = call(&router, post(&path, DESK, json!({ "reason": "誤刷" }))).await;
    assert_eq!(status, StatusCode::OK);
    // Gone from the inbox, and the audit trail says who cleared it and why.
    assert!(body["exceptions"].as_array().expect("a list").is_empty());
    let audit = store
        .audits()
        .into_iter()
        .find(|a| a.action == "EVENT_VOID")
        .expect("a void audit");
    assert_eq!(audit.operator, DESK);
    assert_eq!(audit.reason.as_deref(), Some("誤刷"));
}

#[tokio::test]
async fn an_operator_device_may_be_named_in_chinese() {
    // ADR 0001 D1's own examples are 「櫃檯平板」 and 「教練手機」. HTTP header values are
    // nominally ASCII, so reading one with `to_str` silently turned a Chinese device name
    // into no operator at all -- the anonymous audit row D1 exists to prevent.
    let (router, store) = armed();

    let (status, _) =
        call(&router, post("/api/operator/session/close", "櫃檯平板", json!({}))).await;

    assert_eq!(status, StatusCode::OK, "a Chinese device name must be accepted");
    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.operator, "櫃檯平板", "the name must reach the audit trail verbatim");
}
