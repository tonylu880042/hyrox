//! The venue's own logo (M6 follow-up).
//!
//! A gym's screen should lead with the gym. What makes this more than a file upload is
//! that the hub serves the bytes back to every browser in the venue from its own origin,
//! so what it accepts has to be decided by content rather than by what the upload claims.

mod support;

use axum::http::StatusCode;
use support::{call, get, raw_bytes, running, upload};

const DESK: &str = "FRONT DESK TABLET";

/// A minimal but genuine PNG: the eight magic bytes are what the hub reads.
fn png() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0u8; 64]);
    bytes
}

#[tokio::test]
async fn a_venue_uploads_a_logo_and_the_screens_can_fetch_it() {
    let (router, _store) = running();

    let (status, _body) = call(
        &router,
        upload("/api/operator/logo", DESK, "image/png", png()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Read without an operator name: the projector is a screen on a wall, not an operator.
    let (code, content_type, body) = raw_bytes(&router, get("/api/logo")).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(content_type, "image/png");
    assert_eq!(
        body,
        png(),
        "the bytes served are the bytes that were uploaded"
    );
}

/// Before anybody uploads one there is no logo, and that is an answer rather than a fault:
/// the screens simply lead with the class instead.
#[tokio::test]
async fn a_venue_with_no_logo_says_so_plainly() {
    let (router, _store) = running();

    let (status, _content_type, _body) = raw_bytes(&router, get("/api/logo")).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// SVG is the format somebody reaches for first, and it is the one we will not serve: it
/// can carry script, and this hub would hand it to every screen from its own origin.
#[tokio::test]
async fn an_svg_is_refused_with_a_reason_that_explains_itself() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        upload(
            "/api/operator/logo",
            DESK,
            "image/svg+xml",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(body["error"], "UNSUPPORTED_IMAGE");
    assert!(
        body["message"].as_str().expect("a message").contains("PNG"),
        "the refusal has to say what to upload instead: {body:?}"
    );
}

/// The upload's own claim is not evidence. A file named and declared as a PNG that is not
/// one would be served back as a PNG, which is how a browser gets told to trust it.
#[tokio::test]
async fn something_that_only_claims_to_be_a_png_is_refused() {
    let (router, _store) = running();

    let (status, body) = call(
        &router,
        upload(
            "/api/operator/logo",
            DESK,
            "image/png",
            b"MZ not really".to_vec(),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(body["error"], "UNSUPPORTED_IMAGE");
}

#[tokio::test]
async fn a_photograph_sized_file_is_refused() {
    let (router, _store) = running();
    let mut huge = png();
    huge.resize(huge.len() + application::MAX_ASSET_BYTES + 1, 0u8);

    let (status, body) = call(
        &router,
        upload("/api/operator/logo", DESK, "image/png", huge),
    )
    .await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "IMAGE_TOO_LARGE");
}

#[tokio::test]
async fn uploading_a_logo_needs_an_operator_name_and_is_audited() {
    let (router, store) = running();

    let (status, body) = call(
        &router,
        support::upload_anonymous("/api/operator/logo", "image/png", png()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "OPERATOR_REQUIRED");

    call(
        &router,
        upload("/api/operator/logo", DESK, "image/png", png()),
    )
    .await;
    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "VENUE_ASSET");
    assert_eq!(audit.operator, DESK);
}

#[tokio::test]
async fn a_logo_can_be_removed_and_the_screens_go_back_to_having_none() {
    let (router, _store) = running();
    call(
        &router,
        upload("/api/operator/logo", DESK, "image/png", png()),
    )
    .await;

    let (status, _body) = call(
        &router,
        support::del("/api/operator/logo", DESK, serde_json::json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let (code, _ct, _body) = raw_bytes(&router, get("/api/logo")).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}
