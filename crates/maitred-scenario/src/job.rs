//! A job a worker asks maitred to run, and how it behaves once running.

use crate::machine::Amount;
use crate::time::{Ms, secs};

/// How a job is treated (DESIGN.md: Terms). `none` isn't here: maitred never queues it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Light,
    Heavy,
    Quiet,
}

/// One job. Every job has a label, which is how a scenario names it: ticket ids are random on each run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub label: String,
    pub class: Class,
    /// The reservation its rule gives it; `None` means its class default.
    pub reservation: Option<Amount>,
    /// The pools it asks for one slot of each.
    pub slots: Vec<String>,
    /// What it actually uses while it runs. The simulator reports it as measured; the real target's
    /// `burn` uses it.
    pub uses: Amount,
    /// Every `period` of scenario time, from 0, it switches between `uses` and this, starting with `uses`; `None`
    /// means it uses `uses` all run. Two jobs given the same pair in opposite order are in opposite phase.
    pub alternate: Option<(Ms, Amount)>,
    /// How long it runs once started, if nothing stops it.
    pub runs_for: Ms,
    /// The status it exits with when it finishes on its own.
    pub exit_status: u8,
    /// Whether its holder's measurements reach the sample. A real job lacks a figure only before its holder's first
    /// pass (see [`crate::Running`]); `false` holds that state for the whole run.
    pub measured: bool,
    /// Its running threads, as its holder counts them into `maitred_running_threads` on macOS; `None` means its
    /// CPU use, as threads in thousandths.
    pub running_threads: Option<u32>,
}

impl Job {
    /// A job of `class` that uses nothing much and runs for a second.
    #[must_use]
    pub fn new(label: &str, class: Class) -> Self {
        Self {
            label: label.into(),
            class,
            reservation: None,
            slots: Vec::new(),
            uses: Amount {
                cpu_millicores: 100,
                mem_mb: 64,
            },
            alternate: None,
            runs_for: secs(1),
            exit_status: 0,
            measured: true,
            running_threads: None,
        }
    }

    /// Asks for one slot from `pool`.
    #[must_use]
    pub fn slot(mut self, pool: &str) -> Self {
        self.slots.push(pool.into());
        self
    }

    /// Sets the reservation its rule gives it.
    #[must_use]
    pub fn reserves(mut self, cpu_millicores: u32, mem_mb: u32) -> Self {
        self.reservation = Some(Amount {
            cpu_millicores,
            mem_mb,
        });
        self
    }

    /// Sets what it actually uses while it runs.
    #[must_use]
    pub fn uses(mut self, cpu_millicores: u32, mem_mb: u32) -> Self {
        self.uses = Amount {
            cpu_millicores,
            mem_mb,
        };
        self
    }

    /// Makes it switch to using `cpu_millicores` and `mem_mb` every other `period` of scenario time, so which of two
    /// near-tied jobs is further over its reservation flips from sample to sample.
    #[must_use]
    pub fn alternates_with(mut self, period: Ms, cpu_millicores: u32, mem_mb: u32) -> Self {
        self.alternate = Some((
            period,
            Amount {
                cpu_millicores,
                mem_mb,
            },
        ));
        self
    }

    /// What it uses at scenario time `at`, for the target to pass the publisher as a [`crate::Running`].
    #[must_use]
    pub fn uses_at(&self, at: Ms) -> Amount {
        match self.alternate {
            Some((period, other)) if period.0 > 0 && (at.0 / period.0) % 2 == 1 => other,
            _ => self.uses,
        }
    }

    /// Sets how long it runs once started.
    #[must_use]
    pub fn runs_for(mut self, span: Ms) -> Self {
        self.runs_for = span;
        self
    }

    /// Keeps its figure out of every sample, as before its holder's first pass: it still loads the machine.
    #[must_use]
    pub fn unmeasured(mut self) -> Self {
        self.measured = false;
        self
    }

    /// Sets its running threads.
    #[must_use]
    pub fn with_threads(mut self, threads: u32) -> Self {
        self.running_threads = Some(threads);
        self
    }

    /// Its running threads in thousandths: as set, or its CPU use (a job using 1100 millicores has about 1.1 threads
    /// running).
    #[must_use]
    pub fn threads_milli(&self) -> u32 {
        self.running_threads
            .map_or(self.uses.cpu_millicores, |threads| {
                threads.saturating_mul(1_000)
            })
    }

    /// Sets the status it exits with.
    #[must_use]
    pub fn exits_with(mut self, status: u8) -> Self {
        self.exit_status = status;
        self
    }
}
