//! The kill policy's decisions: `trigger(snapshot)` and `choose_victim(snapshot, trigger)`,
//! ARCHITECTURE.md's "Kill policy", steps 1 and 2. Every holder runs both on the same sample, so
//! every holder names the same victim.

use std::cmp::Reverse;

use crate::{Class, Liveness, Resources, Snapshot, State, Ticket, TicketId, Wall};

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
    if load.mem_available_mb < config.kill_floor_mb {
        return Some(Trigger::Memory {
            critical: load.mem_available_mb < config.mem_critical_mb,
        });
    }
    if load.over_cpu_streak >= config.cpu_kill_samples {
        let critical = load.over_cpu_streak >= config.cpu_kill_samples.saturating_mul(2);
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
/// alone, and never a quiet job. Pure: per-job use comes only from the sample.
#[must_use]
pub fn choose_victim(snap: &Snapshot, trigger: Trigger) -> Option<TicketId> {
    if !snap.live_stop_markers.is_empty() || snap.recreated_recently() {
        return None;
    }
    let candidates: Vec<&Ticket> = snap.tickets.iter().filter(|t| is_candidate(t)).collect();

    let dimension: Option<fn(Resources) -> u64> = match trigger {
        Trigger::Memory { .. } => Some(|r| r.mem_mb),
        Trigger::Cpu { .. } => Some(|r| r.cpu_millicores),
        Trigger::Gpu => None,
    };
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

    let critical = matches!(
        trigger,
        Trigger::Memory { critical: true } | Trigger::Cpu { critical: true }
    );
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

/// A running job of this boot and namespace whose holder is alive, living or orphaned: only its
/// holder can carry out a kill.
fn is_candidate(t: &Ticket) -> bool {
    let this_boot = match t.liveness {
        Liveness::Living => true,
        Liveness::Orphaned { this_boot } => this_boot,
    };
    t.state == State::Running && t.holder_alive && this_boot
}

/// How far a job's use in the sample is over its reservation on one resource, if it is over. A job
/// with no figure in the sample isn't known to be over.
fn over(snap: &Snapshot, t: &Ticket, of: fn(Resources) -> u64) -> Option<u64> {
    let load = snap.load.as_ref()?;
    let used = of(*load.jobs.get(&t.id)?);
    let reserved = of(snap.config.reserve(t.class, t.reservation, load.capacity));
    used.checked_sub(reserved).filter(|&by| by > 0)
}

fn admitted(t: &Ticket) -> Wall {
    t.admitted_at.unwrap_or_default()
}
