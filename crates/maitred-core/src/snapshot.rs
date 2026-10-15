use std::collections::BTreeMap;

use crate::{Config, Resources, Ticket, TicketId, Wall};

/// Everything one pass knows, built by the store under `admit.lock` (ARCHITECTURE.md, One pass, step
/// 3). `decide` and `choose_victim` read nothing else, so every waiter computes the same answer from
/// the same snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub now: Wall,
    pub config: Config,
    /// `load/now.json`, or `None` when it couldn't be read.
    pub load: Option<Load>,
    pub tickets: Vec<Ticket>,
    /// Locked tickets with no readable slot: alive, but their payload can't be read, so each is
    /// charged the heavy minimum.
    pub unreadable: u32,
    /// The last heavy admission: the latest of `load/admissions.jsonl` and the `admitted_at` of
    /// living heavy tickets.
    pub last_heavy: Option<Wall>,
}

/// The sampler's latest sample, as `load/now.json` holds it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Load {
    pub sampled_at: Wall,
    /// False for the first sample after a sleep.
    pub fresh: bool,
    pub woke_at: Option<Wall>,
    pub capacity: Resources,
    /// `None` when the reading is missing.
    pub cpu_used_millicores: Option<u64>,
    pub cpu_pressure: Option<f64>,
    pub mem_available_mb: Option<u64>,
    /// GPU utilisation, 0 to 1; `None` on a machine with no readable GPU.
    pub gpu_utilization: Option<f64>,
    pub quiet_streak: u32,
    /// Each running job's use, as the sampler copied it from the job's holder. Only measurements
    /// whose window overlaps the sample's own interval are copied.
    pub jobs: BTreeMap<TicketId, Resources>,
}
