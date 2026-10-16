//! The kill policy's decisions: `trigger(snapshot)` and `choose_victim(snapshot, trigger)`,
//! ARCHITECTURE.md's "Kill policy", steps 1 and 2. Every holder runs both on the same sample, so
//! every holder names the same victim.

use std::cmp::Reverse;

use crate::{BootTime, Class, Resources, Snapshot, State, Ticket, TicketId};

#[cfg(test)]
mod props;
#[cfg(test)]
mod tests;

/// Why a job has to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Free memory is below `kill_floor`; critical below `mem_critical`.
    Memory { critical: bool },
    /// CPU pressure has been over its ceiling for `cpu_kill_samples`; critical at twice that.
    Cpu { critical: bool },
    /// GPU use has been over its ceiling for `gpu_kill_samples`. It has no critical level.
    Gpu,
}

/// Whether the kill policy acts on this sample, and why. Memory first, being what takes a machine
/// down, then CPU, then GPU. Nothing triggers on a load that can't be relied on (stale, missing a
/// reading, or the first sample after a sleep), while a kill is under way, or while a ticket or
/// marker was just recreated.
#[must_use]
pub fn trigger(snap: &Snapshot) -> Option<Trigger> {
    if !snap.live_stop_markers.is_empty() || snap.recreated_recently() {
        return None;
    }
    let load = snap.reliable_load()?;
    let config = &snap.config;
    // Critical is the machine's state, whichever trigger fired: past mem_critical, or the CPU
    // streak at twice its limit (Kill policy, step 2).
    let critical = load.mem_available_mb < config.mem_critical_mb
        || load.over_cpu_streak >= config.cpu_kill_samples.saturating_mul(2);
    if load.mem_available_mb < config.kill_floor_mb {
        return Some(Trigger::Memory { critical });
    }
    if load.over_cpu_streak >= config.cpu_kill_samples {
        return Some(Trigger::Cpu { critical });
    }
    if load.over_gpu_streak >= config.gpu_kill_samples {
        return Some(Trigger::Gpu);
    }
    None
}

/// The one job to stop for `trigger`, or none. For memory or CPU, the candidate furthest over its
/// reservation on that resource, if any is over; otherwise the most recently admitted heavy job,
/// then light, and quiet only when the trigger is critical. GPU has no per-job figure, so recency
/// alone, and never a quiet job. A victim named earlier for this breach stands while it is still
/// over, or while none is over and it is still eligible. Pure: per-job use comes only from the sample.
#[must_use]
pub fn choose_victim(snap: &Snapshot, trigger: Trigger) -> Option<TicketId> {
    if !snap.live_stop_markers.is_empty() || snap.recreated_recently() {
        return None;
    }
    let candidates: Vec<&Ticket> = snap
        .tickets
        .iter()
        .filter(|t| is_candidate(snap, t))
        .collect();

    let dimension: Option<fn(Resources) -> u64> = match trigger {
        Trigger::Memory { .. } => Some(|r| r.mem_mb),
        Trigger::Cpu { .. } => Some(|r| r.cpu_millicores),
        Trigger::Gpu => None,
    };
    let critical = matches!(
        trigger,
        Trigger::Memory { critical: true } | Trigger::Cpu { critical: true }
    );

    // The victim named earlier for this breach stands (MT-034), so holders reading different
    // samples can't name each other and stand down. It stands only while it is still over, or while
    // no candidate is over and this rule may still choose it: once the overuse has moved to another
    // job, the name moves with it.
    if let Some(named) = snap.named_victim.as_ref() {
        let is_over = |t: &Ticket| dimension.is_some_and(|of| over(snap, t, of).is_some());
        let none_over = !candidates.iter().any(|t| is_over(t));
        let still = candidates
            .iter()
            .find(|t| &t.id == named)
            .is_some_and(|t| is_over(t) || (none_over && (t.class != Class::Quiet || critical)));
        if still {
            return Some(named.clone());
        }
    }

    if let Some(of) = dimension {
        // Furthest over, then most recent, then smallest id.
        let furthest = candidates
            .iter()
            .filter_map(|t| over(snap, t, of).map(|by| (by, *t)))
            .max_by_key(|(by, t)| (*by, admitted(t), Reverse(&t.id)));
        if let Some((_, t)) = furthest {
            return Some(t.id.clone());
        }
    }

    let classes: &[Class] = if critical {
        &[Class::Heavy, Class::Light, Class::Quiet]
    } else {
        &[Class::Heavy, Class::Light]
    };
    classes.iter().find_map(|&class| {
        candidates
            .iter()
            .filter(|t| t.class == class)
            .max_by_key(|t| (admitted(t), Reverse(&t.id)))
            .map(|t| t.id.clone())
    })
}

/// A running job of this boot and namespace whose holder is alive and still passing, living or
/// orphaned: only its holder can carry out a kill.
fn is_candidate(snap: &Snapshot, t: &Ticket) -> bool {
    t.state == State::Running && t.holder_alive && t.is_this_boot() && !holder_stopped(snap, t)
}

/// Whether a job's holder hasn't passed within the stall limit (three of them without reliable
/// load), judged from its admission before its first pass. A SIGSTOP can't be ignored, and a
/// stopped holder can't act (MT-043). Nobody is judged so until a stall limit after a wake.
fn holder_stopped(snap: &Snapshot, t: &Ticket) -> bool {
    if snap.woke_recently() {
        return false;
    }
    let limit = if snap.reliable_load().is_some() {
        snap.config.stall_limit_ms()
    } else {
        snap.config.stall_limit_ms().saturating_mul(3)
    };
    t.holder_pass
        .or(t.admitted_at)
        .is_some_and(|at| snap.now.since(at) > limit)
}

/// How far a job's use in the sample is over its reservation on one resource, if it is over. A job
/// with no figure in the sample isn't known to be over.
fn over(snap: &Snapshot, t: &Ticket, of: fn(Resources) -> u64) -> Option<u64> {
    let load = snap.load.as_ref()?;
    let used = of(*load.jobs.get(&t.id)?);
    let reserved = of(snap.config.reserve(t.class, t.reservation, load.capacity));
    used.checked_sub(reserved).filter(|&by| by > 0)
}

fn admitted(t: &Ticket) -> BootTime {
    t.admitted_at.unwrap_or_default()
}
