//! The machine config a scenario runs under (ARCHITECTURE.md: Admission, Kill policy, Time).
//!
//! The simulator converts it into maitred's own config type; the real-process runner writes it out as the
//! machine's `config.toml`. Every value ARCHITECTURE names is here, so neither target falls back on a
//! default the scenario didn't choose.

use crate::Class;
use crate::scenario::Invalid;
use crate::time::{Ms, ms, secs};

/// Cores in millicores, memory in megabytes: the units ARCHITECTURE uses throughout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Amount {
    pub cpu_millicores: u32,
    pub mem_mb: u32,
}

/// One class's minimum and default reservation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClassConfig {
    pub min: Amount,
    pub default: Amount,
}

/// A numbered set of things only one job may use at a time (ARCHITECTURE.md: Slot pools).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pool {
    pub name: String,
    pub size: u32,
    /// The environment variable that carries a job's slot number.
    pub variable: String,
    /// The first slot's number; the pool's numbers run from it to `first + size - 1`.
    pub first: u32,
    /// Whether the first slot is passed as an empty string rather than its number (the `parallel_tests` convention).
    pub empty_first: bool,
}

/// Which platform's CPU figures the machine has (ARCHITECTURE.md: Inside `decide`, `cpu_load`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    /// `cpu_stall` for the CPU limit and the CPU kill; `cpu_runnable_per_core` recorded for comparison only.
    Linux,
    /// No stall figure: `cpu_runnable_per_core` for the CPU limit, maitred's own running threads for the CPU kill.
    MacOs,
}

/// The machine's settings, as its administrator would write them.
///
/// Pressure and utilisation figures are in thousandths (0.42 is 420), so the type stays comparable.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_field_names)]
pub struct MachineConfig {
    pub platform: Platform,
    pub capacity: Amount,
    // Live limits: nothing starts past these. Each CPU ceiling applies only on the platform whose figure it reads.
    pub mem_floor_mb: u32,
    pub cpu_stall_ceiling_permille: u32,
    pub cpu_runnable_ceiling_permille: u32,
    pub gpu_ceiling_permille: u32,
    // Kill thresholds.
    pub kill_floor_mb: u32,
    pub mem_critical_mb: u32,
    pub cpu_stall_kill_ceiling_permille: u32,
    pub cpu_threads_kill_ceiling_permille: u32,
    pub cpu_kill_samples: u32,
    pub gpu_kill_ceiling_permille: u32,
    pub gpu_kill_samples: u32,
    // Classes.
    pub light: ClassConfig,
    pub heavy: ClassConfig,
    pub quiet: ClassConfig,
    pub max_running_heavy: u32,
    pub quiet_threshold_permille: u32,
    // Durations named as machine config.
    pub sample_interval: Ms,
    pub poll_interval: Ms,
    pub grace: Ms,
    pub settle: Ms,
    pub aging: Ms,
    pub aging_hold: Ms,
    pub quiet_max_wait: Ms,
    pub orphan_ttl: Ms,
    pub kill_wait: Ms,
    // Named constants a debug build lets the config override (ARCHITECTURE.md, Test kit: Time).
    pub admit_lock_retry: Ms,
    pub admit_lock_notice: Ms,
    pub checker_timeout: Ms,
    pub new_file_age: Ms,
    pub request_dir_age: Ms,
    pub stale_wait: Ms,
    pub stall_limit: Ms,
    pub load_freshness: Ms,
    pub pools: Vec<Pool>,
}

impl MachineConfig {
    /// An 8-core, 16 GB Linux machine with every duration short enough for a test to run in seconds.
    ///
    /// The ratios between durations follow ARCHITECTURE's defaults (stall limit 5 poll intervals,
    /// freshness 3 sample intervals), so shortening time doesn't change which rule fires first. The CPU ceilings are
    /// the doc's starting values: 0.3 stall and 1.5 runnable per core to admit, 0.6 stall and 2.0 threads per core to
    /// kill (ARCHITECTURE rev 190).
    #[must_use]
    pub fn for_tests() -> Self {
        let sample = ms(200);
        let poll = ms(200);
        Self {
            platform: Platform::Linux,
            capacity: Amount {
                cpu_millicores: 8_000,
                mem_mb: 16_384,
            },
            mem_floor_mb: 1_024,
            cpu_stall_ceiling_permille: 300,
            cpu_runnable_ceiling_permille: 1_500,
            gpu_ceiling_permille: 950,
            kill_floor_mb: 512,
            mem_critical_mb: 256,
            cpu_stall_kill_ceiling_permille: 600,
            cpu_threads_kill_ceiling_permille: 2_000,
            cpu_kill_samples: 3,
            gpu_kill_ceiling_permille: 980,
            gpu_kill_samples: 3,
            light: ClassConfig {
                min: Amount {
                    cpu_millicores: 250,
                    mem_mb: 256,
                },
                default: Amount {
                    cpu_millicores: 1_000,
                    mem_mb: 512,
                },
            },
            heavy: ClassConfig {
                min: Amount {
                    cpu_millicores: 2_000,
                    mem_mb: 1_024,
                },
                default: Amount {
                    cpu_millicores: 4_000,
                    mem_mb: 2_048,
                },
            },
            quiet: ClassConfig {
                min: Amount {
                    cpu_millicores: 1_000,
                    mem_mb: 512,
                },
                default: Amount {
                    cpu_millicores: 1_000,
                    mem_mb: 512,
                },
            },
            max_running_heavy: 2,
            quiet_threshold_permille: 100,
            sample_interval: sample,
            poll_interval: poll,
            grace: secs(1),
            settle: ms(600),
            aging: secs(3),
            aging_hold: secs(3),
            quiet_max_wait: secs(5),
            orphan_ttl: secs(30),
            kill_wait: secs(1),
            admit_lock_retry: ms(10),
            admit_lock_notice: secs(2),
            checker_timeout: secs(1),
            new_file_age: secs(2),
            request_dir_age: secs(2),
            stale_wait: Ms(sample.0 * 3),
            stall_limit: Ms(poll.0 * 5),
            load_freshness: Ms(sample.0 * 3),
            pools: Vec::new(),
        }
    }

    /// The same machine running macOS.
    #[must_use]
    pub fn on_macos(mut self) -> Self {
        self.platform = Platform::MacOs;
        self
    }

    /// Declares a slot pool numbered from 1.
    #[must_use]
    pub fn pool(mut self, name: &str, size: u32, variable: &str) -> Self {
        self.pools.push(Pool {
            name: name.into(),
            size,
            variable: variable.into(),
            first: 1,
            empty_first: false,
        });
        self
    }

    /// Changes how the last declared pool numbers its slots.
    ///
    /// # Panics
    ///
    /// If no pool has been declared yet.
    #[must_use]
    pub fn numbered_from(mut self, first: u32, empty_first: bool) -> Self {
        let pool = self
            .pools
            .last_mut()
            .expect("numbered_from needs a pool declared before it");
        pool.first = first;
        pool.empty_first = empty_first;
        self
    }

    /// Sets how often a waiting supervisor runs an admission pass, keeping the stall limit at 5 of them.
    #[must_use]
    pub fn poll_interval(mut self, interval: Ms) -> Self {
        self.poll_interval = interval;
        self.stall_limit = Ms(interval.0 * 5);
        self
    }

    /// One class's minimum and default reservation.
    #[must_use]
    pub fn class(&self, class: Class) -> ClassConfig {
        match class {
            Class::Light => self.light,
            Class::Heavy => self.heavy,
            Class::Quiet => self.quiet,
        }
    }

    /// Each rule maitred refuses this config by (ARCHITECTURE.md, Settle and the minimums): a ceiling that can never
    /// be crossed would turn its rule off without a word.
    pub(crate) fn refused(&self) -> Vec<Invalid> {
        let stall_ok = |permille: u32| (1..=1_000).contains(&permille);
        [
            (
                self.cpu_kill_samples >= 1 && self.gpu_kill_samples >= 1,
                "cpu_kill_samples and gpu_kill_samples at least 1",
            ),
            (
                self.mem_critical_mb < self.kill_floor_mb && self.kill_floor_mb < self.mem_floor_mb,
                "mem_critical below kill_floor below mem_floor",
            ),
            (
                stall_ok(self.cpu_stall_ceiling_permille)
                    && stall_ok(self.cpu_stall_kill_ceiling_permille),
                "the stall ceilings within 0 to 1 and above 0",
            ),
            (
                self.cpu_runnable_ceiling_permille > 0
                    && self.cpu_threads_kill_ceiling_permille > 0,
                "the runnable and thread ceilings above 0",
            ),
            (
                self.cpu_stall_kill_ceiling_permille > self.cpu_stall_ceiling_permille,
                "cpu_stall_kill_ceiling above cpu_stall_ceiling",
            ),
            (
                (1..1_000).contains(&self.quiet_threshold_permille),
                "quiet_threshold above 0 and below 1",
            ),
        ]
        .into_iter()
        .filter(|(holds, _)| !holds)
        .map(|(_, rule)| Invalid::ConfigRefused(rule))
        .collect()
    }

    pub(crate) fn declares_pool(&self, name: &str) -> bool {
        self.pools.iter().any(|pool| pool.name == name)
    }
}
