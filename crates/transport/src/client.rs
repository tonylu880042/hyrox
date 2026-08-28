//! The broker shell (CLAUDE.md 5, 15).
//!
//! Deliberately thin, and behind the `broker` feature: everything that decides anything —
//! the contract, the idempotency key, the ACK rule — lives in the sibling modules and is
//! testable with no broker in the build at all (CLAUDE.md 24). This module only moves
//! bytes. Nothing here may grow a business rule (CLAUDE.md 29).

use crate::{topic, DeviceStatus};
use contract::{Ack, EdgeEvent};
use domain::DeviceId;
use rumqttc::{AsyncClient, ClientError, EventLoop, MqttOptions};

/// QoS 1 for everything that matters: at-least-once delivery, with the application ACK and
/// hub-side deduplication making the "at least" safe (CLAUDE.md 15, 16).
pub const QOS: rumqttc::QoS = rumqttc::QoS::AtLeastOnce;

/// Mosquitto's default port (CLAUDE.md 5).
pub const DEFAULT_PORT: u16 = 1883;
/// Short enough that a dead link is noticed within a station transition, long enough not to
/// thrash a venue Wi-Fi. Validate at the venue.
pub const DEFAULT_KEEP_ALIVE_SECS: u64 = 15;
/// In-flight request queue depth for the rumqttc event loop.
pub const DEFAULT_CAPACITY: usize = 1024;

#[derive(Clone, Debug)]
pub struct MqttConfig {
    pub client_id: String,
    pub host: String,
    pub port: u16,
    pub keep_alive_secs: u64,
    /// `false` keeps the broker's QoS 1 session across a reconnect, so in-flight messages
    /// are not silently dropped by the reconnect itself.
    pub clean_session: bool,
    pub capacity: usize,
}

impl MqttConfig {
    /// A broker on the same machine — the Phase 1 layout, where the hub, the broker and
    /// SQLite all sit on one box on the venue LAN (CLAUDE.md 5).
    pub fn local(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            host: "127.0.0.1".to_string(),
            port: DEFAULT_PORT,
            keep_alive_secs: DEFAULT_KEEP_ALIVE_SECS,
            clean_session: false,
            capacity: DEFAULT_CAPACITY,
        }
    }

    pub fn options(&self) -> MqttOptions {
        let mut opts = MqttOptions::new(&self.client_id, &self.host, self.port);
        opts.set_keep_alive(std::time::Duration::from_secs(self.keep_alive_secs));
        opts.set_clean_session(self.clean_session);
        opts
    }
}

/// Opens the connection. The caller owns the event loop and must keep polling it — that is
/// what drives reconnection, so the hub keeps working across a broker restart
/// (CLAUDE.md 31).
pub fn connect(config: &MqttConfig) -> (AsyncClient, EventLoop) {
    AsyncClient::new(config.options(), config.capacity)
}

/// Subscribes the hub to every device's events and health.
pub async fn subscribe_hub(client: &AsyncClient) -> Result<(), ClientError> {
    client.subscribe(topic::ALL_EVENTS, QOS).await?;
    client.subscribe(topic::ALL_STATUS, QOS).await
}

/// Publishes an acknowledgement.
///
/// Takes an [`Ack`], which can only be produced from a committed event — so this function
/// physically cannot be used to ACK something that is not yet durable (CLAUDE.md 15).
pub async fn publish_ack(
    client: &AsyncClient,
    device: &DeviceId,
    ack: &Ack,
) -> Result<(), ClientError> {
    client
        .publish(topic::ack(device), QOS, false, ack.encode())
        .await
}

/// Edge-side publish, used by the simulator and by firmware-equivalent code paths.
pub async fn publish_event(client: &AsyncClient, event: &EdgeEvent) -> Result<(), ClientError> {
    client
        .publish(topic::events(&event.device_id), QOS, false, event.encode())
        .await
}

/// Device health. Retained, so a newly started hub immediately sees a device that was
/// already warning about its journal before the hub came up (CLAUDE.md 18, 21).
pub async fn publish_status(
    client: &AsyncClient,
    status: &DeviceStatus,
) -> Result<(), ClientError> {
    client
        .publish(topic::status(&status.device_id), QOS, true, status.encode())
        .await
}
