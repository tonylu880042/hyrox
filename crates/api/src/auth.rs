//! The operator session boundary.
//!
//! PIN verification is the bootstrap operation.  Everything after it uses a random,
//! server-side session token in an HttpOnly cookie; the browser never receives a bearer token
//! in JSON or local storage.  The registry is intentionally process-local: a hub is one
//! appliance, and a restart invalidates the old browser sessions along with the process.

use crate::error::ApiError;
use crate::identity::OperatorDevice;
use crate::state::Operator;
use application::HubStore;
use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The only cookie issued by the operator PIN flow.
pub const SESSION_COOKIE: &str = "hyrox_operator_session";

const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const SESSION_TOKEN_BYTES: usize = 32;
const MAX_SESSIONS: usize = 1024;
const PIN_WINDOW: Duration = Duration::from_secs(60);
const MAX_PIN_ATTEMPTS: usize = 12;

#[derive(Debug)]
struct Session {
    issued_at: Instant,
    expires_at: Instant,
}

/// Authentication state is kept apart from the virtual class clock.  Session expiry and PIN
/// throttling must continue to use the host's monotonic clock when a demo clock is accelerated.
pub(crate) struct AuthState {
    sessions: Mutex<HashMap<String, Session>>,
    pin_attempts: Mutex<VecDeque<Instant>>,
    ttl: Duration,
    machine_credential_digest: Mutex<Option<[u8; 32]>>,
    /// Generation changes under the same gate as issue/revoke.  A PIN verification that
    /// crossed a concurrent PIN change therefore cannot issue a session for the old PIN.
    pin_generation: AtomicU64,
    session_gate: Mutex<()>,
}

impl AuthState {
    pub(crate) fn new() -> Self {
        Self::with_ttl(SESSION_TTL)
    }

    fn with_ttl(ttl: Duration) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            pin_attempts: Mutex::new(VecDeque::new()),
            ttl,
            machine_credential_digest: Mutex::new(None),
            pin_generation: AtomicU64::new(0),
            session_gate: Mutex::new(()),
        }
    }

    #[cfg(test)]
    fn issue(&self) -> Result<String, ()> {
        self.issue_unlocked()
    }

    pub(crate) fn pin_generation(&self) -> u64 {
        self.pin_generation.load(Ordering::Acquire)
    }

    pub(crate) fn issue_for_pin_generation(&self, expected: u64) -> Result<String, ()> {
        let _gate = self.session_gate.lock().expect("auth gate mutex poisoned");
        if self.pin_generation.load(Ordering::Acquire) != expected {
            return Err(());
        }
        self.issue_unlocked()
    }

    fn issue_unlocked(&self) -> Result<String, ()> {
        let mut bytes = [0u8; SESSION_TOKEN_BYTES];
        getrandom::fill(&mut bytes).map_err(|_| ())?;
        let token = hex(&bytes);
        let now = Instant::now();
        let mut sessions = self.sessions.lock().expect("auth session mutex poisoned");
        sessions.retain(|_, session| session.expires_at > now);

        // A bounded registry is important on a shared venue LAN.  Evict the oldest session
        // when a pathological number of browsers asks for a new one, rather than allowing
        // memory growth or refusing the operator who needs to recover the machine.
        if sessions.len() >= MAX_SESSIONS {
            if let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, session)| session.issued_at)
                .map(|(token, _)| token.clone())
            {
                sessions.remove(&oldest);
            }
        }
        sessions.insert(
            token.clone(),
            Session {
                issued_at: now,
                expires_at: now + self.ttl,
            },
        );
        Ok(token)
    }

    pub(crate) fn authorize(&self, token: &str) -> bool {
        let now = Instant::now();
        let mut sessions = self.sessions.lock().expect("auth session mutex poisoned");
        sessions.retain(|_, session| session.expires_at > now);
        sessions.contains_key(token)
    }

    pub(crate) fn revoke_all(&self) {
        let _gate = self.session_gate.lock().expect("auth gate mutex poisoned");
        self.pin_generation.fetch_add(1, Ordering::AcqRel);
        self.sessions
            .lock()
            .expect("auth session mutex poisoned")
            .clear();
    }

    /// Count all attempts, including successful ones, inside the window.  A successful
    /// attempt clears the window, while a failed sequence is bounded before a four-digit PIN
    /// can be rapidly enumerated.
    pub(crate) fn begin_pin_attempt(&self) -> bool {
        let now = Instant::now();
        let cutoff = now.checked_sub(PIN_WINDOW).unwrap_or(now);
        let mut attempts = self.pin_attempts.lock().expect("PIN mutex poisoned");
        while attempts.front().is_some_and(|attempt| *attempt <= cutoff) {
            attempts.pop_front();
        }
        if attempts.len() >= MAX_PIN_ATTEMPTS {
            return false;
        }
        attempts.push_back(now);
        true
    }

    pub(crate) fn pin_succeeded(&self) {
        self.pin_attempts
            .lock()
            .expect("PIN mutex poisoned")
            .clear();
    }

    pub(crate) fn set_machine_credential(&self, credential: &str) {
        let digest = if credential.trim().is_empty() {
            None
        } else {
            Some(digest(credential.trim().as_bytes()))
        };
        *self
            .machine_credential_digest
            .lock()
            .expect("machine credential mutex poisoned") = digest;
    }

    pub(crate) fn machine_credential_matches(&self, credential: &str) -> bool {
        let expected = self
            .machine_credential_digest
            .lock()
            .expect("machine credential mutex poisoned");
        let Some(expected) = expected.as_ref() else {
            return false;
        };
        constant_time_eq(expected, &digest(credential.trim().as_bytes()))
    }

    #[cfg(test)]
    fn generation_is_advanced_by_revoke(&self) {
        let before = self.pin_generation();
        self.revoke_all();
        assert_eq!(self.pin_generation(), before + 1);
    }
}

/// A valid operator browser session.  It is deliberately a zero-sized extractor: handlers
/// only need proof of authorization, while audit identity still comes from
/// [`OperatorDevice`].
#[derive(Clone, Copy, Debug, Default)]
pub struct OperatorSession;

impl<S> FromRequestParts<Operator<S>> for OperatorSession
where
    S: HubStore + Send + Sync + 'static,
    S::Error: Display + Send,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        operator: &Operator<S>,
    ) -> Result<Self, Self::Rejection> {
        let Some(token) = cookie_value(parts) else {
            return Err(ApiError::auth_required());
        };
        if operator.authorize_session(&token) {
            Ok(Self)
        } else {
            Err(ApiError::auth_required())
        }
    }
}

/// The backup endpoint accepts either an authenticated browser or the separately configured
/// machine credential.  The latter is intentionally a distinct principal and is only used by
/// this handler; it cannot authorize PIN changes, settings, power or reader operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupPrincipal {
    Operator(String),
    Machine,
}

impl BackupPrincipal {
    pub fn operator_name(&self) -> &str {
        match self {
            Self::Operator(name) => name,
            Self::Machine => "MACHINE_BACKUP",
        }
    }
}

impl<S> FromRequestParts<Operator<S>> for BackupPrincipal
where
    S: HubStore + Send + Sync + 'static,
    S::Error: Display + Send,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        operator: &Operator<S>,
    ) -> Result<Self, Self::Rejection> {
        if let Some(token) = cookie_value(parts) {
            if operator.authorize_session(&token) {
                let device = OperatorDevice::parse(parts)?;
                return Ok(Self::Operator(device.0));
            }
        }

        if let Some(credential) = bearer_value(parts) {
            if operator.authorize_machine_credential(&credential) {
                return Ok(Self::Machine);
            }
        }

        Err(ApiError::auth_required())
    }
}

pub(crate) fn session_cookie(token: &str) -> String {
    // Secure is deliberately absent: the appliance is commonly reached over its existing
    // HTTP LAN address, and forcing TLS here would make the authorized on-site flow fail.
    format!(
        "{SESSION_COOKIE}={token}; Path=/; Max-Age={}; HttpOnly; SameSite=Strict",
        SESSION_TTL.as_secs()
    )
}

pub(crate) fn clear_session_cookie() -> &'static str {
    "hyrox_operator_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict"
}

fn cookie_value(parts: &Parts) -> Option<String> {
    let raw = parts.headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == SESSION_COOKIE && !value.is_empty()).then(|| value.to_string())
    })
}

fn bearer_value(parts: &Parts) -> Option<String> {
    let raw = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, value) = raw.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !value.trim().is_empty())
        .then(|| value.trim().to_string())
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{AuthState, MAX_PIN_ATTEMPTS, MAX_SESSIONS};
    use std::time::{Duration, Instant};

    #[test]
    fn issued_tokens_are_random_and_expire_on_monotonic_time() {
        let auth = AuthState::with_ttl(Duration::from_secs(60));
        let first = auth.issue().expect("entropy");
        let second = auth.issue().expect("entropy");
        assert_ne!(first, second);
        assert!(auth.authorize(&second));
        auth.sessions
            .lock()
            .unwrap()
            .get_mut(&first)
            .expect("first session")
            .expires_at = Instant::now() - Duration::from_secs(1);
        auth.sessions
            .lock()
            .unwrap()
            .get_mut(&second)
            .expect("second session")
            .expires_at = Instant::now() - Duration::from_secs(1);
        assert!(!auth.authorize(&first));
        assert!(!auth.authorize(&second));
    }

    #[test]
    fn session_registry_and_pin_attempts_are_bounded() {
        let auth = AuthState::new();
        for _ in 0..(MAX_SESSIONS + 10) {
            auth.issue().expect("entropy");
        }
        assert!(auth.sessions.lock().unwrap().len() <= MAX_SESSIONS);

        for _ in 0..MAX_PIN_ATTEMPTS {
            assert!(auth.begin_pin_attempt());
        }
        assert!(!auth.begin_pin_attempt());
        auth.pin_succeeded();
        assert!(auth.begin_pin_attempt());
    }

    #[test]
    fn machine_credential_is_separate_from_operator_sessions() {
        let auth = AuthState::new();
        auth.set_machine_credential("machine-secret");
        assert!(auth.machine_credential_matches("machine-secret"));
        assert!(!auth.machine_credential_matches("wrong"));
        assert!(!auth.machine_credential_matches(""));
    }

    #[test]
    fn revoking_sessions_advances_the_pin_generation() {
        let auth = AuthState::new();
        auth.generation_is_advanced_by_revoke();
    }
}
