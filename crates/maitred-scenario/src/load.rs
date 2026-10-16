//! The load a scenario puts on the machine, and the load file both targets publish from it.
//!
//! A [`Sample`] is the trace's background load at one moment: work no maitred job accounts for. The
//! [`Publisher`] adds each running job's declared use and keeps the streaks the sampler would keep, giving the
//! [`Published`] figures that `load/now.json` carries (ARCHITECTURE.md: `load/now.json`). The simulator and
//! the test kit's `replay` both call it, so the two targets read identical load.

use crate::machine::{Amount, MachineConfig};
use crate::time::Ms;

/// A critical or optional reading the sampler couldn't take (ARCHITECTURE.md: Critical readings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reading {
    Cpu,
    Memory,
    CpuPressure,
    Gpu,
}

/// The background load at the end of one sample interval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    /// When the sample was taken; its interval runs from the previous sample (or one sample interval before, for
    /// the first).
    pub at: Ms,
    /// CPU demand from outside maitred, in millicores. Demand past capacity shows as pressure.
    pub cpu_demand_millicores: u32,
    /// Memory in use outside maitred.
    pub mem_used_mb: u32,
    /// GPU utilisation in thousandths, if the machine has a readable GPU.
    pub gpu_permille: Option<u32>,
    /// Set on the first sample after the machine woke, to when it woke.
    pub woke_at: Option<Ms>,
    pub missing: Vec<Reading>,
}

impl Sample {
    /// An idle machine at `at`.
    #[must_use]
    pub fn at(at: Ms) -> Self {
        Self {
            at,
            cpu_demand_millicores: 0,
            mem_used_mb: 0,
            gpu_permille: None,
            woke_at: None,
            missing: Vec::new(),
        }
    }

    /// Sets the background CPU demand and memory in use.
    #[must_use]
    pub fn background(mut self, cpu_demand_millicores: u32, mem_used_mb: u32) -> Self {
        self.cpu_demand_millicores = cpu_demand_millicores;
        self.mem_used_mb = mem_used_mb;
        self
    }

    /// Sets the GPU's utilisation.
    #[must_use]
    pub fn gpu(mut self, permille: u32) -> Self {
        self.gpu_permille = Some(permille);
        self
    }

    /// Marks this as the first sample after the machine woke at `woke`.
    #[must_use]
    pub fn woke_at(mut self, woke: Ms) -> Self {
        self.woke_at = Some(woke);
        self
    }

    /// Marks a reading the sampler couldn't take.
    #[must_use]
    pub fn missing(mut self, reading: Reading) -> Self {
        self.missing.push(reading);
        self
    }
}

/// A job running when a sample is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Running<'a> {
    pub label: &'a str,
    /// What it uses; it loads the machine whether or not it is measured.
    pub uses: Amount,
    /// Whether its holder's figure reaches the sample. An unmeasured job counts in the machine's figures but
    /// isn't listed under `jobs`, so admission charges it its whole reservation.
    pub measured: bool,
    /// Its running threads in thousandths, summed into `maitred_running_threads` when it is measured. By default its
    /// CPU use: a job using 1100 millicores has about 1.1 threads running.
    pub threads_milli: u32,
}

impl<'a> Running<'a> {
    /// Sets its running threads, in whole threads.
    #[must_use]
    pub fn threads(mut self, threads: u32) -> Self {
        self.threads_milli = threads.saturating_mul(1_000);
        self
    }

    #[must_use]
    pub fn measured(label: &'a str, uses: Amount) -> Self {
        Self {
            label,
            uses,
            measured: true,
            threads_milli: uses.cpu_millicores,
        }
    }

    #[must_use]
    pub fn unmeasured(label: &'a str, uses: Amount) -> Self {
        Self {
            label,
            uses,
            measured: false,
            threads_milli: uses.cpu_millicores,
        }
    }
}

/// What `load/now.json` says after one sample, in the scenario's units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    pub sampled_at: Ms,
    pub interval: Ms,
    pub fresh: bool,
    pub woke_at: Option<Ms>,
    pub capacity: Amount,
    pub cpu_used_millicores: u32,
    /// Runnable work per core, in thousandths: 1000 is one runnable task per core. 0 when the sample names the
    /// reading missing; a reader checks `missing` first.
    pub cpu_pressure_permille: u32,
    pub mem_available_mb: u32,
    pub gpu_permille: Option<u32>,
    /// maitred's own running threads: the measured jobs' threads, summed (macOS's CPU kill reads it).
    pub maitred_running_threads: u32,
    pub quiet_streak: u32,
    pub over_cpu_streak: u32,
    pub over_gpu_streak: u32,
    /// Each running job's use, by label.
    pub jobs: Vec<(String, Amount)>,
    pub missing: Vec<Reading>,
}

/// Publishes one sample after another, keeping the streaks between them.
#[derive(Clone, Debug)]
pub struct Publisher {
    capacity: Amount,
    interval: Ms,
    quiet_threshold: u32,
    cpu_kill_ceiling: u32,
    gpu_kill_ceiling: u32,
    woke_at: Option<Ms>,
    /// When the previous sample was taken, where this sample's interval begins.
    previous: Option<Ms>,
    /// A gap since the previous sample longer than this restarts every streak.
    load_freshness: Ms,
    /// Whether the CPU-kill streak counts maitred's running threads per core while the CPU is saturated, as on
    /// macOS, rather than pressure (ARCHITECTURE.md, Kill policy step 1).
    threads_for_cpu_streak: bool,
    quiet_streak: u32,
    over_cpu_streak: u32,
    over_gpu_streak: u32,
}

impl Publisher {
    #[must_use]
    pub fn new(machine: &MachineConfig) -> Self {
        Self {
            capacity: machine.capacity,
            interval: machine.sample_interval,
            quiet_threshold: machine.quiet_threshold_permille,
            cpu_kill_ceiling: machine.cpu_kill_ceiling_permille,
            gpu_kill_ceiling: machine.gpu_kill_ceiling_permille,
            woke_at: None,
            previous: None,
            load_freshness: machine.load_freshness,
            threads_for_cpu_streak: false,
            quiet_streak: 0,
            over_cpu_streak: 0,
            over_gpu_streak: 0,
        }
    }

    /// Counts the CPU-kill streak as macOS does: maitred's own running threads per core, over the CPU-kill ceiling,
    /// while CPU use is at capacity.
    #[must_use]
    pub fn counting_threads_for_the_cpu_streak(mut self) -> Self {
        self.threads_for_cpu_streak = true;
        self
    }

    /// The load file after `sample`, with `running` jobs using what they declare.
    ///
    /// `victim_gone` is when the last kill's victim was gone, if there has been one: a kill streak counts
    /// only samples whose interval began after it (ARCHITECTURE.md, Kill policy step 1). The target running the
    /// scenario supplies it: the simulator from its own model, the real-process runner from when the victim's
    /// session emptied.
    ///
    /// A missing CPU-pressure reading counts toward neither the quiet streak nor the CPU-kill streak, and a
    /// missing GPU reading toward no GPU streak: with no figure there is no CPU limit, no CPU kill and no GPU
    /// kill (ARCHITECTURE.md, Critical readings).
    pub fn publish(
        &mut self,
        sample: &Sample,
        running: &[Running<'_>],
        victim_gone: Option<Ms>,
    ) -> Published {
        let job_cpu = running.iter().fold(0_u32, |sum, job| {
            sum.saturating_add(job.uses.cpu_millicores)
        });
        let job_mem: u32 = running.iter().map(|job| job.uses.mem_mb).sum();
        let demand = sample.cpu_demand_millicores.saturating_add(job_cpu);
        let pressure = permille(demand, self.capacity.cpu_millicores);
        let woke = sample.woke_at.is_some();
        if woke {
            self.woke_at = sample.woke_at;
        }
        let began = self.previous.unwrap_or(sample.at - self.interval);
        // A gap longer than load_freshness, whether the sampler restarted or stalled, is no evidence of anything, so
        // every streak starts again (ARCHITECTURE.md, Kill policy step 1).
        let gap = sample.at - began > self.load_freshness;
        self.previous = Some(sample.at);
        let counts_for_kill = !woke && !gap && victim_gone.is_none_or(|gone| began > gone);
        let pressure_read = !sample.missing.contains(&Reading::CpuPressure);
        let cpu_read = !sample.missing.contains(&Reading::Cpu);
        let capacity = u64::from(self.capacity.cpu_millicores);
        // Comparisons are exact: demand per core over a ceiling in thousandths, without rounding the quotient.
        let per_core_over =
            |amount: u32, ceiling: u32| u64::from(amount) * 1_000 > u64::from(ceiling) * capacity;
        let gpu_read = !sample.missing.contains(&Reading::Gpu);

        let quiet = u64::from(demand) * 1_000 < u64::from(self.quiet_threshold) * capacity;
        self.quiet_streak = if woke || gap || !pressure_read || !quiet {
            0
        } else {
            self.quiet_streak + 1
        };
        let measured_threads_milli = running
            .iter()
            .filter(|job| job.measured)
            .fold(0_u32, |sum, job| sum.saturating_add(job.threads_milli));
        let over_cpu = if self.threads_for_cpu_streak {
            // macOS: maitred's own running threads per core, counted only while CPU use is known and at capacity.
            cpu_read
                && demand >= self.capacity.cpu_millicores
                && u64::from(measured_threads_milli)
                    > u64::from(self.cpu_kill_ceiling) * capacity / 1_000
        } else {
            pressure_read && per_core_over(demand, self.cpu_kill_ceiling)
        };
        self.over_cpu_streak = if counts_for_kill && over_cpu {
            self.over_cpu_streak + 1
        } else {
            0
        };
        self.over_gpu_streak = match sample.gpu_permille {
            Some(gpu) if counts_for_kill && gpu_read && gpu > self.gpu_kill_ceiling => {
                self.over_gpu_streak + 1
            }
            _ => 0,
        };

        Published {
            sampled_at: sample.at,
            interval: sample.at - began,
            fresh: !woke,
            woke_at: self.woke_at,
            capacity: self.capacity,
            cpu_used_millicores: demand.min(self.capacity.cpu_millicores),
            cpu_pressure_permille: if pressure_read { pressure } else { 0 },
            mem_available_mb: self
                .capacity
                .mem_mb
                .saturating_sub(sample.mem_used_mb + job_mem),
            gpu_permille: sample.gpu_permille,
            maitred_running_threads: measured_threads_milli.saturating_add(500) / 1_000,
            quiet_streak: self.quiet_streak,
            over_cpu_streak: self.over_cpu_streak,
            over_gpu_streak: self.over_gpu_streak,
            jobs: running
                .iter()
                .filter(|job| job.measured)
                .map(|job| (job.label.to_string(), job.uses))
                .collect(),
            missing: sample.missing.clone(),
        }
    }
}

fn permille(part: u32, whole: u32) -> u32 {
    if whole == 0 {
        0
    } else {
        u32::try_from(u64::from(part) * 1_000 / u64::from(whole)).unwrap_or(u32::MAX)
    }
}
