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
