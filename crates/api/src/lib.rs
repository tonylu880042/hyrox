//! The hub's local web service: REST and WebSocket (CLAUDE.md 22; ADR 0007).
//!
//! Coaches, staff and the big screen reach the hub over the venue LAN from whatever browser
//! they have. This crate owns the router, the handlers and the wire shapes, and owns no
//! rules: every handler parses a request, calls one use case, and turns its answer into a
//! status code (CLAUDE.md 29).
//!
//! ## Dependencies point inward
//!
//! `api` sees `application` and `domain`. It does not see `storage` or `transport`: the
//! store arrives as a generic parameter bounded by `application::HubStore`, exactly as it
//! does in the use cases, so nothing here can reach SQLite or MQTT and the whole surface is
//! exercised against an in-memory fake (CLAUDE.md 3, 24; ADR 0003, 0005).
//!
//! ## The read/write split is a type, not a convention
//!
//! ADR 0001 cuts the screens into one write surface, one narrow write surface, and three
//! read-only ones. Here that cut is three state types, one router each:
//!
//! ```text
//! ReadOnly<S>   /ws  /api/live  /api/coach  /api/session  /api/result/{id}
//! CheckIn<S>    /api/checkin/**          -- bind and rebind, nothing else
//! Operator<S>   /api/operator/**         -- session control, config, readers, inbox
//! ```
//!
//! A handler names the capability it needs in its `State<..>` extractor. [`state::ReadOnly`]
//! holds the store and the live session in private fields and exposes no accessor for
//! either, so a read handler has no expression that reaches a write -- attempting one is a
//! compile error rather than a review finding. [`crate::read`] additionally imports only
//! `get`, and the read paths are disjoint from the write paths, so a `POST` to a read-only
//! surface is a 405 from axum's own method router.
//!
//! ## Two rules every write obeys
//!
//! * **Identity or nothing** (D1). Every mutating route takes an
//!   [`identity::OperatorDevice`] from the `x-operator-device` header and refuses the
//!   request without one. There is no login, so that name is the whole audit identity of
//!   CLAUDE.md 20; defaulting it to an empty string would write a row that looks like a
//!   record of who did something and is not one.
//! * **Refusals are answers** ([`error`]). A domain invariant saying no comes back as 404,
//!   409 or 422 with a stable machine code. Only a failed store write is a 500.
//!
//! ## Every read carries its freshness (D5)
//!
//! Every read response embeds a [`wire::Freshness`]: the hub's clock, the age of the newest
//! event, the socket path, the push interval, and how many sockets are open. D5 calls this
//! mandatory because CLAUDE.md 31's first principle is that no event is lost -- and without
//! it, a still screen and a dead link are the same picture.

pub mod checkin;
pub mod error;
pub mod identity;
pub mod operator;
pub mod read;
pub mod state;
pub mod wire;

pub use error::{ApiError, ErrorBody};
pub use identity::{OperatorDevice, OPERATOR_HEADER};
pub use read::WEBSOCKET_PATH;
pub use state::{CheckIn, Clock, Hub, Operator, ReadOnly};

use application::HubStore;
use axum::Router;
use std::fmt::Display;

/// Prefix for the narrow write surface (ADR 0001, `/checkin`).
pub const CHECKIN_PREFIX: &str = "/api/checkin";
/// Prefix for the write surface (ADR 0001, `/operator`).
pub const OPERATOR_PREFIX: &str = "/api/operator";

/// The whole HTTP surface, assembled from the three capability routers.
///
/// This function is the map ADR 0001 asks for: which surfaces exist, which of them can
/// write, and where each one lives. Nothing mutating is reachable except under
/// [`CHECKIN_PREFIX`] and [`OPERATOR_PREFIX`], because those are the only two routers built
/// with a state type that has a write method on it at all.
///
/// Returns a `Router` with no state left to fill in, so the composition root can merge its
/// own static routes onto it without inheriting any of this crate's plumbing.
pub fn router<S>(hub: Hub<S>) -> Router
where
    S: HubStore + Send + Sync + 'static,
    S::Error: Display + Send,
{
    Router::new()
        .merge(read::router(ReadOnly::new(hub.clone())))
        .nest(CHECKIN_PREFIX, checkin::router(CheckIn::new(hub.clone())))
        .nest(OPERATOR_PREFIX, operator::router(Operator::new(hub)))
}
