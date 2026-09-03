//! Migration 0007 against a database that predates it (ADR 0014).
//!
//! `raw_events` is rebuilt to widen its uniqueness from the round to the tag, and
//! `interpreted_events` rows point at it by id. Same recipe as `migration_0004`: the legacy
//! schema is reconstructed by hand and 0007 runs exactly as sqlx runs it -- foreign keys on,
//! inside a transaction. A rebuild that passes on an empty database and fails on a venue's
//! is the failure this file exists to catch.

use sqlx::{sqlite::SqliteConnectOptions, Executor, Row, SqlitePool};
use std::str::FromStr;

const M1: &str = include_str!("../../../migrations/0001_init.sql");
const M2: &str = include_str!("../../../migrations/0002_audit_log.sql");
const M3: &str = include_str!("../../../migrations/0003_config_readers_bindings.sql");
const M4: &str = include_str!("../../../migrations/0004_workout_templates.sql");
const M5: &str = include_str!("../../../migrations/0005_walk_in_entrants.sql");
const M6: &str = include_str!("../../../migrations/0006_venue_settings.sql");
const M7: &str = include_str!("../../../migrations/0007_multi_tag_rounds.sql");

/// A database as a single-tag build left it: raw reads with interpretations hanging off them.
async fn legacy_db() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("an in-memory url")
        .foreign_keys(true);
    let pool = SqlitePool::connect_with(opts).await.expect("a connection");
    for sql in [M1, M2, M3, M4, M5, M6] {
        pool.execute(sql).await.expect("a legacy migration");
    }
    pool.execute(
        "INSERT INTO sessions (id, name, mode, status, interpreted_event_count, created_at)
            VALUES ('s1', 'THU 19:00', 'TRAINING', 'RUNNING', 1, 1000);
         INSERT INTO session_athletes (session_id, athlete_id, display_name, bib)
            VALUES ('s1', 'a1', 'CHEN YU-TING', 1);
         INSERT INTO raw_events
            (id, device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at)
            VALUES (7, 'a4cf128b3d91', 'rfid-01', 1, 1, 'TAG1', 1100, 1101),
                   (8, 'a4cf128b3d91', 'rfid-01', 1, 2, 'TAG2', 1200, 1201);
         INSERT INTO interpreted_events
            (id, session_id, athlete_id, raw_event_id, kind, station, detected_at, started_timing)
            VALUES (41, 's1', 'a1', 7, 'ENTERED', 'SKIERG', 1100, 1);",
    )
    .await
    .expect("legacy rows");
    pool
}

#[tokio::test]
async fn migration_0007_applies_to_a_database_that_already_holds_reads() {
    let pool = legacy_db().await;

    pool.execute(M7)
        .await
        .expect("0007 must apply to a database with reads in it");

    // Immutable means immutable: the rebuild copies, it does not reinterpret (CLAUDE.md 19).
    let rows: Vec<(i64, String, i64)> =
        sqlx::query("SELECT id, tag_id, detected_at FROM raw_events ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("the reads")
            .iter()
            .map(|r| (r.get("id"), r.get("tag_id"), r.get("detected_at")))
            .collect();
    assert_eq!(rows, [(7, "TAG1".into(), 1100), (8, "TAG2".into(), 1200)]);

    // Ids are preserved, so the interpretation still points at the read it came from.
    let linked: i64 = sqlx::query(
        "SELECT r.id FROM interpreted_events i JOIN raw_events r ON r.id = i.raw_event_id
         WHERE i.id = 41",
    )
    .fetch_one(&pool)
    .await
    .expect("the link must survive the rebuild")
    .get(0);
    assert_eq!(linked, 7);

    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .expect("the check runs");
    assert!(
        violations.is_empty(),
        "the rebuild must leave no dangling references"
    );
}

#[tokio::test]
async fn after_0007_one_round_can_hold_several_tags() {
    let pool = legacy_db().await;
    pool.execute(M7).await.expect("0007 applies");

    // The point of the migration: the same device/boot/sequence, two tags (ADR 0014).
    pool.execute(
        "INSERT INTO raw_events (device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at)
            VALUES ('a4cf128b3d91', 'rfid-01', 1, 9, 'TAGA', 1300, 1301),
                   ('a4cf128b3d91', 'rfid-01', 1, 9, 'TAGB', 1300, 1301);",
    )
    .await
    .expect("two tags from one inventory round must both be storable");

    // ...but the same tag twice in one round is still one read.
    let again = pool
        .execute(
            "INSERT INTO raw_events (device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at)
                VALUES ('a4cf128b3d91', 'rfid-01', 1, 9, 'TAGA', 1300, 1301);",
        )
        .await;
    assert!(
        again.is_err(),
        "device + boot + sequence + tag stays unique"
    );
}
