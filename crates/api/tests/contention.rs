//! What the ingestion path is allowed to wait for (CLAUDE.md 31).
//!
//! Every write and every live read goes through one `Mutex<LiveSession>`. That is fine while
//! the lock is only ever held for work that is bounded and in memory -- a read of the whole
//! class projects in microseconds, which is a fraction of the commit an ingested read holds
//! it for anyway. What is *not* fine is holding it across disk I/O: the store is the one
//! thing whose cost grows with the season, and a coach's screen polling every five seconds
//! must never put a query between a reader's tap and its ACK.

mod support;

use axum::http::StatusCode;
use std::time::Duration;
use support::{call, get, running};

/// The exception inbox reads the store. It must let go of the session first.
#[tokio::test]
async fn a_slow_store_read_does_not_hold_up_the_rest_of_the_hub() {
    let (router, store) = running();
    let gate = store.park_exceptions();

    // The settings screen asks for the exception inbox, and the store does not answer.
    let inbox = tokio::spawn({
        let router = router.clone();
        async move { call(&router, get("/api/operator/exceptions")).await }
    });
    tokio::task::yield_now().await;

    // Meanwhile the live screen -- the same lock the MQTT ingestion path takes -- is served.
    let live = tokio::time::timeout(Duration::from_secs(2), call(&router, get("/api/live")));

    let (status, _body) = live.await.expect("the hub was blocked behind a store read");
    assert_eq!(status, StatusCode::OK);

    gate.notify_one();
    let (status, _) = inbox.await.expect("the inbox task");
    assert_eq!(status, StatusCode::OK);
}
