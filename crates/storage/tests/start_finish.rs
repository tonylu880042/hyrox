//! ADR 0016 through SQLite: the start/finish mat's events and a missed exit must rebuild
//! after a restart exactly as they were live (CLAUDE.md 21).

use domain::{
    interpret, ReaderBinding, ReaderKey, ReaderMode, ReaderRegistration, Session, SessionMode,
};
use domain::{AthleteState, Instant};
use storage::Store;

fn reader(station: &str, mode: ReaderMode) -> ReaderBinding {
    ReaderBinding {
        station: station.into(),
        mode,
    }
}

#[tokio::test]
async fn a_race_with_a_start_a_pass_and_a_missed_exit_rebuilds_identically() {
    let store = Store::open_in_memory().await.unwrap();
    let mut session = Session::new_draft("s1", "Race", SessionMode::Competition);
    session.mark_ready().unwrap();
    session.start().unwrap();
    store.save_session(&session, Instant(0)).await.unwrap();
    store
        .save_athlete("s1", "a1", "Chen", 1, None)
        .await
        .unwrap();

    let mat = reader("WALL_BALLS", ReaderMode::StartFinish);
    let mut live = AthleteState::ready("a1", "Chen");
    for (binding, t) in [
        (mat.clone(), 1_000),
        (reader("SKIERG", ReaderMode::Entry), 241_000),
        // 2N missed
        (reader("SLED_PUSH", ReaderMode::Entry), 800_000),
        (reader("SLED_PUSH", ReaderMode::Exit), 900_000),
        (mat.clone(), 950_000), // the route passes the mat
        (reader("WALL_BALLS", ReaderMode::Entry), 1_100_000),
        (mat.clone(), 1_400_000),
    ] {
        let event = interpret(&mut live, &binding, Instant(t), &session);
        store
            .save_interpreted("s1", "a1", None, &event)
            .await
            .unwrap();
    }

    let rebuilt = store.rebuild_athletes("s1").await.unwrap();

    assert_eq!(rebuilt.len(), 1);
    assert_eq!(format!("{:?}", rebuilt[0]), format!("{live:?}"));
    assert_eq!(rebuilt[0].started_at, Some(Instant(1_000)));
    assert!(rebuilt[0].runs[0].exit_missed);
    assert_eq!(rebuilt[0].runs[2].exited_at, Some(Instant(1_400_000)));
}

#[tokio::test]
async fn a_start_finish_reader_survives_a_restart() {
    let store = Store::open_in_memory().await.unwrap();
    let key = ReaderKey::parse("a4cf128b3d91", "mat-1").unwrap();
    store
        .save_reader(&ReaderRegistration::new(
            key.clone(),
            "WALL_BALLS",
            ReaderMode::StartFinish,
        ))
        .await
        .unwrap();

    let readers = store.readers().await.unwrap();

    assert_eq!(readers.resolve(&key).unwrap().mode, ReaderMode::StartFinish);
}
