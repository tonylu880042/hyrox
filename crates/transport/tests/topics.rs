//! Topic scheme (CLAUDE.md 5, 8, 15, 17).

use domain::DeviceId;
use transport::topic;

fn device() -> DeviceId {
    DeviceId::from_mac_str("a4cf128b3d91").unwrap()
}

#[test]
fn events_flow_up_and_acks_flow_down_on_separate_branches() {
    let d = device();
    assert_eq!(topic::events(&d), "hyrox/v1/edge/esp32-a4cf128b3d91/events");
    assert_eq!(topic::ack(&d), "hyrox/v1/hub/esp32-a4cf128b3d91/ack");
    assert_eq!(topic::status(&d), "hyrox/v1/edge/esp32-a4cf128b3d91/status");
    assert_eq!(topic::TIME_SYNC, "hyrox/v1/hub/time");
}

#[test]
fn the_hub_subscribes_to_every_device_at_once() {
    assert_eq!(topic::ALL_EVENTS, "hyrox/v1/edge/+/events");
    assert_eq!(topic::ALL_STATUS, "hyrox/v1/edge/+/status");
}

#[test]
fn an_arriving_event_topic_names_its_device() {
    let d = device();
    assert_eq!(topic::device_of_events(&topic::events(&d)), Some(d));
}

/// The downlink direction: only an edge collector subscribes to these, and it must be able
/// to tell its own acknowledgements from anything else on the branch.
#[test]
fn an_arriving_ack_topic_names_the_device_it_is_addressed_to() {
    let d = device();
    assert_eq!(topic::device_of_ack(&topic::ack(&d)), Some(d.clone()));
    for t in [&topic::events(&d), &topic::status(&d), "hyrox/v1/hub/time"] {
        assert_eq!(topic::device_of_ack(t), None, "{t} must not parse as an ack");
    }
}

#[test]
fn a_foreign_or_malformed_topic_names_no_device() {
    for t in [
        "hyrox/v1/edge/esp32-a4cf128b3d91/status",
        "hyrox/v1/hub/esp32-a4cf128b3d91/ack",
        "hyrox/v2/edge/esp32-a4cf128b3d91/events",
        "hyrox/v1/edge/not-a-device/events",
        "",
    ] {
        assert_eq!(topic::device_of_events(t), None, "{t} must not parse");
    }
}
