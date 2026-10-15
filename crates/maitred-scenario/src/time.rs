//! Scenario time: milliseconds since the scenario started.
//!
//! A plain count rather than `std::time::Duration`, so a scenario is data that both targets read the same
//! way: the simulator steps through it, and the real-process runner turns it into waits in the test kit's
//! `timing.rs`.

use std::fmt;
use std::ops::{Add, Sub};

/// A moment in a scenario, or a span of time, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ms(pub u64);

/// `n` milliseconds.
#[must_use]
pub const fn ms(n: u64) -> Ms {
    Ms(n)
}

/// `n` seconds.
#[must_use]
pub const fn secs(n: u64) -> Ms {
    Ms(n * 1_000)
}

impl Ms {
    /// The time between two moments, whichever comes first.
    #[must_use]
    pub const fn apart(self, other: Ms) -> Ms {
        Ms(self.0.abs_diff(other.0))
    }
}

impl Add for Ms {
    type Output = Ms;

    fn add(self, other: Ms) -> Ms {
        Ms(self.0 + other.0)
    }
}

impl Sub for Ms {
    type Output = Ms;

    fn sub(self, other: Ms) -> Ms {
        Ms(self.0.saturating_sub(other.0))
    }
}

impl fmt::Display for Ms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ms", self.0)
    }
}
