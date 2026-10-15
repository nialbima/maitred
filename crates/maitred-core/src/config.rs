use std::collections::BTreeMap;

use crate::{Class, Pool, Resources};

/// The machine config's numbers that `decide` and `choose_victim` read. Every length of time is in
/// milliseconds (ARCHITECTURE.md, Test kit, Time: every duration is a named config value).
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub poll_interval_ms: u64,
    pub sample_interval_ms: u64,
    /// The least time between two heavy admissions.
    pub settle_ms: u64,
    /// How long a ticket waits before it holds its place in the budget.
    pub aging_ms: u64,
    /// How long an aged ticket holds its place before the hold lapses.
    pub aging_hold_ms: u64,
    /// How long a quiet ticket waits before the queue drains for it.
    pub quiet_max_wait_ms: u64,
    /// How long the queue drains for a due quiet ticket.
    pub quiet_hold_ms: u64,
    /// How many poll intervals without a pass make a waiting ticket stalled.
    pub stall_polls: u64,
    /// How many sample intervals old the load file may be and still count as fresh.
    pub load_freshness_samples: u64,
    pub mem_floor_mb: u64,
    /// Runnable work per core, over the last sample interval.
    pub cpu_ceiling: f64,
    /// GPU utilisation, 0 to 1.
    pub gpu_ceiling: f64,
    pub max_running_heavy: u32,
    /// The quiet streak, in samples, that lets a quiet job in.
    pub quiet_streak_needed: u32,
    pub class_min: ClassMinimums,
    /// Each declared pool and its size.
    pub pools: BTreeMap<Pool, u32>,
}

impl Config {
    /// The stall limit: a waiting ticket whose last pass is older than this is stalled.
    #[must_use]
    pub fn stall_limit_ms(&self) -> u64 {
        self.stall_polls.saturating_mul(self.poll_interval_ms)
    }

    /// How old the load file may be and still count as fresh.
    #[must_use]
    pub fn load_freshness_ms(&self) -> u64 {
        self.load_freshness_samples
            .saturating_mul(self.sample_interval_ms)
    }
}

/// The least a ticket of each class reserves, whatever it asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClassMinimums {
    pub light: Resources,
    pub heavy: Resources,
    pub quiet: Resources,
}

impl ClassMinimums {
    #[must_use]
    pub fn of(&self, class: Class) -> Resources {
        match class {
            Class::Light => self.light,
            Class::Heavy => self.heavy,
            Class::Quiet => self.quiet,
        }
    }
}
