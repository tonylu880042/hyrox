//! The emulated collector, speaking over a real broker (CLAUDE.md 25).
//!
//! [`crate::Bench`] wires devices to a hub through an in-process [`crate::Link`], which is
//! what keeps every reliability scenario deterministic and broker-free (CLAUDE.md 24). This
//! module is the same device driven over Mosquitto instead: the journal, the presence
//! suppression and the ACK handling are the *same code* — only the pipe changes.
//!
//! It carries no business meaning, exactly like the device it wraps (CLAUDE.md 8): it
//! publishes what the journal is owed an ACK for, and releases what the hub acknowledges.
//!
//! Behind the `broker` feature, so `crates/simulator` still compiles and tests with no
//! rumqttc in the build.

use crate::{AckResult, SimDevice};
use contract::{AckPayload, EventId};
use rumqttc::{AsyncClient, ClientError, EventLoop};
use std::collections::BTreeSet;
use transport::{client, MqttConfig};

/// A [`SimDevice`] bound to a broker connection.
///
/// The caller owns the [`EventLoop`] and must keep polling it — that is what delivers the
/// acknowledgements and what reconnects the link.
pub struct MqttDevice {
    device: SimDevice,
    client: AsyncClient,
    /// Idempotency keys already put on the wire during this connection. Firmware publishes
    /// a new read once and resends the backlog on reconnect; it does not republish the
    /// whole journal every tick. Cleared by [`MqttDevice::on_reconnect`], which is what
    /// makes a reconnect resend everything still unacknowledged (CLAUDE.md 18).
    published: BTreeSet<EventId>,
}

impl MqttDevice {
    /// Opens the connection for `device`. Nothing is published and no ACK can arrive until
    /// the returned event loop is polled.
    pub fn attach(device: SimDevice, config: &MqttConfig) -> (Self, EventLoop) {
        let (client, eventloop) = client::connect(config);
        (Self { device, client, published: BTreeSet::new() }, eventloop)
    }

    pub fn device(&self) -> &SimDevice {
        &self.device
    }

    pub fn device_mut(&mut self) -> &mut SimDevice {
        &mut self.device
    }

    pub fn client(&self) -> &AsyncClient {
        &self.client
    }

    /// Listens for the hub's acknowledgements on this device's own ack topic.
    pub async fn subscribe_acks(&self) -> Result<(), ClientError> {
        client::subscribe_acks(&self.client, self.device.device_id()).await
    }

    /// Publishes everything the journal still owes an ACK for and has not already sent on
    /// this connection. Returns how many went out.
    ///
    /// Publishing releases nothing: only an ACK does (CLAUDE.md 15, 18).
    pub async fn publish_new(&mut self) -> Result<usize, ClientError> {
        let mut sent = 0;
        for event in self.device.publish_batch() {
            let key = event.id();
            if self.published.contains(&key) {
                continue;
            }
            client::publish_event(&self.client, &event).await?;
            self.published.insert(key);
            sent += 1;
        }
        Ok(sent)
    }

    /// Forgets what was published, so the next [`MqttDevice::publish_new`] resends the whole
    /// unacknowledged backlog. What firmware does after a reconnect, and the reason a hub
    /// outage costs redeliveries rather than events.
    pub fn on_reconnect(&mut self) {
        self.published.clear();
    }

    /// Resends everything still owed an ACK, now.
    ///
    /// Firmware needs this on a timer as well as on reconnect: an ACK can be lost without
    /// the link ever going down, and a lost ACK must cost a redelivery rather than an event
    /// (CLAUDE.md 18). The hub deduplicates on `device_id + boot_id + sequence`, so a
    /// redelivery is free (CLAUDE.md 16).
    pub async fn resend_pending(&mut self) -> Result<usize, ClientError> {
        self.on_reconnect();
        self.publish_new().await
    }

    /// Applies an acknowledgement that arrived on this device's ack topic.
    ///
    /// Both statuses release the entry: `Duplicate` means the hub already holds the event
    /// durably, which is exactly as good as having just stored it (CLAUDE.md 16).
    pub fn on_ack(&mut self, ack: &AckPayload) -> AckResult {
        let result = self.device.on_ack(ack);
        if result == AckResult::Released {
            // Released entries are no longer pending, so they will not be republished; drop
            // the bookkeeping with them rather than growing it for the life of the class.
            self.published.remove(&EventId::from(ack));
        }
        result
    }

    /// Publishes the device's health on the status topic, retained (CLAUDE.md 18).
    pub async fn publish_status(&self) -> Result<(), ClientError> {
        client::publish_status(&self.client, &self.device.status()).await
    }
}
