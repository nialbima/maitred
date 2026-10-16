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
    /// How long a quiet ticket waits before the queue drains for it, until it gets in.
    pub quiet_max_wait_ms: u64,
    /// How many poll intervals without a pass make a waiting ticket stalled.
    pub stall_polls: u64,
    /// How many sample intervals old the load file may be and still count as fresh.
    pub load_freshness_samples: u64,
    pub mem_floor_mb: u64,
    /// Free memory below this triggers a kill (below the admission floor).
    pub kill_floor_mb: u64,
    /// Free memory below this lets the kill policy choose a quiet job.
    pub mem_critical_mb: u64,
    /// Runnable work per core, over the last sample interval.
    pub cpu_ceiling: f64,
    /// GPU utilisation, 0 to 1.
    pub gpu_ceiling: f64,
    /// How many consecutive samples over `cpu_kill_ceiling` trigger a kill; twice as many lets the
    /// kill policy choose a quiet job.
    pub cpu_kill_samples: u32,
    /// How many consecutive samples over `gpu_kill_ceiling` trigger a kill.
    pub gpu_kill_samples: u32,
    pub max_running_heavy: u32,
    /// The quiet streak, in samples, that lets a quiet job in.
    pub quiet_streak_needed: u32,
    pub class_min: ClassMinimums,
    /// Each declared pool and its range of numbers.
    pub pools: BTreeMap<Pool, PoolConfig>,
}

impl Config {
    /// The stall limit: a waiting ticket whose last pass is older than this is stalled.
    #[must_use]
    pub fn stall_limit_ms(&self) -> u64 {
        self.stall_polls.saturating_mul(self.poll_interval_ms)
    }

    /// What a ticket of `class` that asked for `asked` holds: at least the class minimum, at most
    /// the machine.
    #[must_use]
    pub fn reserve(&self, class: Class, asked: Resources, capacity: Resources) -> Resources {
        asked.max(self.class_min.of(class)).min(capacity)
    }

    /// Checks the thresholds whose wrong values would make the kill policy misbehave.
    ///
    /// # Errors
    ///
    /// The first threshold that is out of order.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.cpu_kill_samples == 0 {
            return Err(ConfigError::KillSamples("cpu_kill_samples"));
        }
        if self.gpu_kill_samples == 0 {
            return Err(ConfigError::KillSamples("gpu_kill_samples"));
        }
        if !(self.mem_critical_mb < self.kill_floor_mb && self.kill_floor_mb < self.mem_floor_mb) {
            return Err(ConfigError::MemoryThresholds);
        }
        Ok(())
    }

    /// How old the load file may be and still count as fresh.
    #[must_use]
    pub fn load_freshness_ms(&self) -> u64 {
        self.load_freshness_samples
            .saturating_mul(self.sample_interval_ms)
    }
}

/// A machine config that would make the kill policy misbehave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// A kill streak of 0 samples would trigger on a healthy machine, critically.
    KillSamples(&'static str),
    /// The memory thresholds must rise: `mem_critical` < `kill_floor` < `mem_floor`.
    MemoryThresholds,
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

/// A slot pool's numbers: `first` to `first + size - 1`. How a number is written for the job (the
/// variable, an empty first slot) is the supervisor's business, not the core's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolConfig {
    pub first: u32,
    pub size: u32,
}

impl PoolConfig {
    /// Whether `n` is one of this pool's numbers. A number outside the range is retired (the pool
    /// shrank): its holder keeps it until it ends, but it uses up none of the range.
    #[must_use]
    pub fn contains(&self, n: u32) -> bool {
        n >= self.first && n - self.first < self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Config {
        Config {
            poll_interval_ms: 2_000,
            sample_interval_ms: 2_000,
            settle_ms: 30_000,
            aging_ms: 600_000,
            aging_hold_ms: 600_000,
            quiet_max_wait_ms: 3_600_000,
            stall_polls: 5,
            load_freshness_samples: 3,
            mem_floor_mb: 1_000,
            kill_floor_mb: 500,
            mem_critical_mb: 200,
            cpu_ceiling: 1.5,
            gpu_ceiling: 0.9,
            cpu_kill_samples: 3,
            gpu_kill_samples: 3,
            max_running_heavy: 2,
            quiet_streak_needed: 2,
            class_min: ClassMinimums::default(),
            pools: BTreeMap::new(),
        }
    }

    #[test]
    fn a_sensible_config_is_valid() {
        assert_eq!(valid().validate(), Ok(()));
    }

    #[test]
    fn a_cpu_kill_after_zero_samples_is_refused() {
        let config = Config {
            cpu_kill_samples: 0,
            ..valid()
        };

        assert_eq!(
            config.validate(),
            Err(ConfigError::KillSamples("cpu_kill_samples"))
        );
    }

    #[test]
    fn a_gpu_kill_after_zero_samples_is_refused() {
        let config = Config {
            gpu_kill_samples: 0,
            ..valid()
        };

        assert_eq!(
            config.validate(),
            Err(ConfigError::KillSamples("gpu_kill_samples"))
        );
    }

    #[test]
    fn mem_critical_at_or_above_the_kill_floor_is_refused() {
        let config = Config {
            mem_critical_mb: 500,
            ..valid()
        };

        assert_eq!(config.validate(), Err(ConfigError::MemoryThresholds));
    }

    #[test]
    fn a_kill_floor_at_or_above_the_admission_floor_is_refused() {
        let config = Config {
            kill_floor_mb: 1_000,
            ..valid()
        };

        assert_eq!(config.validate(), Err(ConfigError::MemoryThresholds));
    }
}
