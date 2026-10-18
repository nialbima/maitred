//! The load a scenario puts on the machine, and the load file both targets publish from it.
//!
//! A [`Sample`] is the trace's background load at one moment: work no maitred job accounts for. The
//! [`Publisher`] adds each running job's declared use and keeps the streaks the sampler would keep, giving the
//! [`Published`] figures that `load/now.json` carries (ARCHITECTURE.md: `load/now.json`). The simulator and
//! the test kit's `replay` both call it, so the two targets read identical load.

use crate::machine::{Amount, MachineConfig, Platform};
use crate::time::Ms;

/// A critical or optional reading the sampler couldn't take (ARCHITECTURE.md: Critical readings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reading {
    Cpu,
    Memory,
    /// `cpu_stall`: Linux's CPU limit and CPU kill figure.
    CpuStall,
    /// `cpu_runnable_per_core`: macOS's CPU limit figure.
    CpuRunnable,
    Gpu,
}

/// The background load at the end of one sample interval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    /// When the sample was taken; its interval runs from the previous sample (or one sample interval before, for
    /// the first).
    pub at: Ms,
    /// CPU demand from outside maitred, in millicores. Demand past capacity shows as runnable work per core.
    pub cpu_demand_millicores: u32,
    /// The fraction of the interval in which some task waited for a CPU, in thousandths (Linux's `cpu_stall`).
    /// Stated, not derived: the doc gives no formula from demand to stall, so jobs add none, and a scenario that needs
    /// its jobs to cause stall says so here.
    pub stall_permille: u32,
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
            stall_permille: 0,
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

    /// Sets the CPU stall, in thousandths of the interval; capped at the whole interval.
    #[must_use]
    pub fn stall(mut self, permille: u32) -> Self {
        self.stall_permille = permille.min(1_000);
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

/// A holder's pass: when it started and the use it measured, both written to `tickets/<id>.use`
/// (ARCHITECTURE.md: tickets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HolderPass {
    pub at: Ms,
    pub measured: Amount,
}

/// A job running when a sample is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Running<'a> {
    pub label: &'a str,
    /// What it uses at the sample's moment, which is what the machine's figures see.
    pub uses: Amount,
    /// Its running threads in thousandths, summed into `maitred_running_threads` once its holder has passed. By
    /// default its CPU use: a job using 1100 millicores has about 1.1 threads running.
    pub threads_milli: u32,
    /// Its holder's latest pass before the sample; `None` before its first. The sample copies what that pass
    /// measured into `jobs` however old it is, until the holder writes the next (Nick, MT-046), and its start into
    /// `holder_passes`, so the figure's age is `sampled_at` less that start. A job with no pass yet counts in the
    /// machine's figures but isn't listed under `jobs`, so admission charges it its whole reservation.
    pub last_pass: Option<HolderPass>,
}

impl<'a> Running<'a> {
    /// A job whose holder hasn't passed yet.
    #[must_use]
    pub fn new(label: &'a str, uses: Amount) -> Self {
        Self {
            label,
            uses,
            threads_milli: uses.cpu_millicores,
            last_pass: None,
        }
    }

    /// Sets its running threads, in whole threads.
    #[must_use]
    pub fn threads(mut self, threads: u32) -> Self {
        self.threads_milli = threads.saturating_mul(1_000);
        self
    }

    /// Sets its holder's latest pass: when it started, and the use it measured then.
    #[must_use]
    pub fn holder_passed(mut self, at: Ms, measured: Amount) -> Self {
        self.last_pass = Some(HolderPass { at, measured });
        self
    }
}

/// What `load/now.json` says after one sample, in the scenario's units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    /// Which platform's figures the file carries ("Absent or null" in the doc).
    pub platform: Platform,
    pub sampled_at: Ms,
    pub interval: Ms,
    pub fresh: bool,
    pub woke_at: Option<Ms>,
    pub capacity: Amount,
    pub cpu_used_millicores: u32,
    /// `cpu_stall` in thousandths, on Linux; `None` on macOS, which has none. 0 when the sample names it missing; a
    /// reader checks `missing` first.
    pub cpu_stall_permille: Option<u32>,
    /// `cpu_runnable_per_core` in thousandths: 1000 is one runnable task per core. 0 when the sample names it
    /// missing.
    pub cpu_runnable_per_core_permille: u32,
    pub mem_available_mb: u32,
    pub gpu_permille: Option<u32>,
    /// maitred's own running threads: the measured jobs' threads, summed (macOS's CPU kill reads it).
    pub maitred_running_threads: u32,
    pub quiet_streak: u32,
    pub over_cpu_streak: u32,
    pub over_gpu_streak: u32,
    /// Each running job's use as its holder last measured it, by label, for every job whose holder has passed.
    pub jobs: Vec<(String, Amount)>,
    /// When each running job's holder last started a pass, by label, for every job whose holder has passed.
    pub holder_passes: Vec<(String, Ms)>,
    pub missing: Vec<Reading>,
}

/// Publishes one sample after another, keeping the streaks between them.
#[derive(Clone, Debug)]
pub struct Publisher {
    platform: Platform,
    capacity: Amount,
    interval: Ms,
    quiet_threshold: u32,
    cpu_stall_kill_ceiling: u32,
    cpu_threads_kill_ceiling: u32,
    gpu_kill_ceiling: u32,
    woke_at: Option<Ms>,
    /// When the previous sample was taken, where this sample's interval begins.
    previous: Option<Ms>,
    /// A gap since the previous sample longer than this restarts every streak.
    load_freshness: Ms,
    quiet_streak: u32,
    over_cpu_streak: u32,
    over_gpu_streak: u32,
}

impl Publisher {
    #[must_use]
    pub fn new(machine: &MachineConfig) -> Self {
        Self {
            platform: machine.platform,
            capacity: machine.capacity,
            interval: machine.sample_interval,
            quiet_threshold: machine.quiet_threshold_permille,
            cpu_stall_kill_ceiling: machine.cpu_stall_kill_ceiling_permille,
            cpu_threads_kill_ceiling: machine.cpu_threads_kill_ceiling_permille,
            gpu_kill_ceiling: machine.gpu_kill_ceiling_permille,
            woke_at: None,
            previous: None,
            load_freshness: machine.load_freshness,
            quiet_streak: 0,
            over_cpu_streak: 0,
            over_gpu_streak: 0,
        }
    }

    /// The load file after `sample`, with `running` jobs using what they declare.
    ///
    /// `victim_gone` is when the last kill's victim was gone, if there has been one: a kill streak counts
    /// only samples whose interval began after it (ARCHITECTURE.md, Kill policy step 1). The target running the
    /// scenario supplies it: the simulator from its own model, the real-process runner from when the victim's
    /// session emptied.
    ///
    /// The quiet streak counts CPU use under the quiet threshold of capacity, on every platform, and a missing CPU
    /// reading counts toward no quiet streak (ARCHITECTURE rev 190). A missing stall reading counts toward no CPU-kill
    /// streak on Linux, and a missing CPU reading on macOS; a missing GPU reading toward no GPU streak
    /// (ARCHITECTURE.md, Critical readings).
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
        let runnable = permille(demand, self.capacity.cpu_millicores);
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
        let stall_read = !sample.missing.contains(&Reading::CpuStall);
        let runnable_read = !sample.missing.contains(&Reading::CpuRunnable);
        let cpu_read = !sample.missing.contains(&Reading::Cpu);
        let capacity = u64::from(self.capacity.cpu_millicores);
        // Comparisons are exact: demand per core over a ceiling in thousandths, without rounding the quotient.
        let per_core_over =
            |amount: u32, ceiling: u32| u64::from(amount) * 1_000 > u64::from(ceiling) * capacity;
        let gpu_read = !sample.missing.contains(&Reading::Gpu);

        // cpu_used_millicores over capacity, as the doc defines the quiet streak.
        let cpu_used = demand.min(self.capacity.cpu_millicores);
        let quiet = u64::from(cpu_used) * 1_000 < u64::from(self.quiet_threshold) * capacity;
        self.quiet_streak = if woke || gap || !cpu_read || !quiet {
            0
        } else {
            self.quiet_streak + 1
        };
        let measured_threads_milli = running
            .iter()
            .filter(|job| job.last_pass.is_some())
            .fold(0_u32, |sum, job| sum.saturating_add(job.threads_milli));
        let over_cpu = match self.platform {
            // macOS: maitred's own running threads per core, counted only while CPU use is known and at capacity.
            Platform::MacOs => {
                cpu_read
                    && demand >= self.capacity.cpu_millicores
                    && per_core_over(measured_threads_milli, self.cpu_threads_kill_ceiling)
            }
            Platform::Linux => stall_read && sample.stall_permille > self.cpu_stall_kill_ceiling,
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
            platform: self.platform,
            sampled_at: sample.at,
            interval: sample.at - began,
            fresh: !woke,
            woke_at: self.woke_at,
            capacity: self.capacity,
            cpu_used_millicores: cpu_used,
            cpu_stall_permille: (self.platform == Platform::Linux).then_some(if stall_read {
                sample.stall_permille
            } else {
                0
            }),
            cpu_runnable_per_core_permille: if runnable_read { runnable } else { 0 },
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
                .filter_map(|job| Some((job.label.to_string(), job.last_pass?.measured)))
                .collect(),
            holder_passes: running
                .iter()
                .filter_map(|job| Some((job.label.to_string(), job.last_pass?.at)))
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
