//! The broker client against a real broker (CLAUDE.md 5, 15).
//!
//! Every other test in this workspace runs with no broker, no database and no hardware
//! (CLAUDE.md 24), and that must stay true: these are the only tests that need Mosquitto.
//! So they **skip themselves** when nothing answers on the configured address, rather than
//! failing. `cargo test --workspace` stays green on a machine with no broker; it simply
//! proves less.
//!
//! ```text
//! brew services start mosquitto     # or: mosquitto -p 1883
//! cargo test -p transport --test broker -- --nocapture
//! ```
//!
//! `--nocapture` is worth using: a skipped test says so on stdout, and a silently skipped
//! test is indistinguishable from a passing one.
//!
//! Override the address with `HYROX_TEST_MQTT_HOST` / `HYROX_TEST_MQTT_PORT`.
//!
//! What is covered here is the transport shell only: connect, subscribe, publish, receive,
//! in both directions. What an event *means* is tested without a broker, where it belongs.

// With the `broker` feature off there is no client to test, and rumqttc is not even in the
// build. The topic scheme and the classifier still are, and are still tested.
#![cfg(feature = "broker")]

use contract::{AckPayload, AckStatus, DeviceId, EdgeEvent, ReaderId};
use std::time::Duration;
use transport::{
    client::{self, EventLoop},
    topic, DeviceStatus, Inbound, MqttConfig,
};

/// Long enough for a loopback broker to answer several times over; short enough that a
/// hung test fails the suite in seconds rather than minutes.
const TIMEOUT: Duration = Duration::from_secs(5);

/// These tests share one broker and one topic tree, and the hub subscription is a wildcard,
/// so a concurrent test's traffic would arrive in the middle of this one's. They run one at
/// a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn broker_address() -> (String, u16) {
    let host = std::env::var("HYROX_TEST_MQTT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("HYROX_TEST_MQTT_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(client::DEFAULT_PORT);
    (host, port)
}

/// The gate: `None` when nothing is listening, so the test can report a skip instead of a
/// failure. A plain TCP connect, deliberately -- it asks the one question that matters
/// (is there a broker?) without needing an MQTT session to ask it.
fn broker(client_id: &str) -> Option<MqttConfig> {
    use std::net::{TcpStream, ToSocketAddrs};
    let (host, port) = broker_address();
    let addr = format!("{host}:{port}").to_socket_addrs().ok()?.next()?;
    if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err() {
        println!("SKIPPED: no MQTT broker on {host}:{port} (start Mosquitto to run this test)");
        return None;
    }
    // A per-test client id, and a clean session: these tests must not inherit a queue left
    // by an earlier run, and must not leave one behind. The hub itself does the opposite,
    // and for a good reason -- see `DEFAULT_CLIENT_ID` in `apps/hub-server`.
    let mut config = MqttConfig::local(format!("hyrox-test-{}-{}", client_id, std::process::id()));
    config.host = host;
    config.port = port;
    config.clean_session = true;
    Some(config)
}

/// Skips the test body when no broker answers, and serialises the ones that do run.
macro_rules! require_broker {
    ($client_id:expr) => {{
        let config = match broker($client_id) {
            Some(config) => config,
            None => return,
        };
        (config, SERIAL.lock().await)
    }};
}

/// A MAC no dev venue uses. The hub subscribes with a wildcard, so anything else publishing
/// to this broker -- a `hub-server` left running, a retained status from an earlier run --
/// lands in these tests' subscriptions too. They filter on this device rather than assuming
/// they have the broker to themselves.
fn device() -> DeviceId {
    DeviceId::from_mac_str("de:ad:be:ef:00:01").expect("canonical MAC")
}

/// Whether a message concerns the device under test. Everything else is somebody else's
/// traffic and is skipped, never asserted on.
fn is_ours(inbound: &Inbound) -> bool {
    let ours = device();
    match inbound {
        Inbound::Event(event) => event.device_id == ours,
        Inbound::Status(status) => status.device_id == ours,
        Inbound::Ack(ack) => ack.device_id == ours,
        Inbound::Undecodable { topic: name, .. } => {
            topic::device_of_events(name).as_ref() == Some(&ours)
                || topic::device_of_status(name).as_ref() == Some(&ours)
        }
        Inbound::Connected { .. } | Inbound::Foreign { .. } => true,
    }
}

fn event(sequence: i64) -> EdgeEvent {
    EdgeEvent {
        device_id: device(),
        reader_id: ReaderId::parse("rfid-skierg-entry").expect("canonical reader id"),
        boot_id: 7,
        sequence,
        tag_id: "E280117000001234".to_string(),
        detected_at: 1_787_734_821_382,
        uptime_ms: 382_912,
    }
}

/// Polls until something classified arrives, or the timeout expires.
async fn next(eventloop: &mut EventLoop) -> Option<Inbound> {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            match client::next_inbound(eventloop).await {
                Ok(Some(inbound)) => return inbound,
                Ok(None) => continue,
                Err(e) => panic!("broker connection failed: {e}"),
            }
        }
    })
    .await
    .ok()
}

/// Polls past the connection handshake and past other publishers' traffic, and returns the
/// first message about the device under test.
async fn next_message(eventloop: &mut EventLoop) -> Option<Inbound> {
    loop {
        match next(eventloop).await? {
            Inbound::Connected { .. } => continue,
            other if !is_ours(&other) => continue,
            other => return Some(other),
        }
    }
}

/// Waits for the CONNACK, so a subscription made afterwards is really in place before the
/// test publishes anything into it.
async fn wait_connected(eventloop: &mut EventLoop) {
    match next(eventloop).await {
        Some(Inbound::Connected { .. }) => {}
        other => panic!("expected a connection, got {other:?}"),
    }
}

/// Keeps polling for a moment so queued requests -- a SUBSCRIBE, a PUBLISH -- actually reach
/// the broker before the test moves on. `AsyncClient` only enqueues; the event loop is what
/// sends, which is the same reason the hub must never stop polling it.
///
/// Nothing about the device under test is expected to arrive during a settle; anything that
/// does means the test's own sequencing is wrong, so it fails loudly rather than being
/// swallowed. Other publishers' traffic is discarded.
async fn settle(eventloop: &mut EventLoop) {
    let _ = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            match client::next_inbound(eventloop).await {
                Ok(Some(Inbound::Connected { .. })) | Ok(None) => continue,
                Ok(Some(unexpected)) if is_ours(&unexpected) => {
                    panic!("unexpected message while settling: {unexpected:?}")
                }
                Ok(Some(_)) => continue,
                Err(e) => panic!("broker connection failed: {e}"),
            }
        }
    })
    .await;
}

#[tokio::test]
async fn broker_connects_and_accepts_the_hub_subscription() {
    let (config, _serial) = require_broker!("connect");
    let (client, mut eventloop) = client::connect(&config);
    wait_connected(&mut eventloop).await;
    client::subscribe_hub(&client).await.expect("subscribe");
    // The SUBACK is not surfaced as an `Inbound`; that the loop keeps turning without an
    // error, and that no message arrives on a wildcard nobody is publishing to, is what
    // says the subscription was accepted.
    settle(&mut eventloop).await;
}

#[tokio::test]
async fn broker_carries_an_event_from_a_device_to_the_hub_subscription() {
    let (config, _serial) = require_broker!("event-up");
    let (hub, mut hub_loop) = client::connect(&config);
    wait_connected(&mut hub_loop).await;
    client::subscribe_hub(&hub).await.expect("subscribe");
    settle(&mut hub_loop).await;

    let (edge, mut edge_loop) = client::connect(&MqttConfig {
        client_id: format!("{}-edge", config.client_id),
        ..config.clone()
    });
    wait_connected(&mut edge_loop).await;
    let sent = event(10_382);
    client::publish_event(&edge, &sent).await.expect("publish");
    tokio::spawn(async move { while client::next_inbound(&mut edge_loop).await.is_ok() {} });

    match next_message(&mut hub_loop).await {
        Some(Inbound::Event(received)) => {
            assert_eq!(*received, sent);
            // Official timing survives the wire untouched (CLAUDE.md 17).
            assert_eq!(received.detected_at, sent.detected_at);
        }
        other => panic!("expected the event back, got {other:?}"),
    }
}

#[tokio::test]
async fn broker_carries_an_ack_from_the_hub_to_the_device_that_earned_it() {
    let (config, _serial) = require_broker!("ack-down");
    let device = device();

    let (edge, mut edge_loop) = client::connect(&config);
    wait_connected(&mut edge_loop).await;
    client::subscribe_acks(&edge, &device).await.expect("subscribe acks");
    settle(&mut edge_loop).await;

    let (hub, mut hub_loop) = client::connect(&MqttConfig {
        client_id: format!("{}-hub", config.client_id),
        ..config.clone()
    });
    wait_connected(&mut hub_loop).await;
    // The only way to obtain an `Ack` is a committed event, so this test has to commit one
    // first (ADR 0002). That is the point: there is no shortcut, not even for a test.
    let store = CommittingStore;
    let ack = contract::ingest(&store, &contract::ReceivedEvent::new(event(10_382), 1))
        .await
        .expect("commit");
    client::publish_ack(&hub, &device, &ack).await.expect("publish ack");
    tokio::spawn(async move { while client::next_inbound(&mut hub_loop).await.is_ok() {} });

    match next_message(&mut edge_loop).await {
        Some(Inbound::Ack(payload)) => {
            assert_eq!(
                *payload,
                AckPayload {
                    device_id: device,
                    boot_id: 7,
                    sequence: 10_382,
                    status: AckStatus::Stored,
                }
            );
        }
        other => panic!("expected the ack back, got {other:?}"),
    }
}

#[tokio::test]
async fn broker_retains_device_health_for_a_hub_that_connects_later() {
    let (config, _serial) = require_broker!("status-retained");
    let status = DeviceStatus {
        device_id: device(),
        boot_id: 7,
        pending_events: 8_123,
        journal_capacity: 10_000,
        warning: Some(transport::DeviceWarning::JournalNearlyFull),
    };

    let (edge, mut edge_loop) = client::connect(&config);
    wait_connected(&mut edge_loop).await;
    client::publish_status(&edge, &status).await.expect("publish status");
    settle(&mut edge_loop).await;

    // A hub that starts *after* the warning must still see it (CLAUDE.md 18, 21).
    let (hub, mut hub_loop) = client::connect(&MqttConfig {
        client_id: format!("{}-hub", config.client_id),
        ..config.clone()
    });
    wait_connected(&mut hub_loop).await;
    client::subscribe_hub(&hub).await.expect("subscribe");
    match next_message(&mut hub_loop).await {
        Some(Inbound::Status(received)) => assert_eq!(*received, status),
        other => panic!("expected the retained status, got {other:?}"),
    }

    // Clear the retained message, or it outlives the test run and greets the next one.
    edge.publish(topic::status(&status.device_id), client::QOS, true, Vec::new())
        .await
        .expect("clear retained status");
    settle(&mut edge_loop).await;
}

#[tokio::test]
async fn broker_a_payload_that_is_not_an_event_does_not_stop_the_subscriber() {
    let (config, _serial) = require_broker!("undecodable");
    let (hub, mut hub_loop) = client::connect(&config);
    wait_connected(&mut hub_loop).await;
    client::subscribe_hub(&hub).await.expect("subscribe");
    settle(&mut hub_loop).await;

    let (edge, mut edge_loop) = client::connect(&MqttConfig {
        client_id: format!("{}-edge", config.client_id),
        ..config.clone()
    });
    wait_connected(&mut edge_loop).await;
    let events = topic::events(&device());
    edge.publish(events.clone(), client::QOS, false, b"{not json".to_vec())
        .await
        .expect("publish rubbish");
    let good = event(10_383);
    client::publish_event(&edge, &good).await.expect("publish");
    tokio::spawn(async move { while client::next_inbound(&mut edge_loop).await.is_ok() {} });

    match next_message(&mut hub_loop).await {
        Some(Inbound::Undecodable { topic, payload, .. }) => {
            assert_eq!(topic, events);
            assert_eq!(payload, b"{not json");
        }
        other => panic!("expected an undecodable payload, got {other:?}"),
    }
    // The broken device must not have taken the class with it (CLAUDE.md 31).
    match next_message(&mut hub_loop).await {
        Some(Inbound::Event(received)) => assert_eq!(*received, good),
        other => panic!("expected the good event after the bad one, got {other:?}"),
    }
}

/// The smallest store that can honestly mint a `Commit`: it claims durability, which is all
/// this file needs in order to have an `Ack` to publish. The real one is `crates/storage`.
struct CommittingStore;

impl contract::EventStore for CommittingStore {
    type Error = std::convert::Infallible;

    async fn commit(
        &self,
        _event: &contract::ReceivedEvent,
    ) -> Result<contract::CommitOutcome, Self::Error> {
        Ok(contract::CommitOutcome::Stored)
    }
}
