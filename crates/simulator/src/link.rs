//! A deterministic model of the MQTT link (CLAUDE.md 25).
//!
//! Real brokers duplicate, reorder and drop under load, but they do it when they feel like
//! it. Here each failure mode is a switch, so "what happens when the ACK is lost" is a
//! test rather than a stakeout — and the same scenario replays identically every time
//! (CLAUDE.md 21, 29). No randomness on purpose.
//!
//! This models the *transport*, not the broker: QoS 1 already permits every one of these.

use mqtt::{AckPayload, EdgeEvent};

/// Whether the link delivers each message once or twice. QoS 1 is at-*least*-once, so a
/// duplicate is correct behaviour, not a fault the transport must prevent (CLAUDE.md 15).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Duplication {
    #[default]
    None,
    EveryMessage,
}

/// Whether a batch keeps its published order. Arrival order carries no meaning: timing
/// comes from `detected_at` (CLAUDE.md 17).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ordering {
    #[default]
    AsPublished,
    Reversed,
}

/// Whether acknowledgements make it back. A lost ACK must cost a redelivery and nothing
/// more (CLAUDE.md 18).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AckDelivery {
    #[default]
    Delivered,
    Lost,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkFaults {
    pub duplication: Duplication,
    pub ordering: Ordering,
    pub ack_delivery: AckDelivery,
}

impl LinkFaults {
    /// A well-behaved link.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with_duplication(mut self, duplication: Duplication) -> Self {
        self.duplication = duplication;
        self
    }

    pub fn with_ordering(mut self, ordering: Ordering) -> Self {
        self.ordering = ordering;
        self
    }

    pub fn with_ack_delivery(mut self, ack_delivery: AckDelivery) -> Self {
        self.ack_delivery = ack_delivery;
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Link {
    faults: LinkFaults,
}

impl Link {
    pub fn new(faults: LinkFaults) -> Self {
        Self { faults }
    }

    pub fn faults(&self) -> LinkFaults {
        self.faults
    }

    pub fn set_duplication(&mut self, duplication: Duplication) {
        self.faults.duplication = duplication;
    }

    pub fn set_ordering(&mut self, ordering: Ordering) {
        self.faults.ordering = ordering;
    }

    /// Lets a test restore the ACK path mid-scenario, which is what a recovering network
    /// actually does.
    pub fn set_ack_delivery(&mut self, ack_delivery: AckDelivery) {
        self.faults.ack_delivery = ack_delivery;
    }

    /// Carries one device's published batch to the hub, applying the configured faults.
    pub fn deliver(&self, batch: Vec<EdgeEvent>) -> Vec<EdgeEvent> {
        let mut out = batch;
        if self.faults.ordering == Ordering::Reversed {
            out.reverse();
        }
        if self.faults.duplication == Duplication::EveryMessage {
            out = out.into_iter().flat_map(|e| [e.clone(), e]).collect();
        }
        out
    }

    /// Carries one acknowledgement back. `None` is a lost ACK — the event stays pending.
    pub fn deliver_ack(&self, ack: AckPayload) -> Option<AckPayload> {
        match self.faults.ack_delivery {
            AckDelivery::Delivered => Some(ack),
            AckDelivery::Lost => None,
        }
    }
}
