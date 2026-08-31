//! Templates, exercises and machines in SQLite, and the 0004 migration (ADR 0008).

use domain::{
    Exercise, ExerciseCategory, ExerciseLibrary, Instant, PhysicalStation, Session, SessionMode,
    SessionStatus, Target, TargetType, TemplateCategory, TemplateSource, Unit, Weight, WeightUnit,
    WorkoutBlock, WorkoutExercise, WorkoutTemplate,
};
use storage::Store;

async fn store() -> Store {
    Store::open_in_memory().await.expect("an in-memory store")
}

fn ex(code: &str, value: u32, unit: Unit) -> WorkoutExercise {
    let lib = ExerciseLibrary::preset();
    WorkoutExercise::new(code, Target::new(lib.get(code).unwrap(), value, unit).unwrap())
}

fn template() -> WorkoutTemplate {
    WorkoutTemplate::system("sys1", "HYROX Engine 800", TemplateCategory::Engine)
        .with_description("Cardio / endurance")
        .with_estimated_duration(50)
        .with_difficulty("INTERMEDIATE")
        .with_block(
            WorkoutBlock::rounds("Main", 3)
                .with_rest(domain::Duration(60_000))
                .with_exercises(vec![
                    ex("RUN", 800, Unit::Meter),
                    ex("WALL_BALL", 50, Unit::Reps)
                        .with_weight(Weight::new(4, WeightUnit::Kilogram))
                        .with_notes("chest to wall"),
                ]),
        )
}

#[tokio::test]
async fn a_template_round_trips_whole() {
    let store = store().await;
    let t = template();

    store.save_template(&t).await.expect("saved");
    let back = store.template("sys1").await.expect("read").expect("a stored template");

    assert_eq!(back, t, "every field, blocks included, comes back as it went in");
}

#[tokio::test]
async fn saving_a_template_twice_replaces_it_rather_than_duplicating() {
    let store = store().await;
    store.save_template(&template()).await.unwrap();
    let mut edited = template();
    edited.name = "Renamed".into();
    edited.version = 2;
    store.save_template(&edited).await.unwrap();

    let all = store.templates().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "Renamed");
    assert_eq!(all[0].version, 2);
}

#[tokio::test]
async fn a_template_that_was_never_stored_is_none_not_an_error() {
    assert!(store().await.template("nope").await.expect("a clean read").is_none());
}

#[tokio::test]
async fn deleting_reports_whether_a_row_went() {
    let store = store().await;
    store.save_template(&template()).await.unwrap();

    assert!(store.delete_template("sys1").await.unwrap());
    assert!(!store.delete_template("sys1").await.unwrap(), "a second delete removes nothing");
    assert!(store.templates().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_source_survives_storage_so_the_read_only_rule_survives_a_restart() {
    let store = store().await;
    store.save_template(&template()).await.unwrap();
    store
        .save_template(&WorkoutTemplate::new("t2", "Mine", TemplateCategory::Custom))
        .await
        .unwrap();

    let stored = store.templates().await.unwrap();
    let system = stored.iter().find(|t| t.id == "sys1").unwrap();
    let coach = stored.iter().find(|t| t.id == "t2").unwrap();

    assert_eq!(system.source, TemplateSource::System);
    assert!(!system.is_editable());
    assert_eq!(coach.source, TemplateSource::Coach);
    assert!(coach.is_editable());
}

// --- exercises ----------------------------------------------------------------------------

#[tokio::test]
async fn the_exercise_library_round_trips() {
    let store = store().await;
    for e in ExerciseLibrary::preset().iter() {
        store.save_exercise(e).await.expect("saved");
    }

    let back = store.exercises().await.expect("read");

    assert_eq!(back.len(), 9);
    let wall_ball = back.get("WALL_BALL").expect("wall ball");
    assert_eq!(wall_ball.station_key, "WALL BALLS");
    assert_eq!(wall_ball.default_target_type, TargetType::Reps);
    assert_eq!(wall_ball.supported_target_types, vec![TargetType::Reps, TargetType::Time]);
    assert_eq!(back.get("ROWERG").unwrap().category, ExerciseCategory::Erg);
}

#[tokio::test]
async fn saving_an_exercise_twice_replaces_it() {
    let store = store().await;
    let mut e: Exercise = ExerciseLibrary::preset().get("RUN").unwrap().clone();
    store.save_exercise(&e).await.unwrap();
    e.display_name = "Runnning".into();
    store.save_exercise(&e).await.unwrap();

    assert_eq!(store.exercises().await.unwrap().len(), 1);
}

// --- machines -------------------------------------------------------------------------------

#[tokio::test]
async fn stations_round_trip_and_keep_their_capability() {
    let store = store().await;
    for e in ExerciseLibrary::preset().iter() {
        store.save_exercise(e).await.unwrap();
    }
    for id in ["ROW_01", "ROW_02"] {
        store
            .save_station(&PhysicalStation::new(id, "ROWERG", id).with_zone("ERG ROW"))
            .await
            .expect("saved");
    }

    let map = store.stations().await.expect("read");

    assert_eq!(map.serving("ROWERG").count(), 2);
    assert!(map.can_serve("ROW_02", "ROWERG"));
    assert_eq!(map.get("ROW_01").unwrap().zone.as_deref(), Some("ERG ROW"));
}

// --- migration 0004 (the session-status rewrite) ----------------------------------------------

#[tokio::test]
async fn the_new_session_states_round_trip() {
    let store = store().await;
    for (id, apply) in [
        ("s-ready", 1),
        ("s-running", 2),
        ("s-paused", 3),
        ("s-completed", 4),
        ("s-cancelled", 5),
    ] {
        let mut s = Session::new_draft(id, id, SessionMode::Training);
        s.mark_ready().unwrap();
        if apply >= 2 {
            s.start().unwrap();
        }
        if apply >= 3 {
            s.pause(Instant(5_000)).unwrap();
        }
        match apply {
            4 => s.complete().unwrap(),
            5 => s.cancel().unwrap(),
            _ => {}
        }
        store.save_session(&s, Instant(1_000)).await.expect("saved");
    }

    for (id, expected) in [
        ("s-ready", SessionStatus::Ready),
        ("s-running", SessionStatus::Running),
        ("s-paused", SessionStatus::Paused),
        ("s-completed", SessionStatus::Completed),
        ("s-cancelled", SessionStatus::Cancelled),
    ] {
        let back = store.session(id).await.unwrap().expect("a stored session");
        assert_eq!(back.status, expected, "{id}");
    }
}

/// The whole point of persisting the pause: a hub that restarts mid-pause must come back
/// paused with its accumulated total, not hand the class back the time it stood still.
#[tokio::test]
async fn a_pause_survives_a_reload() {
    let store = store().await;
    let mut s = Session::new_draft("s1", "Class", SessionMode::Training);
    s.mark_ready().unwrap();
    s.start().unwrap();
    s.pause(Instant(10_000)).unwrap();
    s.resume(Instant(25_000)).unwrap();
    s.pause(Instant(30_000)).unwrap();
    store.save_session(&s, Instant(0)).await.unwrap();

    let back = store.session("s1").await.unwrap().unwrap();

    assert_eq!(back.status, SessionStatus::Paused);
    assert_eq!(back.paused_total, domain::Duration(15_000));
    assert_eq!(back.paused_since, Some(Instant(30_000)));
    assert_eq!(back.clock(Instant(0)).elapsed(Instant(60_000)), domain::Duration(15_000));
}

/// A paused class is still today's class: `active_session` has to pick it back up, or a
/// restart during a coffee break would silently start a new one.
#[tokio::test]
async fn a_paused_session_is_still_the_active_one() {
    let store = store().await;
    let mut old = Session::new_draft("old", "Yesterday", SessionMode::Training);
    old.mark_ready().unwrap();
    old.start().unwrap();
    old.complete().unwrap();
    store.save_session(&old, Instant(0)).await.unwrap();

    let mut paused = Session::new_draft("today", "Today", SessionMode::Training);
    paused.mark_ready().unwrap();
    paused.start().unwrap();
    paused.pause(Instant(10_000)).unwrap();
    store.save_session(&paused, Instant(1)).await.unwrap();

    let active = store.active_session().await.unwrap().expect("a session to resume");
    assert_eq!(active.id, "today");
}
