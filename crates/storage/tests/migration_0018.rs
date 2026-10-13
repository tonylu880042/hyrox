//! Migration 0018 against a database that predates it (ADR 0018).
//!
//! `raw_events` is rebuilt to carry `uptime_ms` in its uniqueness, and two tables point at
//! it by id. Same recipe and same reason as `migration_0007`: the legacy schema is built
//! from the migrations before it and 0018 runs as sqlx runs it -- foreign keys on, inside a
//! transaction -- against a database with a class already in it.

use sqlx::{sqlite::SqliteConnectOptions, Executor, Row, SqlitePool};
use std::str::FromStr;

const BEFORE: [&str; 17] = [
    include_str!("../../../migrations/0001_init.sql"),
    include_str!("../../../migrations/0002_audit_log.sql"),
    include_str!("../../../migrations/0003_config_readers_bindings.sql"),
    include_str!("../../../migrations/0004_workout_templates.sql"),
    include_str!("../../../migrations/0005_walk_in_entrants.sql"),
    include_str!("../../../migrations/0006_venue_settings.sql"),
    include_str!("../../../migrations/0007_multi_tag_rounds.sql"),
    include_str!("../../../migrations/0008_device_id_is_the_bare_mac.sql"),
    include_str!("../../../migrations/0009_venue_assets.sql"),
    include_str!("../../../migrations/0010_stored_finish.sql"),
    include_str!("../../../migrations/0011_accepted_exceptions.sql"),
    include_str!("../../../migrations/0012_raw_interpretation_context.sql"),
    include_str!("../../../migrations/0013_finish_origin.sql"),
    include_str!("../../../migrations/0014_interpreted_raw_event_index.sql"),
    include_str!("../../../migrations/0015_active_session.sql"),
    include_str!("../../../migrations/0016_start_finish_mat.sql"),
    include_str!("../../../migrations/0017_zone_readers.sql"),
];
const M18: &str = include_str!("../../../migrations/0018_raw_event_uptime.sql");

/// A venue database as the build before ADR 0018 left it: reads, an interpretation and an
/// interpretation context hanging off them.
async fn legacy_db() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("an in-memory url")
        .foreign_keys(true);
    let pool = SqlitePool::connect_with(opts).await.expect("a connection");
    for sql in BEFORE {
        pool.execute(sql).await.expect("a legacy migration");
    }
    pool.execute(
        "INSERT INTO sessions (id, name, mode, status, interpreted_event_count, created_at)
            VALUES ('s1', 'THU 19:00', 'TRAINING', 'RUNNING', 1, 1000);
         INSERT INTO raw_events
            (id, device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at,
             presence)
            VALUES (7, 'a4cf128b3d91', 'rfid-01', 1, 1, 'TAG1', 1100, 1101, 'ARRIVED'),
                   (8, 'a4cf128b3d91', 'rfid-01', 1, 2, 'TAG1', 1200, 1201, 'DEPARTED');
         INSERT INTO interpreted_events
            (id, session_id, athlete_id, raw_event_id, kind, station, detected_at,
             started_timing, acknowledged_by)
            VALUES (41, 's1', 'a1', 7, 'ENTERED', 'SKIERG', 1100, 1, 'coach');
         INSERT INTO raw_interpretation_context
            (raw_event_id, session_id, athlete_id, on_roster, reader_station, reader_mode,
             reader_min_visit_ms)
            VALUES (8, 's1', 'a1', 1, 'SKIERG', 'ZONE', 15000);",
    )
    .await
    .expect("legacy rows");
    pool
}

#[tokio::test]
async fn migration_0018_carries_every_read_and_everything_hanging_off_it() {
    let pool = legacy_db().await;

    pool.execute(M18)
        .await
        .expect("0018 must apply to a database with reads in it");

    // Immutable means immutable (CLAUDE.md 19). `uptime_ms` is NULL, not zero: nobody
    // recorded it, and the store must be able to tell.
    let rows: Vec<(i64, i64, String, Option<i64>)> =
        sqlx::query("SELECT id, detected_at, presence, uptime_ms FROM raw_events ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("the reads")
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
            .collect();
    assert_eq!(
        rows,
        [
            (7, 1100, "ARRIVED".to_string(), None),
            (8, 1200, "DEPARTED".to_string(), None)
        ]
    );

    let interpretation: (i64, String) = sqlx::query_as(
        "SELECT r.id, i.acknowledged_by FROM interpreted_events i
         JOIN raw_events r ON r.id = i.raw_event_id WHERE i.id = 41",
    )
    .fetch_one(&pool)
    .await
    .expect("the interpretation still points at its read");
    assert_eq!(interpretation, (7, "coach".to_string()));

    let context: (String, i64) = sqlx::query_as(
        "SELECT c.reader_mode, c.reader_min_visit_ms FROM raw_interpretation_context c
         JOIN raw_events r ON r.id = c.raw_event_id WHERE r.id = 8",
    )
    .fetch_one(&pool)
    .await
    .expect("the context still points at its read");
    assert_eq!(context, ("ZONE".to_string(), 15000));

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
async fn after_0018_a_reused_key_is_storable_and_a_true_duplicate_is_not() {
    let pool = legacy_db().await;
    pool.execute(M18).await.expect("0018 applies");

    let insert = |uptime: i64| {
        format!(
            "INSERT INTO raw_events
                (device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at,
                 uptime_ms)
             VALUES ('a4cf128b3d91', 'rfid-01', 1, 9, 'TAGA', 1300, 1301, {uptime})"
        )
    };
    pool.execute(insert(120_345).as_str())
        .await
        .expect("a read");
    // The point of the migration: the same key and tag from a wiped device, told apart by
    // how long that device had been up.
    pool.execute(insert(120_999).as_str())
        .await
        .expect("a wiped device reusing the key must be storable");
    assert!(
        pool.execute(insert(120_345).as_str()).await.is_err(),
        "key + tag + uptime stays unique"
    );
}

/// 0007 lost this index by creating it `IF NOT EXISTS` while the old table still owned the
/// name. The check-in queue and the retroactive claim both look reads up by tag.
#[tokio::test]
async fn after_0018_reads_are_indexed_by_tag_again() {
    let pool = legacy_db().await;
    pool.execute(M18).await.expect("0018 applies");

    let owner: Option<String> = sqlx::query_scalar(
        "SELECT tbl_name FROM sqlite_master WHERE type = 'index' AND name = 'idx_raw_by_tag'",
    )
    .fetch_optional(&pool)
    .await
    .expect("the catalogue");
    assert_eq!(owner.as_deref(), Some("raw_events"));
}
