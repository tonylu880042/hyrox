//! Migration 0004 against a database that predates it (ADR 0008).
//!
//! This is the one destructive step in the whole feature: `sessions` is rebuilt to widen its
//! status CHECK, and three tables hold rows pointing at it. Two earlier drafts of the
//! migration passed every test on a fresh database and failed on a real one, so the legacy
//! schema is reconstructed here by hand and 0004 is run against it exactly as sqlx runs it
//! -- foreign keys on, inside a transaction.

use sqlx::{sqlite::SqliteConnectOptions, Executor, Row, SqlitePool};
use std::str::FromStr;

const M1: &str = include_str!("../../../migrations/0001_init.sql");
const M2: &str = include_str!("../../../migrations/0002_audit_log.sql");
const M3: &str = include_str!("../../../migrations/0003_config_readers_bindings.sql");
const M4: &str = include_str!("../../../migrations/0004_workout_templates.sql");

/// A database as a build before ADR 0008 left it: an ARMED session, a CLOSED one, and rows
/// in all three tables that reference them.
async fn legacy_db() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("an in-memory url")
        .foreign_keys(true);
    let pool = SqlitePool::connect_with(opts).await.expect("a connection");
    for sql in [M1, M2, M3] {
        pool.execute(sql).await.expect("a legacy migration");
    }
    pool.execute(
        "INSERT INTO sessions (id, name, mode, status, interpreted_event_count, created_at)
            VALUES ('s1', 'THU 19:00', 'TRAINING', 'ARMED', 2, 1000),
                   ('s0', 'WED 19:00', 'TRAINING', 'CLOSED', 0, 500),
                   ('s2', 'FRI 19:00', 'TRAINING', 'DRAFT', 0, 2000);
         INSERT INTO session_athletes VALUES ('s1', 'a1', 'CHEN YU-TING', 1);
         INSERT INTO session_configs VALUES ('s1', '{\"session_id\":\"s1\"}');
         INSERT INTO raw_events
            (device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at)
            VALUES ('a4cf128b3d91', 'rfid-01', 1, 1, 'TAG1', 1100, 1101);
         INSERT INTO interpreted_events
            (id, session_id, athlete_id, raw_event_id, kind, station, detected_at, started_timing)
            VALUES (41, 's1', 'a1', 1, 'ENTERED', 'SKIERG', 1100, 1),
                   (42, 's1', 'a1', NULL, 'EXITED', 'SKIERG', 1200, 0);",
    )
    .await
    .expect("legacy rows");
    pool
}

async fn scalar(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query(sql).fetch_one(pool).await.expect("a row").get::<i64, _>(0)
}

#[tokio::test]
async fn migration_0004_applies_to_a_database_that_already_holds_events() {
    let pool = legacy_db().await;

    pool.execute(M4).await.expect("0004 must apply to a database with events in it");

    // The statuses were translated, not dropped.
    let statuses: Vec<(String, String)> =
        sqlx::query("SELECT id, status FROM sessions ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("sessions")
            .into_iter()
            .map(|r| (r.get("id"), r.get("status")))
            .collect();
    assert_eq!(
        statuses,
        [
            ("s0".to_string(), "COMPLETED".to_string()),
            ("s1".to_string(), "RUNNING".to_string()),
            ("s2".to_string(), "DRAFT".to_string()),
        ]
    );

    // Nothing was lost on the way through.
    assert_eq!(scalar(&pool, "SELECT count(*) FROM interpreted_events").await, 2);
    assert_eq!(scalar(&pool, "SELECT count(*) FROM raw_events").await, 1);
    assert_eq!(scalar(&pool, "SELECT count(*) FROM session_athletes").await, 1);
    assert_eq!(scalar(&pool, "SELECT count(*) FROM session_configs").await, 1);

    // Interpreted row ids are named by audit records and by the void use case, so they must
    // survive the rebuild unchanged.
    assert_eq!(scalar(&pool, "SELECT min(id) FROM interpreted_events").await, 41);
    assert_eq!(scalar(&pool, "SELECT max(id) FROM interpreted_events").await, 42);

    // And the link back to the immutable raw row still resolves.
    assert_eq!(
        scalar(&pool, "SELECT raw_event_id FROM interpreted_events WHERE id = 41").await,
        1
    );
}

#[test]
fn the_migration_does_not_touch_the_immutable_event_tables() {
    // Belt and braces on CLAUDE.md 19: 0004 may rebuild `interpreted_events` to repoint one
    // foreign key, but it must never rebuild, alter or delete from `raw_events`.
    assert!(!M4.contains("DROP TABLE raw_events"));
    assert!(!M4.contains("ALTER TABLE raw_events"));
    assert!(!M4.contains("DELETE FROM"));
}

#[tokio::test]
async fn no_scaffolding_tables_are_left_behind() {
    let pool = legacy_db().await;
    pool.execute(M4).await.expect("applied");

    let leftovers: Vec<String> = sqlx::query(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND (name LIKE '%\\_old' ESCAPE '\\' OR name LIKE '%\\_new' ESCAPE '\\')",
    )
    .fetch_all(&pool)
    .await
    .expect("a listing")
    .into_iter()
    .map(|r| r.get::<String, _>("name"))
    .collect();

    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

#[tokio::test]
async fn every_child_row_still_resolves_to_its_session() {
    let pool = legacy_db().await;
    pool.execute(M4).await.expect("applied");

    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .expect("a check");

    assert!(violations.is_empty(), "{} dangling references", violations.len());
}

/// The new vocabulary has to be *permitted*, not merely written: a CHECK constraint that
/// was not widened would refuse the first pause.
#[tokio::test]
async fn the_widened_check_accepts_every_new_state() {
    let pool = legacy_db().await;
    pool.execute(M4).await.expect("applied");

    for status in ["DRAFT", "READY", "RUNNING", "PAUSED", "COMPLETED", "CANCELLED"] {
        sqlx::query(
            "INSERT INTO sessions (id, name, mode, status, interpreted_event_count, created_at)
             VALUES (?1, 'x', 'TRAINING', ?2, 0, 0)",
        )
        .bind(format!("s-{status}"))
        .bind(status)
        .execute(&pool)
        .await
        .unwrap_or_else(|e| panic!("{status} must be a legal status: {e}"));
    }

    let refused = sqlx::query(
        "INSERT INTO sessions (id, name, mode, status, interpreted_event_count, created_at)
         VALUES ('s-bad', 'x', 'TRAINING', 'ARMED', 0, 0)",
    )
    .execute(&pool)
    .await;
    assert!(refused.is_err(), "the retired vocabulary must not come back");
}
