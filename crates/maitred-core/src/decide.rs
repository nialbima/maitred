//! Admission: `decide(snapshot, me)`, ARCHITECTURE.md's "Inside `decide`" line by line.

use std::collections::BTreeMap;

use crate::{Class, Liveness, Pool, Resources, Snapshot, State, Ticket, TicketId, Wall};

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
    /// The walk stopped at a due quiet ticket, so the machine drains for it.
    DrainingForQuiet(TicketId),
    /// The walk stopped at a ticket admitted first in.
    BehindFirstIn(TicketId),
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

    if snap
        .tickets
        .iter()
        .filter_map(|t| t.recreated_at)
        .any(|at| snap.now.since(at) < config.stall_limit_ms())
    {
        return Decision::Wait(Reason::TicketsRecreated);
    }

    let charged: Vec<&Ticket> = snap.tickets.iter().filter(|t| is_charged(t)).collect();

    let Some(load) = reliable_load(snap) else {
        let nothing_charged = charged.is_empty() && snap.unreadable == 0;
        let first = waiting(snap, false).first().map(|t| &t.id);
        return if nothing_charged
            && first == Some(me)
            && slots_free(snap, mine, &BTreeMap::new()).is_ok()
        {
            Decision::Admit(Admission {
                fallback: true,
                ..Admission::default()
            })
        } else {
            Decision::Wait(Reason::NoReliableLoad)
        };
    };

    if load.mem_available_mb < config.mem_floor_mb {
        return Decision::Wait(Reason::LiveLimit(Limit::Memory));
    }
    if load.cpu_pressure > config.cpu_ceiling {
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
    for t in waiting(snap, true) {
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
                    return Decision::Wait(Reason::QuietJobRunning);
                }
                walk.admit_on_paper(t);
                continue;
            }
            Err(reason) => reason,
        };
        // First in: nothing living is charged and nothing was admitted earlier in this walk. It
        // waives cores and memory only, never a class rule or a slot.
        if walk.first_in_open()
            && allowed.is_ok()
            && slots_free(snap, t, &walk.slots_on_paper).is_ok()
        {
            return if is_me {
                Decision::Admit(Admission {
                    first_in: true,
                    ..Admission::default()
                })
            } else {
                Decision::Wait(Reason::BehindFirstIn(t.id.clone()))
            };
        }
        if is_me {
            return Decision::Wait(held);
        }
        if walk.due_quiet(t) {
            return Decision::Wait(Reason::DrainingForQuiet(t.id.clone()));
        }
        // An aged ticket that only the budget keeps out holds its place in it.
        if walk.aged(t) && allowed.is_ok() {
            walk.hold(t);
        }
    }
    // `me` is living and waiting, and a ticket deciding for itself has just written its pass, so the
    // walk reaches it. Only a snapshot built for someone else's pass can leave it stalled.
    Decision::Wait(Reason::NotWaiting)
}

/// The load figures `decide` needs, once it has checked they can be relied on.
struct Reliable {
    capacity: Resources,
    cpu_used_millicores: u64,
    cpu_pressure: f64,
    mem_available_mb: u64,
    gpu_utilization: Option<f64>,
    quiet_streak: u32,
}

/// The load, if it is fresh, recent enough and has its critical readings (CPU and memory).
fn reliable_load(snap: &Snapshot) -> Option<Reliable> {
    let load = snap.load.as_ref()?;
    if !load.fresh || snap.now.since(load.sampled_at) > snap.config.load_freshness_ms() {
        return None;
    }
    Some(Reliable {
        capacity: load.capacity,
        cpu_used_millicores: load.cpu_used_millicores?,
        cpu_pressure: load.cpu_pressure?,
        mem_available_mb: load.mem_available_mb?,
        gpu_utilization: load.gpu_utilization,
        quiet_streak: load.quiet_streak,
    })
}

/// Charged: living tickets admitted or running, and running orphans of this boot whose holder is
/// alive. (Unreadable tickets are charged too; the snapshot only counts them.)
fn is_charged(t: &Ticket) -> bool {
    match t.liveness {
        Liveness::Living => matches!(t.state, State::Admitted | State::Running),
        Liveness::Orphaned {
            holder_alive,
            this_boot,
        } => holder_alive && this_boot && t.state == State::Running,
    }
}

/// Living waiting tickets by `(seq, id)`, without the stalled ones when `skip_stalled`.
fn waiting(snap: &Snapshot, skip_stalled: bool) -> Vec<&Ticket> {
    let limit = snap.config.stall_limit_ms();
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
        .filter(|t| !skip_stalled || woke_recently || snap.now.since(t.last_pass) <= limit)
        .collect();
    tickets.sort_by(|a, b| (a.seq, &a.id).cmp(&(b.seq, &b.id)));
    tickets
}

/// Whether each pool `t` asks of has a slot no ticket names and nobody earlier in this walk took.
fn slots_free(snap: &Snapshot, t: &Ticket, on_paper: &BTreeMap<Pool, u32>) -> Result<(), Reason> {
    for pool in &t.pools {
        let size = snap.config.pools.get(pool).copied().unwrap_or(0);
        let named = snap
            .tickets
            .iter()
            .filter(|o| o.slots.contains_key(pool))
            .count();
        let taken = u64::try_from(named).unwrap_or(u64::MAX)
            + u64::from(on_paper.get(pool).copied().unwrap_or(0));
        if taken >= u64::from(size) {
            return Err(Reason::NoSlot(pool.clone()));
        }
    }
    Ok(())
}

/// The budget and the counts as the walk changes them on paper.
struct Walk<'a> {
    snap: &'a Snapshot,
    capacity: Resources,
    quiet_streak: u32,
    /// Free cores and memory; can go below zero once an aged ticket holds its place.
    free_cpu: i128,
    free_mem: i128,
    heavy: u32,
    last_heavy: Option<Wall>,
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
            heavy: 0,
            last_heavy: snap.last_heavy,
            living_charged: snap.unreadable > 0 || charged.iter().any(|t| t.is_living()),
            admitted_on_paper: 0,
            slots_on_paper: BTreeMap::new(),
        };
        let unreadable = walk.reserve_class(Class::Heavy, Resources::default());
        for _ in 0..snap.unreadable {
            walk.take(unreadable);
        }
        for t in charged {
            // Orphans count as using nothing, so they hold their whole reservation.
            let used = match t.liveness {
                Liveness::Living => snap
                    .load
                    .as_ref()
                    .and_then(|l| l.jobs.get(&t.id))
                    .copied()
                    .unwrap_or_default(),
                Liveness::Orphaned { .. } => Resources::default(),
            };
            walk.take(walk.reserve(t).saturating_sub(used));
            // An orphan's class barrier doesn't apply, so it doesn't count toward max_running.
            if t.is_living() && t.class == Class::Heavy {
                walk.heavy += 1;
            }
        }
        walk
    }

    fn reserve_class(&self, class: Class, asked: Resources) -> Resources {
        asked
            .max(self.snap.config.class_min.of(class))
            .min(self.capacity)
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
                if idle && (streak || self.tainted(t)) {
                    Ok(())
                } else {
                    Err(Reason::QuietNeedsIdle)
                }
            }
        }
    }

    fn fits(&self, t: &Ticket) -> Result<(), Reason> {
        let r = self.reserve(t);
        if i128::from(r.cpu_millicores) > self.free_cpu || i128::from(r.mem_mb) > self.free_mem {
            return Err(Reason::Budget);
        }
        slots_free(self.snap, t, &self.slots_on_paper)
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

    /// A quiet ticket past `quiet_max_wait`: admitted without the streak, and marked so.
    fn tainted(&self, t: &Ticket) -> bool {
        t.class == Class::Quiet && t.waited_ms > self.snap.config.quiet_max_wait_ms
    }

    fn due_quiet(&self, t: &Ticket) -> bool {
        let config = &self.snap.config;
        self.tainted(t)
            && t.waited_ms
                <= config
                    .quiet_max_wait_ms
                    .saturating_add(config.quiet_hold_ms)
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
