use std::collections::{BTreeMap, BTreeSet};

use crate::{BootTime, Config, Pool, Position, Resources, Ticket, TicketId};

/// Everything one pass knows, built by the store under `admit.lock` (ARCHITECTURE.md, One pass, step
/// 3). `decide` and `choose_victim` read nothing else, so every waiter computes the same answer from
/// the same snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub now: BootTime,
    pub config: Config,
    /// `load/now.json`, or `None` when it couldn't be read.
    pub load: Option<Load>,
    pub tickets: Vec<Ticket>,
    /// Locked tickets with no readable slot: alive, but their payload can't be read, so each is
    /// charged the heavy minimum.
    pub unreadable: u32,
    /// The last heavy admission: the latest of `load/admissions.jsonl` and the `admitted_at` of
    /// living heavy tickets.
    pub last_heavy: Option<BootTime>,
    /// Tickets an administrator moved (`maitred queue move`), from the machine root's `order` file:
    /// each sorts at its position instead of `(seq, 0)`. Ids with no ticket are ignored.
    pub moved: BTreeMap<TicketId, Position>,
    /// Tickets whose stop marker exists and is locked: a kill is under way. A marker whose lock is
    /// free belongs to a dead holder and isn't here.
    pub live_stop_markers: BTreeSet<TicketId>,
    /// Slot numbers whose lock the snapshot's builder couldn't take though no ticket names them: a
    /// leftover daemon holds them, or the sampler hasn't made their file yet. Taken, and no drain
    /// frees them.
    pub held_outside: BTreeMap<Pool, BTreeSet<u32>>,
    /// The victim the first holder named for the breach under way, if any: later holders keep it
    /// while it is still a candidate, so holders reading different samples can't name each other.
    pub named_victim: Option<TicketId>,
    /// When this supervisor last found a gap between its own passes longer than the stall limit:
    /// the machine slept, or it was stopped. Until a stall limit has passed since, it judges nobody
    /// stalled, since the others may not have passed since waking either. Unlike `woke_at`, it
    /// needs no sampler.
    pub my_wake: Option<BootTime>,
}

impl Snapshot {
    /// Whether any ticket was recreated within the stall limit: other deleted files may not have
    /// come back yet, so nobody is admitted or chosen. A holder recreating its stop marker sets its
    /// ticket's `recreated_at`, so a marker's recreation shows here too.
    #[must_use]
    pub fn recreated_recently(&self) -> bool {
        self.tickets
            .iter()
            .filter(|t| t.is_this_boot())
            .filter_map(|t| t.recreated_at)
            .any(|at| self.now.since(at) < self.config.stall_limit_ms())
    }

    /// The load, if it is fresh (not the first sample after a sleep), recent enough, and has its
    /// critical readings, CPU use and available memory (ARCHITECTURE, Critical readings).
    pub(crate) fn reliable_load(&self) -> Option<Reliable> {
        let load = self.load.as_ref()?;
        if !load.fresh || self.now.since(load.sampled_at) > self.config.load_freshness_ms() {
            return None;
        }
        Some(Reliable {
            capacity: load.capacity,
            cpu_used_millicores: load.cpu_used_millicores?,
            cpu_pressure: load.cpu_pressure,
            mem_available_mb: load.mem_available_mb?,
            gpu_utilization: load.gpu_utilization,
            quiet_streak: load.quiet_streak,
            over_cpu_streak: load.over_cpu_streak,
            over_gpu_streak: load.over_gpu_streak,
        })
    }
}

/// The load figures `decide` and the kill policy need, once they can be relied on.
pub(crate) struct Reliable {
    pub(crate) capacity: Resources,
    pub(crate) cpu_used_millicores: u64,
    /// Not a critical reading: missing, it turns off only the CPU live limit.
    pub(crate) cpu_pressure: Option<f64>,
    pub(crate) mem_available_mb: u64,
    pub(crate) gpu_utilization: Option<f64>,
    pub(crate) quiet_streak: u32,
    pub(crate) over_cpu_streak: u32,
    pub(crate) over_gpu_streak: u32,
}

/// The sampler's latest sample, as `load/now.json` holds it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Load {
    pub sampled_at: BootTime,
    /// False for the first sample after a sleep.
    pub fresh: bool,
    pub woke_at: Option<BootTime>,
    pub capacity: Resources,
    /// `None` when the reading is missing.
    pub cpu_used_millicores: Option<u64>,
    pub cpu_pressure: Option<f64>,
    pub mem_available_mb: Option<u64>,
    /// GPU utilisation, 0 to 1; `None` on a machine with no readable GPU.
    pub gpu_utilization: Option<f64>,
    pub quiet_streak: u32,
    /// Consecutive samples over `cpu_kill_ceiling`, counted only from after the last victim was gone.
    pub over_cpu_streak: u32,
    /// Consecutive samples over `gpu_kill_ceiling`, counted the same way.
    pub over_gpu_streak: u32,
    /// Each running job's use, as the sampler copied it from the job's holder. Only measurements
    /// whose window overlaps the sample's own interval are copied.
    pub jobs: BTreeMap<TicketId, Resources>,
}
