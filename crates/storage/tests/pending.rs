//! Recovery query coverage for the raw-to-interpretation handoff.

use domain::{Instant, Session, SessionMode};
use sqlx::Row;
use storage::Store;

const START: Instant = Instant(1_787_734_800_000);

#[tokio::test]
async fn pending_query_uses_the_raw_event_index_with_ten_thousand_completed_rows() {
    let store = Store::open_in_memory().await.unwrap();
    let session = Session::new_draft("s1", "Index fixture", SessionMode::Training);
    store.save_session(&session, START).await.unwrap();
    store
        .save_athlete("s1", "a1", "Alice", 1, None)
        .await
        .unwrap();

    let mut tx = store.pool().begin().await.unwrap();
    for id in 1..=10_000_i64 {
        sqlx::query(
            "INSERT INTO raw_events
                (device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind("a4cf128b3d91")
        .bind("rfid-01")
        .bind(1_i64)
        .bind(id)
        .bind(format!("TAG-{id}"))
        .bind(START.0 + id)
        .bind(START.0 + id)
        .execute(&mut *tx)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO raw_interpretation_context
                (raw_event_id, session_id, athlete_id, bound_session_id, on_roster,
                 reader_station, reader_mode, planned_json)
             VALUES (?1, 's1', 'a1', 's1', 1, 'RUN', 'ENTRY', NULL)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO interpreted_events
                (session_id, athlete_id, raw_event_id, kind, station, detected_at,
                 transition_ms, started_timing, exception_reason)
             VALUES ('s1', 'a1', ?1, 'ENTERED', 'RUN', ?2, NULL, 1, NULL)",
        )
        .bind(id)
        .bind(START.0 + id)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();

    let plan = sqlx::query(
        "EXPLAIN QUERY PLAN
         SELECT r.id
           FROM raw_events r
           JOIN raw_interpretation_context c ON c.raw_event_id = r.id
          WHERE c.session_id = 's1'
            AND NOT EXISTS (
                SELECT 1 FROM interpreted_events i WHERE i.raw_event_id = r.id
            )",
    )
    .fetch_all(store.pool())
    .await
    .unwrap();
    let details: Vec<String> = plan.iter().map(|row| row.get("detail")).collect();
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("idx_interpreted_by_raw_event")),
        "the correlated interpretation lookup must use its index: {details:?}"
    );

    let started = std::time::Instant::now();
    let pending = store.pending_raw_reads("s1").await.unwrap();
    eprintln!(
        "pending_raw_reads over 10,000 completed contexts: {:.2} ms",
        started.elapsed().as_secs_f64() * 1_000.0
    );
    assert!(
        pending.is_empty(),
        "all ten thousand contexts are interpreted"
    );
}
