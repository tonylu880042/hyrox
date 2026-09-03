//! Migration 0008 against a database written by a prefixed build (ADR 0015).
//!
//! The failure this guards is silent: leave `readers` on its old keys and every read from
//! the wall files as UNKNOWN_READER, with no error anywhere to say why.

use sqlx::{sqlite::SqliteConnectOptions, Executor, Row, SqlitePool};
use std::str::FromStr;

const MIGRATIONS: [&str; 7] = [
    include_str!("../../../migrations/0001_init.sql"),
    include_str!("../../../migrations/0002_audit_log.sql"),
    include_str!("../../../migrations/0003_config_readers_bindings.sql"),
    include_str!("../../../migrations/0004_workout_templates.sql"),
    include_str!("../../../migrations/0005_walk_in_entrants.sql"),
    include_str!("../../../migrations/0006_venue_settings.sql"),
    include_str!("../../../migrations/0007_multi_tag_rounds.sql"),
];
const M8: &str = include_str!("../../../migrations/0008_device_id_is_the_bare_mac.sql");

/// A database as a prefixed build left it: reads and a reader map, both on `esp32-` ids.
async fn prefixed_db() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("an in-memory url")
        .foreign_keys(true);
    let pool = SqlitePool::connect_with(opts).await.expect("a connection");
    for sql in MIGRATIONS {
        pool.execute(sql).await.expect("a legacy migration");
    }
    pool.execute(
        "INSERT INTO raw_events
            (id, device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at)
            VALUES (7, 'esp32-a4cf128b3d91', 'rfid-01', 1, 1, 'TAG1', 1100, 1101);
         INSERT INTO readers (device_id, reader_id, station, zone, mode)
            VALUES ('esp32-a4cf128b3d91', 'rfid-01', 'SKIERG', NULL, 'ENTRY');",
    )
    .await
    .expect("legacy rows");
    pool
}

async fn one_string(pool: &SqlitePool, sql: &str) -> String {
    sqlx::query(sql)
        .fetch_one(pool)
        .await
        .expect("a row")
        .get::<String, _>(0)
}

#[tokio::test]
async fn migration_0008_strips_the_prefix_from_stored_ids() {
    let pool = prefixed_db().await;

    pool.execute(M8).await.expect("0008 applies");

    assert_eq!(
        one_string(&pool, "SELECT device_id FROM raw_events WHERE id = 7").await,
        "a4cf128b3d91"
    );
    assert_eq!(
        one_string(&pool, "SELECT device_id FROM readers").await,
        "a4cf128b3d91"
    );
}

#[tokio::test]
async fn migration_0008_leaves_an_already_migrated_database_alone() {
    let pool = prefixed_db().await;
    pool.execute(M8).await.expect("0008 applies");
    // Running it twice must not eat six more characters off an already-bare id.
    pool.execute(M8).await.expect("0008 is idempotent");

    assert_eq!(
        one_string(&pool, "SELECT device_id FROM raw_events WHERE id = 7").await,
        "a4cf128b3d91"
    );
    assert_eq!(
        one_string(&pool, "SELECT device_id FROM readers").await,
        "a4cf128b3d91"
    );
}
