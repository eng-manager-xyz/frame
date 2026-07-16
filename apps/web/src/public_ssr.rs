use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use frame_client::{
    ClientError, ClientErrorCode, FrameClient, FrameOrigin, Health, NativeTransport,
    PublicShareSummary, TransportPolicy,
};

use crate::config::RuntimeConfig;

const FAILURE_THRESHOLD: u8 = 3;
const OPEN_INTERVAL: Duration = Duration::from_secs(10);

/// Anonymous, fixed-origin SSR reader. The public origin remains the contract
/// authority while preview deployments may send requests to a separate,
/// validated staging API origin. The transport disables ambient proxies and
/// redirects and enforces the configured deadline and response-size bound.
#[derive(Clone)]
pub struct PublicSsr {
    client: Arc<FrameClient<NativeTransport>>,
    circuit: Arc<Mutex<CircuitState>>,
}

impl PublicSsr {
    pub fn from_config(config: &RuntimeConfig) -> Result<Self, ClientError> {
        let public_origin = FrameOrigin::parse_https(config.public_origin().as_str())?;
        let api_origin = FrameOrigin::parse_https(config.api_origin().as_str())?;
        let policy = TransportPolicy::new(Duration::from_millis(1_500), 64 * 1024, 2)?;
        let client = FrameClient::new(public_origin, NativeTransport::new()?)
            .with_api_origin(api_origin)
            .with_policy(policy);
        Ok(Self {
            client: Arc::new(client),
            circuit: Arc::new(Mutex::new(CircuitState::default())),
        })
    }

    pub async fn health(&self) -> Result<Health, ClientError> {
        let permit = CircuitPermit::acquire(Arc::clone(&self.circuit))?;
        let result = self.client.health().await;
        permit.finish(!result.as_ref().is_err_and(circuit_failure));
        result
    }

    pub async fn public_share(&self, identifier: &str) -> Result<PublicShareSummary, ClientError> {
        let permit = CircuitPermit::acquire(Arc::clone(&self.circuit))?;
        let result = self.client.public_share(identifier).await;
        permit.finish(!result.as_ref().is_err_and(circuit_failure));
        result
    }

    #[must_use]
    pub fn circuit_status(&self) -> &'static str {
        self.circuit
            .lock()
            .map_or("unavailable", |state| state.status(Instant::now()))
    }
}

fn circuit_failure(error: &ClientError) -> bool {
    matches!(
        error.code(),
        ClientErrorCode::IncompatibleVersion
            | ClientErrorCode::InvalidContract
            | ClientErrorCode::PrivacyViolation
            | ClientErrorCode::DeadlineExceeded
            | ClientErrorCode::TransportUnavailable
            | ClientErrorCode::ResponseTooLarge
            | ClientErrorCode::UnsupportedContentType
            | ClientErrorCode::RedirectRejected
            | ClientErrorCode::MalformedResponse
    )
}

#[derive(Default)]
struct CircuitState {
    consecutive_failures: u8,
    open_until: Option<Instant>,
    half_open_probe: bool,
}

impl CircuitState {
    fn begin(&mut self, now: Instant) -> bool {
        let Some(open_until) = self.open_until else {
            return true;
        };
        if now < open_until || self.half_open_probe {
            return false;
        }
        self.half_open_probe = true;
        true
    }

    fn finish(&mut self, success: bool, now: Instant) {
        self.half_open_probe = false;
        if success {
            self.consecutive_failures = 0;
            self.open_until = None;
            return;
        }
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= FAILURE_THRESHOLD {
            self.open_until = Some(now + OPEN_INTERVAL);
        }
    }

    fn status(&self, now: Instant) -> &'static str {
        match self.open_until {
            Some(until) if now < until => "open",
            Some(_) => "half_open",
            None => "closed",
        }
    }
}

struct CircuitPermit {
    state: Arc<Mutex<CircuitState>>,
    finished: bool,
}

impl CircuitPermit {
    fn acquire(state: Arc<Mutex<CircuitState>>) -> Result<Self, ClientError> {
        let admitted = state
            .lock()
            .map_err(|_| ClientError::transport_unavailable())?
            .begin(Instant::now());
        if !admitted {
            return Err(ClientError::transport_unavailable());
        }
        Ok(Self {
            state,
            finished: false,
        })
    }

    fn finish(mut self, success: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.finish(success, Instant::now());
        }
        self.finished = true;
    }
}

impl Drop for CircuitPermit {
    fn drop(&mut self) {
        if !self.finished
            && let Ok(mut state) = self.state.lock()
        {
            state.finish(false, Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_opens_after_bounded_failures_and_allows_one_recovery_probe() {
        let start = Instant::now();
        let mut state = CircuitState::default();
        for _ in 0..FAILURE_THRESHOLD {
            assert!(state.begin(start));
            state.finish(false, start);
        }
        assert_eq!(state.status(start), "open");
        assert!(!state.begin(start + Duration::from_secs(9)));

        let recovery = start + OPEN_INTERVAL;
        assert!(state.begin(recovery));
        assert!(!state.begin(recovery));
        state.finish(true, recovery);
        assert_eq!(state.status(recovery), "closed");
        assert!(state.begin(recovery));
    }

    #[test]
    fn failed_half_open_probe_reopens_the_circuit() {
        let start = Instant::now();
        let mut state = CircuitState {
            consecutive_failures: FAILURE_THRESHOLD,
            open_until: Some(start),
            half_open_probe: false,
        };
        assert!(state.begin(start));
        state.finish(false, start);
        assert_eq!(state.status(start), "open");
    }
}
