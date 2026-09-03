//! End-to-end edge → link → hub behaviour, with no broker and no hardware (CLAUDE.md 24, 25).
//!
//! The link is a deterministic fault model rather than a real network: duplication,
//! reordering and ACK loss are switches, so a failure mode is a test and not a stakeout.

use contract::{EventId, ReaderId};
use simulator::{
    AbsentTimeout, AckDelivery, Bench, DeviceConfig, Duplication, InMemoryHub, Link, LinkFaults,
    Ordering, ReaderConfig, SimDevice,
};

const TAG_A: &str = "E280117000001234";
const TAG_B: &str = "E280117000005678";
const T0: i64 = 1_787_734_800_000;

fn rid(id: &str) -> ReaderId {
    ReaderId::parse(id).unwrap()
}

fn device(mac: &str, readers: &[&str]) -> SimDevice {
    let mut config = DeviceConfig::new(mac).unwrap();
    for r in readers {
        config = config
            .with_reader(ReaderConfig::new(r, AbsentTimeout::from_millis(4_000).unwrap()).unwrap());
    }
    SimDevice::boot(config, T0).unwrap()
}

fn bench(devices: Vec<SimDevice>, faults: LinkFaults) -> Bench {
    Bench::new(devices, Link::new(faults), InMemoryHub::new())
}

// --- multiple devices, readers and tags ----------------------------------------------

#[tokio::test]
async fn a_fleet_of_devices_reports_independently() {
    let mut b = bench(
        vec![
            device("a4:cf:12:8b:3d:91", &["rfid-01"]),
            device("a4:cf:12:8b:3d:92", &["rfid-01"]),
            device("a4:cf:12:8b:3d:93", &["rfid-01", "rfid-02"]),
        ],
        LinkFaults::none(),
    );

    for (i, d) in b.devices_mut().iter_mut().enumerate() {
        d.rf_read(&rid("rfid-01"), TAG_A, T0 + i as i64).unwrap();
    }
    b.device_mut(2)
        .rf_read(&rid("rfid-02"), TAG_B, T0 + 50)
        .unwrap();

    let report = b.flush(T0 + 200).await;
    assert_eq!(report.delivered, 4);
    assert_eq!(b.hub().committed_count(), 4);
    assert_eq!(b.pending_total(), 0, "everyone was acknowledged");

    // Same boot_id and sequence on two devices must stay two events (CLAUDE.md 16).
    let keys = b.hub().arrival_order();
    assert_eq!(
        keys.iter().collect::<std::collections::HashSet<_>>().len(),
        4
    );
}

#[test]
fn macs_are_configurable_per_device() {
    let d = device("A4-CF-12-8B-3D-91", &["rfid-01"]);
    assert_eq!(d.device_id().as_str(), "a4cf128b3d91");
    assert_eq!(d.device_id().as_str(), "a4cf128b3d91");
}

// --- repeated reads and re-arm, through the whole path --------------------------------

#[tokio::test]
async fn repeated_reads_never_reach_the_hub_but_a_rearm_does() {
    let mut b = bench(
        vec![device("a4cf128b3d91", &["rfid-01"])],
        LinkFaults::none(),
    );
    let d = b.device_mut(0);
    for t in (0..3_000).step_by(200) {
        d.rf_read(&rid("rfid-01"), TAG_A, T0 + t).unwrap();
    }
    d.rf_read(&rid("rfid-01"), TAG_A, T0 + 8_000).unwrap(); // after a 5 s absence

    b.flush(T0 + 9_000).await;
    assert_eq!(b.hub().committed_count(), 2, "one entry, one re-entry");
}

// --- disconnect, reconnect, resend ----------------------------------------------------

#[tokio::test]
async fn nothing_is_delivered_while_the_link_is_down() {
    let mut b = bench(
        vec![device("a4cf128b3d91", &["rfid-01"])],
        LinkFaults::none(),
    );
    b.device_mut(0).disconnect();
    for (i, tag) in [TAG_A, TAG_B].iter().enumerate() {
        b.device_mut(0)
            .rf_read(&rid("rfid-01"), tag, T0 + i as i64 * 100)
            .unwrap();
    }

    let report = b.flush(T0 + 1_000).await;
    assert_eq!(report.delivered, 0);
    assert_eq!(b.hub().committed_count(), 0);
    assert_eq!(b.pending_total(), 2, "held on the device, not lost");
}

#[tokio::test]
async fn reconnecting_resends_every_unacknowledged_event() {
    let mut b = bench(
        vec![device("a4cf128b3d91", &["rfid-01"])],
        LinkFaults::none(),
    );
    b.device_mut(0).disconnect();
    for (i, tag) in [TAG_A, TAG_B].iter().enumerate() {
        b.device_mut(0)
            .rf_read(&rid("rfid-01"), tag, T0 + i as i64 * 100)
            .unwrap();
    }
    b.flush(T0 + 1_000).await;

    b.device_mut(0).reconnect();
    let report = b.flush(T0 + 2_000).await;

    assert_eq!(report.delivered, 2);
    assert_eq!(b.hub().committed_count(), 2);
    assert_eq!(b.pending_total(), 0);
}

#[tokio::test]
async fn an_outage_across_a_reboot_still_loses_nothing() {
    // Disconnect, record, power-cycle, reconnect: the classic venue failure (CLAUDE.md 18).
    let mut b = bench(
        vec![device("a4cf128b3d91", &["rfid-01"])],
        LinkFaults::none(),
    );
    b.device_mut(0).disconnect();
    b.device_mut(0).rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    b.device_mut(0).reboot(T0 + 30_000);
    b.device_mut(0)
        .rf_read(&rid("rfid-01"), TAG_B, T0 + 31_000)
        .unwrap();
    b.device_mut(0).reconnect();

    b.flush(T0 + 32_000).await;
    assert_eq!(b.hub().committed_count(), 2);
    assert_eq!(b.pending_total(), 0);
}

// --- duplicate delivery ----------------------------------------------------------------

#[tokio::test]
async fn a_duplicated_delivery_is_committed_once_and_acked_twice() {
    // Duplicate delivery is allowed, duplicate processing is not (CLAUDE.md 16).
    let mut b = bench(
        vec![device("a4cf128b3d91", &["rfid-01"])],
        LinkFaults::none().with_duplication(Duplication::EveryMessage),
    );
    b.device_mut(0).rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();

    let report = b.flush(T0 + 100).await;
    assert_eq!(report.delivered, 2, "the link doubled it");
    assert_eq!(b.hub().commit_calls(), 2);
    assert_eq!(b.hub().committed_count(), 1, "one row");
    assert_eq!(b.pending_total(), 0);
}

#[tokio::test]
async fn a_resend_of_an_already_committed_event_is_harmless() {
    // What actually happens after a lost ACK: the edge sends it again, the hub recognises
    // the key and says DUPLICATE.
    let mut b = bench(
        vec![device("a4cf128b3d91", &["rfid-01"])],
        LinkFaults::none().with_ack_delivery(AckDelivery::Lost),
    );
    b.device_mut(0).rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    b.flush(T0 + 100).await;
    assert_eq!(b.pending_total(), 1, "no ACK, so still owed");

    b.link_mut().set_ack_delivery(AckDelivery::Delivered);
    let report = b.flush(T0 + 200).await;

    assert_eq!(report.delivered, 1, "resent");
    assert_eq!(b.hub().committed_count(), 1, "still one row");
    assert_eq!(b.pending_total(), 0);
}

// --- missing ACK -----------------------------------------------------------------------

#[tokio::test]
async fn a_lost_ack_never_releases_an_event() {
    let mut b = bench(
        vec![device("a4cf128b3d91", &["rfid-01"])],
        LinkFaults::none().with_ack_delivery(AckDelivery::Lost),
    );
    b.device_mut(0).rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();

    for round in 1..=3 {
        let report = b.flush(T0 + round * 100).await;
        assert_eq!(report.lost_acks, 1);
        assert_eq!(b.pending_total(), 1, "round {round}");
    }
    assert_eq!(
        b.hub().committed_count(),
        1,
        "redelivery did not duplicate the row"
    );
}

#[tokio::test]
async fn a_failed_commit_produces_no_ack_and_the_edge_retries() {
    // The rule from CLAUDE.md 15, seen from the edge: no commit, no ACK, no release.
    let mut b = bench(
        vec![device("a4cf128b3d91", &["rfid-01"])],
        LinkFaults::none(),
    );
    b.hub().set_failing(true);
    b.device_mut(0).rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();

    let report = b.flush(T0 + 100).await;
    assert_eq!(report.rejected, 1);
    assert_eq!(report.acked, 0);
    assert_eq!(b.pending_total(), 1);

    b.hub().set_failing(false);
    b.flush(T0 + 200).await;
    assert_eq!(b.hub().committed_count(), 1);
    assert_eq!(b.pending_total(), 0);
}

// --- out-of-order arrival ---------------------------------------------------------------

#[tokio::test]
async fn out_of_order_arrival_does_not_disturb_official_timing() {
    // Arrival order is a property of the network. Timing comes from `detected_at`
    // (CLAUDE.md 17), so the hub can reconstruct the true order regardless.
    let mut b = bench(
        vec![device("a4cf128b3d91", &["rfid-01"])],
        LinkFaults::none().with_ordering(Ordering::Reversed),
    );
    let taps = [(TAG_A, 0), (TAG_B, 500), (TAG_A, 9_000)];
    for (tag, offset) in taps {
        b.device_mut(0)
            .rf_read(&rid("rfid-01"), tag, T0 + offset)
            .unwrap();
    }

    b.flush(T0 + 10_000).await;
    assert_eq!(b.hub().committed_count(), 3);

    let arrived: Vec<i64> = b
        .hub()
        .arrival_order()
        .iter()
        .map(|k| b.hub().official_time(k).unwrap())
        .collect();
    assert_eq!(
        arrived,
        [T0 + 9_000, T0 + 500, T0],
        "arrival really was reversed"
    );

    let mut sorted = arrived.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        [T0, T0 + 500, T0 + 9_000],
        "detected_at recovers the truth"
    );
}

#[tokio::test]
async fn a_late_arrival_never_changes_an_events_timestamp() {
    let mut b = bench(
        vec![device("a4cf128b3d91", &["rfid-01"])],
        LinkFaults::none(),
    );
    b.device_mut(0).disconnect();
    b.device_mut(0).rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    b.device_mut(0).reconnect();

    // Delivered five minutes late after an outage.
    b.flush(T0 + 300_000).await;
    let key = b.hub().arrival_order()[0].clone();
    assert_eq!(b.hub().official_time(&key), Some(T0));
    assert_eq!(
        b.hub().arrival_lag_ms(&key),
        Some(300_000),
        "lag is diagnostics"
    );
}

#[tokio::test]
async fn everything_at_once_still_loses_nothing() {
    // Duplicated, reordered, ACKs dropped, an outage and a reboot in the middle.
    let mut b = bench(
        vec![
            device("a4cf128b3d91", &["rfid-01", "rfid-02"]),
            device("a4cf128b3d92", &["rfid-01"]),
        ],
        LinkFaults::none()
            .with_duplication(Duplication::EveryMessage)
            .with_ordering(Ordering::Reversed)
            .with_ack_delivery(AckDelivery::Lost),
    );

    let mut expected: Vec<EventId> = Vec::new();
    b.device_mut(0).rf_read(&rid("rfid-01"), TAG_A, T0).unwrap();
    b.device_mut(0)
        .rf_read(&rid("rfid-02"), TAG_B, T0 + 100)
        .unwrap();
    b.device_mut(1)
        .rf_read(&rid("rfid-01"), TAG_A, T0 + 200)
        .unwrap();
    b.flush(T0 + 300).await;

    b.device_mut(1).disconnect();
    b.device_mut(1).reboot(T0 + 1_000);
    b.device_mut(1)
        .rf_read(&rid("rfid-01"), TAG_B, T0 + 1_100)
        .unwrap();
    b.device_mut(1).reconnect();
    b.link_mut().set_ack_delivery(AckDelivery::Delivered);
    b.flush(T0 + 2_000).await;

    for d in b.devices_mut() {
        expected.extend(d.pending().iter().map(|e| e.id()));
    }
    assert!(expected.is_empty(), "every event was acknowledged");
    assert_eq!(b.hub().committed_count(), 4, "four reads, four rows");
    assert!(
        b.hub().commit_calls() > 4,
        "and plenty of redelivery on the way"
    );
}
