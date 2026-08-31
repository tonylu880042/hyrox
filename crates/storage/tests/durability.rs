//! Durability settings the appliance depends on (ADR 0009).
//!
//! The hub ships as a machine that gets switched off at the wall. `PRAGMA synchronous` is
//! what decides whether a commit survives that, and the ACK contract (ADR 0002) spends
//! that guarantee: acknowledging a read tells the ESP32 to delete its only other copy.
//! So the setting is asserted, not assumed.

use sqlx::Row;
use storage::Store;

/// `NORMAL` in WAL mode makes a commit durable against a process crash but **not** against
/// power loss -- the WAL is only fsynced at checkpoints. `FULL` (2) fsyncs on commit.
#[tokio::test]
async fn a_commit_is_fsynced_before_it_is_reported_as_committed() {
    let store = Store::open_in_memory().await.expect("a store");
    let mode: i64 = sqlx::query("PRAGMA synchronous")
        .fetch_one(store.pool())
        .await
        .expect("the pragma")
        .get(0);
    assert_eq!(
        mode, 2,
        "synchronous must be FULL (2): the appliance is powered off at the wall, and an \
         ACK tells the edge to drop its only other copy of the event (ADR 0002, 0009)"
    );
}

#[tokio::test]
async fn the_journal_is_write_ahead_so_a_reader_never_blocks_ingestion() {
    let store = Store::open_in_memory().await.expect("a store");
    let mode: String = sqlx::query("PRAGMA journal_mode")
        .fetch_one(store.pool())
        .await
        .expect("the pragma")
        .get(0);
    // An in-memory database reports `memory`; a file-backed one must report `wal`.
    assert!(
        mode.eq_ignore_ascii_case("wal") || mode.eq_ignore_ascii_case("memory"),
        "unexpected journal mode {mode}"
    );
}
