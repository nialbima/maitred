//! The test kit's own timing, in one file (ARCHITECTURE.md, Test kit: Time). Waits and timeouts the test
//! kit chooses are test timing, not maitred's config, so they are named here and nowhere else.

use std::time::Duration;

/// A count of milliseconds as a `Duration`. Every scenario time and every command-line time reaches the
/// standard library through this one function.
#[must_use]
pub fn millis(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// How finely `burn` divides a core it runs at part load: it spins for its share of each period and sleeps
/// for the rest.
#[must_use]
pub fn burn_duty_period() -> Duration {
    Duration::from_millis(10)
}

/// How long a test waits for something that must happen before calling it missing. No load on a working machine
/// comes near it, so only a failing test reaches it; it is there so a hung process fails the test with a message
/// instead of hanging the run (MT-054).
#[must_use]
pub fn wait_ceiling() -> Duration {
    Duration::from_secs(60)
}

/// How long a process runs when a test must never see it end by itself: far past `wait_ceiling`, so a test that
/// waits on it fails at the ceiling rather than passing because time ran out. Tests kill it when they are done.
pub const OUTLIVE_MS: u64 = 600_000;

/// How often the pause controller looks for a newly paused process while it waits for one.
#[must_use]
pub fn pause_accept_retry() -> Duration {
    Duration::from_millis(5)
}
