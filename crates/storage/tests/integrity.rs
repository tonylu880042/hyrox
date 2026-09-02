//! Damage detection and online backup (ADR 0012).
//!
//! The appliance runs one SQLite file on whatever flash the machine shipped with, and it is
//! switched off at the wall. Concurrency is not what corrupts a database -- WAL and
//! `synchronous=FULL` cover that (see `durability.rs`) -- but hardware and a badly taken
//! backup are. The two things asserted here are what we do about it:
//!
//! 1. a damaged file is found **on the way in**, before the hub can acknowledge anything;
//! 2. a backup can be taken while the hub is running, without copying `-wal` by hand.

use std::io::{Seek, SeekFrom, Write};
use storage::{Store, StoreError};

/// A real file, because none of this means anything in memory.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("hyrox-integrity-tests");
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let path = dir.join(name);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    path
}

fn url(path: &std::path::Path) -> String {
    format!("sqlite://{}", path.display())
}

#[tokio::test]
async fn a_healthy_database_passes_its_check() {
    let path = scratch("healthy.db");
    let store = Store::open(&url(&path)).await.expect("a store");

    store.quick_check().await.expect("a fresh database is not damaged");
}

/// The point of checking on the way in: the hub must not open a damaged file, start
/// acknowledging reads, and thereby tell the ESP32 to delete its only other copy of them
/// (ADR 0002; CLAUDE.md 15, 31).
#[tokio::test]
async fn a_damaged_database_is_refused_on_open() {
    let path = scratch("damaged.db");
    {
        let store = Store::open(&url(&path)).await.expect("a store");
        // Enough rows that the file is more than one page, so the damage lands in data
        // rather than in the header.
        for i in 0..200 {
            store
                .save_audit(&application::AuditEntry {
                    at: domain::Instant(1_000 + i),
                    operator: "TEST".into(),
                    action: "NOISE".into(),
                    subject: format!("row {i} with enough text to fill a page or two"),
                    reason: None,
                    before: None,
                    after: None,
                })
                .await
                .expect("a write");
        }
        // Fold the WAL into the main file and close the pool, so what is on disk is the
        // whole database -- the same state the machine is in when it is switched off.
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(store.pool())
            .await
            .expect("checkpoint");
        store.pool().close().await;
    }

    scribble_over_a_page(&path);

    let opened = Store::open(&url(&path)).await;

    match opened {
        Err(StoreError::Damaged(problems)) => {
            assert!(!problems.is_empty(), "a damage report names what is wrong");
        }
        Err(other) => panic!("expected a damage report, got {other}"),
        Ok(_) => panic!("a corrupted database was opened as if it were fine"),
    }
}

/// `VACUUM INTO` is SQLite's supported way to copy a live database. Copying the file with
/// `cp` while the hub is running is the classic way to produce a corrupt backup -- the
/// `-wal` alongside it holds committed transactions the main file does not.
#[tokio::test]
async fn a_backup_can_be_taken_while_the_database_is_in_use() {
    let path = scratch("live.db");
    let backup = scratch("live-backup.db");
    let store = Store::open(&url(&path)).await.expect("a store");
    store
        .save_audit(&application::AuditEntry {
            at: domain::Instant(4_000),
            operator: "DOOR TABLET".into(),
            action: "ATHLETE_ENTER".into(),
            subject: "K7QD2M".into(),
            reason: None,
            before: None,
            after: Some("陳小明".into()),
        })
        .await
        .expect("a write");

    store.backup_to(&backup).await.expect("a backup");

    // The copy opens on its own and holds the row -- including one written since the last
    // checkpoint, which is exactly what a `cp` of the main file would have missed.
    let restored = Store::open(&url(&backup)).await.expect("the backup opens");
    restored.quick_check().await.expect("the backup is not damaged");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log")
        .fetch_one(restored.pool())
        .await
        .expect("a count");
    assert_eq!(rows, 1, "the backup holds the committed row");
}

/// Refusing to overwrite is SQLite's own behaviour and we keep it: a backup that silently
/// replaces last night's is one backup, not a history.
#[tokio::test]
async fn a_backup_will_not_overwrite_one_that_is_already_there() {
    let path = scratch("twice.db");
    let backup = scratch("twice-backup.db");
    let store = Store::open(&url(&path)).await.expect("a store");
    store.backup_to(&backup).await.expect("the first backup");

    let second = store.backup_to(&backup).await;

    assert!(second.is_err(), "an existing backup file is not overwritten");
}

/// Writes rubbish into the middle of the file, past the header and the schema.
fn scribble_over_a_page(path: &std::path::Path) {
    let mut file = std::fs::OpenOptions::new().write(true).open(path).expect("the db file");
    let len = file.metadata().expect("metadata").len();
    assert!(len > 8192, "the fixture should be several pages, got {len} bytes");
    file.seek(SeekFrom::Start(len / 2)).expect("seek");
    file.write_all(&[0x5A; 2048]).expect("scribble");
    file.flush().expect("flush");
}
