//! Self sign-up and the entry code (ADR 0011).
//!
//! A mock race takes entries from people the gym has never seen. They register on their own
//! phone, get six characters back, and that value is their athlete id, their QR, and the
//! number they look their result up with. The tests here are about the boundary: this is
//! the one unauthenticated write on the hub, so what it *cannot* do matters as much as what
//! it can.

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{anonymous, call, get, running};

fn sign_up(name: &str) -> axum::http::Request<axum::body::Body> {
    anonymous(
        "POST",
        "/api/checkin/signup",
        json!({ "display_name": name }),
    )
}

#[tokio::test]
async fn an_entrant_registers_themselves_and_is_handed_a_code() {
    let (router, store) = running();

    let (status, body) = call(&router, sign_up("陳小明")).await;

    assert_eq!(status, StatusCode::OK);
    let code = body["code"].as_str().expect("a code");
    assert!(
        domain::EntryCode::parse(code).is_ok(),
        "{code:?} should be an entry code"
    );
    assert_eq!(body["display_name"], "陳小明");
    assert!(
        body["bib"].as_i64().is_some(),
        "an entrant is given a number to wear"
    );

    let saved = store.saved_athletes().pop().expect("a roster row");
    assert_eq!(
        saved.athlete_id, code,
        "the code is the athlete id, not a second number"
    );
    assert_eq!(saved.member_id, None);
}

/// D1 refuses an anonymous write everywhere else, because an audit row naming nobody looks
/// like a record and is not one. Here there genuinely is no device, so the row names the
/// act instead -- and it must say so rather than borrow a tablet's name.
#[tokio::test]
async fn self_registration_is_audited_as_self_registration() {
    let (router, store) = running();

    call(&router, sign_up("陳小明")).await;

    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "ATHLETE_ENTER");
    assert_eq!(audit.operator, "SELF SIGN-UP");
}

#[tokio::test]
async fn a_blank_name_is_refused() {
    let (router, _store) = running();

    let (status, body) = call(&router, sign_up("   ")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "NAME_REQUIRED");
}

/// Printed bibs are the desk's to hand out, and a member id is a claim nobody checked.
/// Extra fields are ignored rather than honoured.
#[tokio::test]
async fn signing_up_cannot_claim_a_bib_or_a_membership() {
    let (router, store) = running();
    // The desk hands 77 to somebody, the way a printed bib is handed out.
    call(
        &router,
        support::post(
            "/api/checkin/entrants",
            "DOOR TABLET",
            json!({ "display_name": "王淑芬", "bib": 77 }),
        ),
    )
    .await;

    let (status, _body) = call(
        &router,
        anonymous(
            "POST",
            "/api/checkin/signup",
            json!({ "display_name": "陳小明", "bib": 77, "member_id": "M-1042" }),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the extra fields are ignored, not an error"
    );
    let saved = store.saved_athletes().pop().expect("a roster row");
    assert_eq!(
        saved.member_id, None,
        "an unauthenticated route cannot claim a membership"
    );
    assert_ne!(saved.bib, 77, "77 belongs to whoever the desk gave it to");
}

/// Signing up is inert until a helper hands over a band: the surface's other writes still
/// require the desk's device name.
#[tokio::test]
async fn signing_up_does_not_open_the_rest_of_the_check_in_surface() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        anonymous(
            "POST",
            "/api/checkin/bind",
            json!({ "tag_id": "E280117000001234", "athlete_id": "a1" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "OPERATOR_REQUIRED");
}

// --- the entrant's own two reads ------------------------------------------------------

#[tokio::test]
async fn an_entrant_reads_their_own_row_by_code() {
    let (router, _store) = running();
    let (_, signed_up) = call(&router, sign_up("陳小明")).await;
    let code = signed_up["code"].as_str().expect("a code").to_string();

    let (status, body) = call(&router, get(&format!("/api/entry/{code}"))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["code"], code);
    assert_eq!(body["row"]["name"], "陳小明");
    assert_eq!(body["row"]["athlete_id"], code);
    // Whether the class ranks at all, so a blank place reads as "no places here" rather
    // than "you came nowhere" (ADR 0010).
    assert!(body["ordering"].is_string());
}

#[tokio::test]
async fn a_code_is_read_the_way_a_person_types_it() {
    let (router, _store) = running();
    let (_, signed_up) = call(&router, sign_up("陳小明")).await;
    let code = signed_up["code"].as_str().expect("a code").to_lowercase();

    let (status, _body) = call(&router, get(&format!("/api/entry/{code}"))).await;

    assert_eq!(status, StatusCode::OK, "lower case is the same code");
}

#[tokio::test]
async fn an_unknown_code_is_a_clear_no() {
    let (router, _store) = running();

    let (status, body) = call(&router, get("/api/entry/K7QD2M")).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "UNKNOWN_ENTRY");
}

#[tokio::test]
async fn something_that_is_not_a_code_is_refused_before_any_lookup() {
    let (router, _store) = running();

    let (status, body) = call(&router, get("/api/entry/not-a-code")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "INVALID_BODY");
}

/// The QR carries the six characters and nothing else. A desk scanner is a keyboard: it
/// has to type a code into a search box, and one that typed a whole URL would be useless.
#[tokio::test]
async fn the_hub_draws_the_entry_qr_itself() {
    let (router, _store) = running();
    let (_, signed_up) = call(&router, sign_up("陳小明")).await;
    let code = signed_up["code"].as_str().expect("a code").to_string();

    let response = support::raw(&router, get(&format!("/api/entry/{code}/qr.svg"))).await;

    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        response.1, "image/svg+xml; charset=utf-8",
        "served by the hub, not a CDN"
    );
    assert!(response.2.starts_with("<?xml") || response.2.starts_with("<svg"));
}
