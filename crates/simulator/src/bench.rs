//! A fleet of devices, a faulty link and a hub, wired into one loop.
//!
//! One `flush` is one delivery round: every online device publishes its backlog, the link
//! mangles it, the hub commits and acknowledges, and the acknowledgements come back the
//! same way. Because nothing here is random or wall-clock driven, a scenario is a
//! reproducible statement about the system (CLAUDE.md 21, 24).

use crate::{InMemoryHub, Link, SimDevice};
use mqtt::{ingest, ReceivedEvent};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlushReport {
    /// Messages that reached the hub, duplicates included.
    pub delivered: usize,
    /// Acknowledgements that made it back to a device.
    pub acked: usize,
    /// Acknowledgements the link swallowed. The events stay pending.
    pub lost_acks: usize,
    /// Deliveries the hub refused — malformed, or storage down. No ACK was produced.
    pub rejected: usize,
}

pub struct Bench {
    devices: Vec<SimDevice>,
    link: Link,
    hub: InMemoryHub,
}

impl Bench {
    pub fn new(devices: Vec<SimDevice>, link: Link, hub: InMemoryHub) -> Self {
        Self { devices, link, hub }
    }

    pub fn devices(&self) -> &[SimDevice] {
        &self.devices
    }

    pub fn devices_mut(&mut self) -> &mut [SimDevice] {
        &mut self.devices
    }

    pub fn device_mut(&mut self, index: usize) -> &mut SimDevice {
        &mut self.devices[index]
    }

    pub fn hub(&self) -> &InMemoryHub {
        &self.hub
    }

    pub fn link_mut(&mut self) -> &mut Link {
        &mut self.link
    }

    /// Events across the whole fleet still owed an acknowledgement.
    pub fn pending_total(&self) -> usize {
        self.devices.iter().map(SimDevice::pending_count).sum()
    }

    /// One delivery round at hub-clock time `received_at`.
    ///
    /// `received_at` is stamped on arrival and is diagnostics only; every event keeps the
    /// `detected_at` its device gave it (CLAUDE.md 17).
    pub async fn flush(&mut self, received_at: i64) -> FlushReport {
        let mut report = FlushReport::default();

        for device in self.devices.iter_mut() {
            let batch = self.link.deliver(device.publish_batch());
            report.delivered += batch.len();

            // Acks are collected first, then routed back: the hub answers deliveries, it
            // does not reach into the device.
            let mut acks = Vec::new();
            for event in batch {
                match ingest(&self.hub, &ReceivedEvent::new(event, received_at)).await {
                    Ok(ack) => acks.push(ack.into_payload()),
                    Err(_) => report.rejected += 1,
                }
            }
            for ack in acks {
                match self.link.deliver_ack(ack) {
                    Some(delivered) => {
                        device.on_ack(&delivered);
                        report.acked += 1;
                    }
                    None => report.lost_acks += 1,
                }
            }
        }
        report
    }
}
