//! Per-source budget for `POST /sync/pair`.
//!
//! A claim is unauthenticated by design — approval on the desktop is the gate —
//! so the only cost to an attacker is the request itself. A fixed window per
//! source keeps one host from filling the pending table or spamming the
//! approval UI while leaving a whole household behind one NAT able to pair.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Fixed-window counter, keyed by source address.
pub struct ClaimRateLimiter {
    window: Duration,
    max: u32,
    windows: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl ClaimRateLimiter {
    /// Allow `max` claims per source per `window`.
    pub fn new(window: Duration, max: u32) -> Self {
        Self {
            window,
            max,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Count a claim from `ip` at `now`; `true` when it is within budget.
    ///
    /// Closed windows are dropped first, so the map is bounded by the peers
    /// that claimed inside the current window rather than by every peer seen.
    pub fn allow_at(&self, ip: IpAddr, now: Instant) -> bool {
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        windows.retain(|_, (start, _)| now.duration_since(*start) < self.window);

        let entry = windows.entry(ip).or_insert((now, 0));
        if entry.1 >= self.max {
            return false;
        }
        entry.1 += 1;
        true
    }

    /// [`Self::allow_at`] at the current instant.
    pub fn allow(&self, ip: IpAddr) -> bool {
        self.allow_at(ip, Instant::now())
    }
}

impl Default for ClaimRateLimiter {
    /// Ten claims a minute per source: generous for a person retrying a scan,
    /// tight enough that a script cannot occupy the pairing table.
    fn default() -> Self {
        Self::new(Duration::from_secs(60), 10)
    }
}
