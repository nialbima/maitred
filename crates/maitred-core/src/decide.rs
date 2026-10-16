//! Admission: `decide(snapshot, me)`, ARCHITECTURE.md's "Inside `decide`" line by line.

use std::collections::{BTreeMap, BTreeSet};

use crate::snapshot::Reliable;
use crate::{
    BootTime, Class, Config, Liveness, Pool, PoolConfig, Position, Resources, Snapshot, State,
    Ticket, TicketId,
};

#[cfg(test)]
mod props;
#[cfg(test)]
mod tests;

/// What a waiting supervisor does after one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Admit(Admission),
    Wait(Reason),
}

/// How a ticket got in, for its record. The slot numbers aren't chosen here: the admitter takes the
/// lowest number no ticket names whose lock it can get (ARCHITECTURE.md, Slot pools), and only a lock
/// attempt can tell a free slot from one held outside any ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "three independent facts a record reports"
)]
pub struct Admission {
    /// Admitted over the budget because nothing living was charged.
    pub first_in: bool,
    /// A quiet job admitted past `quiet_max_wait` without the quiet streak.
    pub tainted: bool,
    /// Admitted one at a time because there was no reliable load.
    pub fallback: bool,
}

/// The first rule that held a ticket back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The ticket isn't a living waiting ticket in the snapshot.
    NotWaiting,
    /// A ticket was recreated within the stall limit; others may not have come back yet.
    TicketsRecreated,
    /// The load file is missing, stale or lacks a critical reading, and this ticket isn't first in
    /// line on an empty machine.
    NoReliableLoad,
    LiveLimit(Limit),
    /// A living quiet job is charged.
    QuietJobRunning,
    /// A heavy ticket waits out the time since the last heavy admission.
    Settle,
    /// `max_running_heavy` heavy jobs are charged or admitted on paper.
    MaxRunning,
    /// A quiet ticket waits for an idle machine and the quiet streak.
    QuietNeedsIdle,
    /// Not enough free cores or memory.
    Budget,
    /// Every slot of this pool is named by a ticket or taken on paper.
    NoSlot(Pool),
    /// The walk stopped at a due quiet ticket or one whose hold lapsed, so the machine drains for it.
    DrainingFor(TicketId),
    /// The walk stopped at a ticket admitted first in.
    BehindFirstIn(TicketId),
    /// A quiet ticket ahead was admitted in this walk, and nothing starts beside a quiet job.
    BehindQuiet(TicketId),
    /// The ticket's own last pass is older than the stall limit, so the walk skips it.
    Stalled,
}

/// A live limit the machine is past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Memory,
    Cpu,
    Gpu,
}

/// Decides whether the ticket `me` may start now. Pure: the answer depends only on the snapshot.
#[must_use]
pub fn decide(snap: &Snapshot, me: &TicketId) -> Decision {
    let config = &snap.config;
    let Some(mine) = snap.tickets.iter().find(|t| &t.id == me) else {
        return Decision::Wait(Reason::NotWaiting);
    };
    if mine.state != State::Waiting || !mine.is_living() {
        return Decision::Wait(Reason::NotWaiting);
    }

    if snap.recreated_recently() {
        return Decision::Wait(Reason::TicketsRecreated);
    }

    let charged: Vec<&Ticket> = snap.tickets.iter().filter(|t| is_charged(t)).collect();

    let Some(load) = snap.reliable_load() else {
        return fallback(snap, mine, me, &charged);
    };

    if load.mem_available_mb < config.mem_floor_mb {
        return Decision::Wait(Reason::LiveLimit(Limit::Memory));
    }
    if load
        .cpu_pressure
        .is_some_and(|pressure| pressure > config.cpu_ceiling)
    {
        return Decision::Wait(Reason::LiveLimit(Limit::Cpu));
    }
    if load
        .gpu_utilization
        .is_some_and(|gpu| gpu > config.gpu_ceiling)
    {
        return Decision::Wait(Reason::LiveLimit(Limit::Gpu));
    }
    if charged
        .iter()
        .any(|t| t.is_living() && t.class == Class::Quiet)
    {
        return Decision::Wait(Reason::QuietJobRunning);
    }

    let mut walk = Walk::new(snap, &load, &charged);
    for t in waiting(snap, snap.config.stall_limit_ms()) {
        let is_me = &t.id == me;
        let allowed = walk.allowed(t);
        let held = match allowed.clone().and_then(|()| walk.fits(t)) {
            Ok(()) => {
                if is_me {
                    let tainted = walk.tainted(t);
                    return Decision::Admit(Admission {
                        tainted,
                        ..Admission::default()
                    });
                }
                // Nothing starts beside a quiet job, including one admitted earlier in this walk.
                if t.class == Class::Quiet {
                    return Decision::Wait(Reason::BehindQuiet(t.id.clone()));
                }
                walk.admit_on_paper(t);
                continue;
            }
            Err(reason) => reason,
        };
        // First in: nothing living is charged and nothing was admitted earlier in this walk. It
        // waives cores only: never memory, whose over-commit takes a machine down, a class rule or
        // a slot.
        if walk.first_in_open()
            && walk.memory_fits(t)
            && allowed.is_ok()
            && slots_free(snap, t, &walk.slots_on_paper).is_ok()
        {
            return if is_me {
                Decision::Admit(Admission {
                    first_in: true,
                    tainted: walk.tainted(t),
                    ..Admission::default()
                })
            } else {
                Decision::Wait(Reason::BehindFirstIn(t.id.clone()))
            };
        }
        if is_me {
            return Decision::Wait(held);
        }
        // Draining: nothing new starts, so once nothing living is charged it gets in first.
        // Only a drain that could let it in: otherwise the walk would stop at it for ever. A lapsed
        // ticket drains only when its class allows it, since draining frees nothing a class rule
        // (settle, max_running) is waiting on; a quiet ticket's class rule is the idle machine
        // the drain makes.
        if (walk.due_quiet(t) || (walk.lapsed(t) && allowed.is_ok())) && walk.fits_once_drained(t) {
            return Decision::Wait(Reason::DrainingFor(t.id.clone()));
        }
        // An aged ticket that only the budget keeps out holds its place in it; one waiting for a
        // slot, kept out by its class, or too big to fit even once drained would hold budget it
        // can't use.
        if walk.aged(t) && held == Reason::Budget && walk.fits_once_drained(t) {
            walk.hold(t);
        }
    }
    // `me` is living and waiting but the walk skipped it: its last pass is past the stall limit.
    Decision::Wait(Reason::Stalled)
}

/// With no reliable load: one job at a time, blind. A quiet ticket keeps its own rule: no load
/// figures (most often the burst after a wake) are no time for a benchmark, so it waits until it is
/// due, then gets in tainted; until then it doesn't hold the line for anyone else.
fn fallback(snap: &Snapshot, mine: &Ticket, me: &TicketId, charged: &[&Ticket]) -> Decision {
    if mine.class == Class::Quiet && !due_quiet(&snap.config, mine) {
        return Decision::Wait(Reason::QuietNeedsIdle);
    }
    // As for first in, a charged orphan or unreadable ticket doesn't count: one that never ends
    // must not hold the queue.
    let nothing_charged = !charged.iter().any(|t| t.is_living());
    if let Err(reason) = slots_free(snap, mine, &BTreeMap::new()) {
        return Decision::Wait(reason);
    }
    // First in line among those the fallback could admit: a quiet ticket not yet due, or one whose
    // slot isn't free, doesn't hold the line for anyone else.
    // A waiter silent for three stall limits is stalled even here (MT-039): after a wake every
    // live supervisor passes within a poll interval, well inside it.
    let first = waiting(snap, snap.config.stall_limit_ms().saturating_mul(3))
        .into_iter()
        .filter(|t| t.class != Class::Quiet || due_quiet(&snap.config, t))
        .find(|t| slots_free(snap, t, &BTreeMap::new()).is_ok())
        .map(|t| &t.id);
    if nothing_charged && first == Some(me) {
        Decision::Admit(Admission {
            fallback: true,
            tainted: mine.class == Class::Quiet,
            ..Admission::default()
        })
    } else {
        Decision::Wait(Reason::NoReliableLoad)
    }
}

/// A quiet ticket past `quiet_max_wait`: the streak isn't needed any more, and the walk stops at it
/// until it gets in.
fn due_quiet(config: &Config, t: &Ticket) -> bool {
    t.class == Class::Quiet && t.waited_ms > config.quiet_max_wait_ms
}

/// Charged: living tickets admitted or running, and running orphans of this boot whose holder is
/// alive. (Unreadable tickets are charged too; the snapshot only counts them.)
fn is_charged(t: &Ticket) -> bool {
    match t.liveness {
        Liveness::Living => matches!(t.state, State::Admitted | State::Running),
        Liveness::Orphaned { this_boot } => {
            t.holder_alive && this_boot && t.state == State::Running
        }
    }
}

/// Living waiting tickets by `(position, id)`, without those whose last pass is older than
/// `limit`.
fn waiting(snap: &Snapshot, limit: u64) -> Vec<&Ticket> {
    // Nobody is judged stalled until the stall limit has passed since the machine last woke.
    let woke_recently = snap
        .load
        .as_ref()
        .and_then(|l| l.woke_at)
        .is_some_and(|w| snap.now.since(w) < limit);
    let mut tickets: Vec<&Ticket> = snap
        .tickets
        .iter()
        .filter(|t| t.is_living() && t.state == State::Waiting)
        .filter(|t| woke_recently || snap.now.since(t.last_pass) <= limit)
        .collect();
    // By position, then id: `(seq, 0)` unless an administrator moved the ticket.
    let position = |t: &Ticket| {
        snap.moved
            .get(&t.id)
            .copied()
            .unwrap_or(Position { seq: t.seq, sub: 0 })
    };
    tickets.sort_by(|a, b| (position(a), &a.id).cmp(&(position(b), &b.id)));
    tickets
}

/// Whether each pool `t` asks of has a slot no ticket names and nobody earlier in this walk took.
fn slots_free(snap: &Snapshot, t: &Ticket, on_paper: &BTreeMap<Pool, u32>) -> Result<(), Reason> {
    for pool in &t.pools {
        // An undeclared pool has no slots; a number outside the range is retired and uses none.
        let Some(cfg) = snap.config.pools.get(pool) else {
            return Err(Reason::NoSlot(pool.clone()));
        };
        let taken = taken_in_range(snap, pool, *cfg, |_| true)
            + u64::from(on_paper.get(pool).copied().unwrap_or(0));
        if taken >= u64::from(cfg.size) {
            return Err(Reason::NoSlot(pool.clone()));
        }
    }
    Ok(())
}

/// How many of a pool's current numbers are taken: named by a ticket past waiting that `holder`
/// selects (a waiting ticket holds no slot, whatever stale number it still names), or held outside
/// any ticket. A number outside the range is retired and uses none of it.
fn taken_in_range(
    snap: &Snapshot,
    pool: &Pool,
    cfg: PoolConfig,
    holder: impl Fn(&Ticket) -> bool,
) -> u64 {
    // Distinct numbers: a number is taken or it isn't, however many places list it.
    let named = snap
        .tickets
        .iter()
        .filter(|o| o.state != State::Waiting && holder(o))
        .filter_map(|o| o.slots.get(pool).copied());
    let outside = snap.held_outside.get(pool).into_iter().flatten().copied();
    let taken: BTreeSet<u32> = named.chain(outside).filter(|&n| cfg.contains(n)).collect();
    u64::try_from(taken.len()).unwrap_or(u64::MAX)
}

/// The budget and the counts as the walk changes them on paper.
struct Walk<'a> {
    snap: &'a Snapshot,
    capacity: Resources,
    quiet_streak: u32,
    /// Free cores and memory; can go below zero once an aged ticket holds its place.
    free_cpu: i128,
    free_mem: i128,
    /// Free memory once every living job has ended: what is free now plus what each holds (its
    /// measured use, or its whole reservation when unmeasured). A drain frees no more than this.
    drained_mem: i128,
    heavy: u32,
    last_heavy: Option<BootTime>,
    living_charged: bool,
    admitted_on_paper: u32,
    slots_on_paper: BTreeMap<Pool, u32>,
}

impl<'a> Walk<'a> {
    fn new(snap: &'a Snapshot, load: &Reliable, charged: &[&Ticket]) -> Self {
        let mut walk = Walk {
            snap,
            capacity: load.capacity,
            quiet_streak: load.quiet_streak,
            free_cpu: i128::from(load.capacity.cpu_millicores)
                - i128::from(load.cpu_used_millicores),
            free_mem: i128::from(load.mem_available_mb),
            drained_mem: i128::from(load.mem_available_mb),
            heavy: 0,
            last_heavy: snap.last_heavy,
            living_charged: charged.iter().any(|t| t.is_living()),
            admitted_on_paper: 0,
            slots_on_paper: BTreeMap::new(),
        };
        // An unreadable ticket is treated like an orphan (MT-029): charged the heavy minimum, but it
        // blocks neither first-in nor quiet's idle check, and no drain counts on freeing it, since
        // it may be a waiter. If it is really running, the sample shows its load.
        let unreadable = walk.reserve_class(Class::Heavy, Resources::default());
        for _ in 0..snap.unreadable {
            walk.take(unreadable);
        }
        for t in charged {
            // A living job's use from the sample, if measured. Orphans count as using nothing, so
            // they hold their whole reservation.
            let measured = match t.liveness {
                Liveness::Living => snap.load.as_ref().and_then(|l| l.jobs.get(&t.id)).copied(),
                Liveness::Orphaned { .. } => None,
            };
            walk.take(walk.reserve(t).saturating_sub(measured.unwrap_or_default()));
            if t.is_living() {
                walk.drained_mem += i128::from(measured.unwrap_or_else(|| walk.reserve(t)).mem_mb);
            }
            // An orphan's class barrier doesn't apply, so it doesn't count toward max_running.
            if t.is_living() && t.class == Class::Heavy {
                walk.heavy += 1;
            }
        }
        walk
    }

    fn reserve_class(&self, class: Class, asked: Resources) -> Resources {
        self.snap.config.reserve(class, asked, self.capacity)
    }

    fn reserve(&self, t: &Ticket) -> Resources {
        self.reserve_class(t.class, t.reservation)
    }

    fn take(&mut self, r: Resources) {
        self.free_cpu -= i128::from(r.cpu_millicores);
        self.free_mem -= i128::from(r.mem_mb);
    }

    /// The class rules: light always; heavy under `max_running_heavy` and past settle; quiet on an
    /// idle machine with the streak, or past `quiet_max_wait` without it.
    fn allowed(&self, t: &Ticket) -> Result<(), Reason> {
        let config = &self.snap.config;
        match t.class {
            Class::Light => Ok(()),
            Class::Heavy => {
                if self.heavy >= config.max_running_heavy {
                    Err(Reason::MaxRunning)
                } else if self
                    .last_heavy
                    .is_some_and(|at| self.snap.now.since(at) < config.settle_ms)
                {
                    Err(Reason::Settle)
                } else {
                    Ok(())
                }
            }
            Class::Quiet => {
                let idle = !self.living_charged && self.admitted_on_paper == 0;
                let streak = self.quiet_streak >= config.quiet_streak_needed;
                if idle && (streak || self.due_quiet(t)) {
                    Ok(())
                } else {
                    Err(Reason::QuietNeedsIdle)
                }
            }
        }
    }

    /// Slots first: a ticket short of a slot reports `NoSlot` even when the budget is short too,
    /// so an aged ticket holds budget only when budget is all it lacks.
    fn fits(&self, t: &Ticket) -> Result<(), Reason> {
        slots_free(self.snap, t, &self.slots_on_paper)?;
        let r = self.reserve(t);
        if i128::from(r.cpu_millicores) > self.free_cpu || i128::from(r.mem_mb) > self.free_mem {
            return Err(Reason::Budget);
        }
        Ok(())
    }

    /// Whether a drain could let `t` in: its memory fits in `drained_mem`, and every pool it asks
    /// has a slot that ending every living job would free (a slot an orphan or leftover names stays
    /// taken, and an undeclared or empty pool has none).
    fn fits_once_drained(&self, t: &Ticket) -> bool {
        let slots = t.pools.iter().all(|pool| {
            self.snap.config.pools.get(pool).is_some_and(|cfg| {
                // A drain ends living jobs only: what non-living tickets name, or what is held
                // outside any ticket, stays taken.
                taken_in_range(self.snap, pool, *cfg, |o| !o.is_living()) < u64::from(cfg.size)
            })
        });
        slots && i128::from(self.reserve(t).mem_mb) <= self.drained_mem
    }

    fn memory_fits(&self, t: &Ticket) -> bool {
        i128::from(self.reserve(t).mem_mb) <= self.free_mem
    }

    fn admit_on_paper(&mut self, t: &Ticket) {
        self.take(self.reserve(t));
        self.admitted_on_paper += 1;
        if t.class == Class::Heavy {
            self.heavy += 1;
            self.last_heavy = Some(self.snap.now);
        }
        for pool in &t.pools {
            *self.slots_on_paper.entry(pool.clone()).or_default() += 1;
        }
    }

    fn first_in_open(&self) -> bool {
        !self.living_charged && self.admitted_on_paper == 0
    }

    /// A quiet ticket admitted without the quiet streak, which only `quiet_max_wait` allows: its
    /// timing can't be trusted, so its record is marked tainted.
    fn tainted(&self, t: &Ticket) -> bool {
        t.class == Class::Quiet && self.quiet_streak < self.snap.config.quiet_streak_needed
    }

    /// A ticket whose aged hold has lapsed: it drains the machine instead, so it is admitted
    /// eventually even under steady traffic. A quiet ticket has its own drain (`due_quiet`).
    fn lapsed(&self, t: &Ticket) -> bool {
        let config = &self.snap.config;
        t.class != Class::Quiet
            && t.waited_ms > config.aging_ms.saturating_add(config.aging_hold_ms)
    }

    fn due_quiet(&self, t: &Ticket) -> bool {
        due_quiet(&self.snap.config, t)
    }

    fn aged(&self, t: &Ticket) -> bool {
        let config = &self.snap.config;
        t.waited_ms > config.aging_ms
            && t.waited_ms <= config.aging_ms.saturating_add(config.aging_hold_ms)
    }

    fn hold(&mut self, t: &Ticket) {
        self.take(self.reserve(t));
    }
}
