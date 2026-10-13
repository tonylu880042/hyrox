//! ADR 0017 end to end over SQLite: a zone reader's arrivals and departures are stored as
//! sent, interpreted into a station visit, and survive a restart and a late check-in.

use application::{ingest_read, register_reader, LiveSession, OperatorCommand};
use contract::{EdgeEvent, Presence, ReceivedEvent};
use domain::{
    Duration, Instant, ReaderKey, ReaderMode, ReaderRegistration, Session, SessionConfig,
    SessionMode, TagId,
};
use storage::Store;

const START: Instant = Instant(1_000_000);
const DEVICE: &str = "a4cf128b3d91";

async fn running(store: &Store) -> LiveSession {
    let mut session = Session::new_draft("s1", "Race", SessionMode::Competition);
    session.mark_ready().unwrap();
    session.start().unwrap();
    store.save_session(&session, START).await.unwrap();
    let config = SessionConfig::new("s1");
    store.save_session_config(&config).await.unwrap();
    store
        .save_athlete("s1", "a1", "Chen", 1, None)
        .await
        .unwrap();
    LiveSession::new(session, config, START)
        .with_athletes(store.rebuild_athletes("s1").await.unwrap())
}

fn command() -> OperatorCommand {
    OperatorCommand::new("DESK", START).with_reason("venue setup")
}

async fn zone_reader(live: &mut LiveSession, store: &Store, min_visit: Duration) {
    let registration = ReaderRegistration::new(
        ReaderKey::parse(DEVICE, "zone-1").unwrap(),
        "SKIERG",
        ReaderMode::Zone,
    )
    .with_min_visit(min_visit);
    register_reader(live, store, &registration, &command())
        .await
        .unwrap();
}

async fn bind(live: &mut LiveSession, store: &Store, at: i64) {
    // A binding holds from its own moment; check-in claims only reads taken before it
    // (ADR 0001 D3).
    let now = OperatorCommand::new("DESK", Instant(START.0 + at)).with_reason("check-in");
    application::checkin::bind_tag(live, store, &TagId::parse("TAG-1").unwrap(), "a1", &now)
        .await
        .unwrap();
}

async fn send(live: &mut LiveSession, store: &Store, sequence: i64, t: i64, presence: Presence) {
    let event = EdgeEvent {
        device_id: contract::DeviceId::from_mac_str(DEVICE).unwrap(),
        reader_id: contract::ReaderId::parse("zone-1").unwrap(),
        boot_id: 1,
        sequence,
        tag_id: vec!["TAG-1".into()],
        detected_at: START.0 + t,
        uptime_ms: t,
        presence,
    };
    ingest_read(live, store, &ReceivedEvent::new(event, START.0 + t + 8_000))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_arrival_and_a_departure_make_a_visit_that_survives_a_restart() {
    let store = Store::open_in_memory().await.unwrap();
    let mut live = running(&store).await;
    zone_reader(&mut live, &store, Duration(15_000)).await;
    bind(&mut live, &store, 0).await;

    send(&mut live, &store, 1, 10_000, Presence::Arrived).await;
    send(&mut live, &store, 2, 250_000, Presence::Departed).await;

    let rebuilt = store.rebuild_athletes("s1").await.unwrap();
    assert_eq!(rebuilt[0].runs[0].entered_at, Instant(START.0 + 10_000));
    assert_eq!(
        rebuilt[0].runs[0].exited_at,
        Some(Instant(START.0 + 250_000))
    );
    assert_eq!(
        serde_json::to_value(&rebuilt[0]).unwrap(),
        serde_json::to_value(live.athlete("a1").unwrap()).unwrap()
    );
    let presence: Vec<String> =
        sqlx::query_scalar("SELECT presence FROM raw_events ORDER BY sequence")
            .fetch_all(store.pool())
            .await
            .unwrap();
    assert_eq!(
        presence,
        ["ARRIVED", "DEPARTED"],
        "the raw record keeps what was sent"
    );
}

#[tokio::test]
async fn the_per_reader_guard_is_stored_and_applied() {
    let store = Store::open_in_memory().await.unwrap();
    let mut live = running(&store).await;
    zone_reader(&mut live, &store, Duration(2_000)).await;
    bind(&mut live, &store, 0).await;
    let readers = store.readers().await.unwrap();
    let key = ReaderKey::parse(DEVICE, "zone-1").unwrap();
    assert_eq!(readers.resolve(&key).unwrap().min_visit, Duration(2_000));

    send(&mut live, &store, 1, 10_000, Presence::Arrived).await;
    send(&mut live, &store, 2, 16_000, Presence::Departed).await;

    assert_eq!(
        live.athlete("a1").unwrap().runs[0].exited_at,
        Some(Instant(START.0 + 16_000)),
        "6 s is a visit under a 2 s guard"
    );
}

#[tokio::test]
async fn reads_taken_before_check_in_are_claimed_with_their_presence() {
    let store = Store::open_in_memory().await.unwrap();
    let mut live = running(&store).await;
    zone_reader(&mut live, &store, Duration(15_000)).await;

    // The band was read before anyone bound it to the athlete.
    send(&mut live, &store, 1, 10_000, Presence::Arrived).await;
    send(&mut live, &store, 2, 250_000, Presence::Departed).await;
    bind(&mut live, &store, 300_000).await;

    let a = live.athlete("a1").unwrap();
    assert_eq!(a.runs.len(), 1);
    assert_eq!(
        a.runs[0].exited_at,
        Some(Instant(START.0 + 250_000)),
        "a departure claimed late must not become a second arrival"
    );
}
