//! Per-node budget for pairing claims.
//!
//! A claim is unauthenticated by design — approval on the desktop is the gate —
//! so the only cost to an attacker is the request itself. A fixed window per
//! node id keeps one peer from filling the pending table or spamming the
//! approval UI, and keying by node id (rather than socket address) means a
//! household behind one NAT is unaffected while a single identity is still
//! bounded.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Fixed-window counter, keyed by node id.
pub struct ClaimRateLimiter {
    window: Duration,
    max: u32,
    windows: Mutex<HashMap<String, (Instant, u32)>>,
}

impl ClaimRateLimiter {
    /// Allow `max` claims per node per `window`.
    pub fn new(window: Duration, max: u32) -> Self {
        Self {
            window,
            max,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Count a claim from `node_id` at `now`; `true` when it is within budget.
    ///
    /// Closed windows are dropped first, so the map is bounded by the peers
    /// that claimed inside the current window rather than by every peer seen.
    pub fn allow_at(&self, node_id: &str, now: Instant) -> bool {
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        windows.retain(|_, (start, _)| now.duration_since(*start) < self.window);

        let entry = windows.entry(node_id.to_string()).or_insert((now, 0));
        if entry.1 >= self.max {
            return false;
        }
        entry.1 += 1;
        true
    }

    /// [`Self::allow_at`] at the current instant.
    pub fn allow(&self, node_id: &str) -> bool {
        self.allow_at(node_id, Instant::now())
    }
}

impl Default for ClaimRateLimiter {
    /// Ten claims a minute per node id: generous for a person retrying a scan,
    /// tight enough that a script cannot occupy the pairing table.
    fn default() -> Self {
        Self::new(Duration::from_secs(60), 10)
    }
}

/// The process-wide limiter a pairing claim is checked against.
pub fn claim_limiter() -> &'static ClaimRateLimiter {
    static LIMITER: LazyLock<ClaimRateLimiter> = LazyLock::new(ClaimRateLimiter::default);
    &LIMITER
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_budget_holds_until_the_window_closes() {
        let limiter = ClaimRateLimiter::new(Duration::from_secs(60), 2);
        let start = Instant::now();

        assert!(limiter.allow_at("node-a", start));
        assert!(limiter.allow_at("node-a", start));
        assert!(
            !limiter.allow_at("node-a", start),
            "the third claim in the window is over budget"
        );
        assert!(
            limiter.allow_at("node-b", start),
            "another node has its own budget"
        );

        assert!(
            limiter.allow_at("node-a", start + Duration::from_secs(61)),
            "the window reopened"
        );
    }

    #[test]
    fn the_shared_limiter_is_keyed_by_node_id() {
        let limiter = claim_limiter();
        for _ in 0..10 {
            assert!(limiter.allow("shared-key-test"));
        }
        assert!(!limiter.allow("shared-key-test"));
    }
}
