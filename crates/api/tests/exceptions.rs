//! Clearing the exception inbox without erasing anything (ADR 0001 D4).
//!
//! D4 named three actions and only one was built: **void**. Void is the destructive one --
//! it takes the interpretation out of every replay -- and using it on an exception that is
//! simply harmless is the wrong tool. A duplicate tap somebody has looked at and judged
//! unimportant is still a true record of what the reader saw; what it stops being is
//! somebody's outstanding work.
//!
//! So the second action is *accept as is*: the row stays, the replay is untouched, and the
//! inbox and its badge stop counting it. An operator with a clean inbox is one who will
//! still notice the next exception (CLAUDE.md 31 principle 6).

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{call, get, post, running};

const DESK: &str = "FRONT DESK TABLET";

fn an_exception(store: &support::FakeStore) -> i64 {
    store.seed_interpreted(
        "a1",
        domain::Interpreted::Exception {
            reason: domain::ExceptionReason::ImpossibleTransition,
            at: support::START,
        },
    )
}

#[tokio::test]
async fn an_accepted_exception_leaves_the_inbox_without_being_erased() {
    let (router, store) = running();
    let id = an_exception(&store);

    let path = format!("/api/operator/exceptions/{id}/accept");
    let (status, body) = call(&router, post(&path, DESK, json!({}))).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body["exceptions"].as_array().expect("a list").is_empty(),
        "the inbox is clear"
    );
    // Nothing was voided: the interpretation is still in the log, which is the whole
    // difference between this and the void button next to it.
    assert!(
        store.voided().is_empty(),
        "accepting must not void anything"
    );
    let audit = store
        .audits()
        .into_iter()
        .find(|a| a.action == "EXCEPTION_ACCEPT")
        .expect("an accept audit");
    assert_eq!(audit.operator, DESK);
    assert_eq!(audit.subject, id.to_string());
}

/// A reason is welcome but not required: nothing is being changed or removed, so demanding
/// one would only teach operators to type "ok" thirty times an evening (ADR 0001 D1 asks for
/// reasons on the destructive actions).
#[tokio::test]
async fn accepting_takes_a_reason_but_does_not_insist_on_one() {
    let (router, store) = running();
    let id = an_exception(&store);

    let path = format!("/api/operator/exceptions/{id}/accept");
    let (status, _) = call(&router, post(&path, DESK, json!({ "reason": "重複靠卡" }))).await;

    assert_eq!(status, StatusCode::OK);
    let audit = store
        .audits()
        .into_iter()
        .find(|a| a.action == "EXCEPTION_ACCEPT")
        .expect("an accept audit");
    assert_eq!(audit.reason.as_deref(), Some("重複靠卡"));
}

/// The number on the big screen and the number in the inbox are the same number, and both
/// mean "still somebody's problem".
#[tokio::test]
async fn the_badge_stops_counting_an_accepted_exception() {
    let store = std::sync::Arc::new(support::FakeStore::new());
    let mut state = support::running_session();
    // A class that has already had one exception, the way a resumed one comes back.
    state.exception_count = 1;
    let (router, store) = support::hub(state, store);
    let id = an_exception(&store);

    let (_, before) = call(&router, get("/api/live")).await;
    assert_eq!(
        before["snapshot"]["exceptions"], 1,
        "the screen shows the outstanding one"
    );

    let path = format!("/api/operator/exceptions/{id}/accept");
    call(&router, post(&path, DESK, json!({}))).await;

    let (_, after) = call(&router, get("/api/live")).await;
    assert_eq!(
        after["snapshot"]["exceptions"], 0,
        "the badge follows the inbox"
    );
}

#[tokio::test]
async fn accepting_something_that_is_not_there_is_a_404() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        post("/api/operator/exceptions/999/accept", DESK, json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "UNKNOWN_EVENT");
}

#[tokio::test]
async fn accepting_needs_an_operator_name_like_every_other_write() {
    let (router, store) = running();
    let id = an_exception(&store);

    let path = format!("/api/operator/exceptions/{id}/accept");
    let (status, body) = call(&router, support::anonymous("POST", &path, json!({}))).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "OPERATOR_REQUIRED");
}

#[tokio::test]
async fn reinterpreting_an_exception_voids_it_and_commits_new_interpretation() {
    let (router, store) = running();
    let id = an_exception(&store);

    let path = format!("/api/operator/exceptions/{id}/reinterpret");
    let (status, body) = call(
        &router,
        post(
            &path,
            DESK,
            json!({
                "station": "RUN",
                "mode": "ENTRY",
                "reason": "手動改判進站"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body["exceptions"].as_array().expect("a list").is_empty(),
        "the inbox is cleared"
    );
    assert_eq!(store.voided(), vec![id], "original exception was voided");

    let audit = store
        .audits()
        .into_iter()
        .find(|a| a.action == "EVENT_REINTERPRET")
        .expect("a reinterpret audit");
    assert_eq!(audit.operator, DESK);
    assert_eq!(audit.subject, id.to_string());
    assert_eq!(audit.reason.as_deref(), Some("手動改判進站"));
}

#[tokio::test]
async fn reinterpreting_an_exception_demands_a_reason() {
    let (router, store) = running();
    let id = an_exception(&store);

    let path = format!("/api/operator/exceptions/{id}/reinterpret");
    let (status, body) = call(
        &router,
        post(
            &path,
            DESK,
            json!({
                "station": "RUN",
                "mode": "ENTRY"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "REASON_REQUIRED");
}

#[tokio::test]
async fn reinterpreting_an_exception_needs_an_operator_name() {
    let (router, store) = running();
    let id = an_exception(&store);

    let path = format!("/api/operator/exceptions/{id}/reinterpret");
    let (status, body) = call(
        &router,
        support::anonymous(
            "POST",
            &path,
            json!({
                "station": "RUN",
                "mode": "ENTRY",
                "reason": "無操作者"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "OPERATOR_REQUIRED");
}

#[tokio::test]
async fn reinterpreting_something_that_is_not_there_is_a_404() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        post(
            "/api/operator/exceptions/999/reinterpret",
            DESK,
            json!({
                "station": "RUN",
                "mode": "ENTRY",
                "reason": "不存在的事件"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "UNKNOWN_EVENT");
}
