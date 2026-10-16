//! Property tests over random snapshots: the `decide` row of ARCHITECTURE.md's Testing table. Each
//! property asks `decide` for every waiting ticket of one snapshot, as every waiter does in one pass,
//! and checks what would be admitted together.

use proptest::prelude::*;

use super::*;
use crate::testgen::snapshot;
use crate::{Liveness, Resources, Snapshot, State, Ticket};

/// Every ticket `decide` admits in this snapshot, with how.
fn admissions(snap: &Snapshot) -> Vec<(&Ticket, Admission)> {
    snap.tickets
        .iter()
        .filter_map(|t| match decide(snap, &t.id) {
            Decision::Admit(how) => Some((t, how)),
            Decision::Wait(_) => None,
        })
        .collect()
}

fn reserve(snap: &Snapshot, t: &Ticket) -> Resources {
    let capacity = snap
        .load
        .as_ref()
        .map_or(Resources::new(u64::MAX, u64::MAX), |l| l.capacity);
    t.reservation
        .max(snap.config.class_min.of(t.class))
        .min(capacity)
}

fn living_charged(t: &Ticket) -> bool {
    t.liveness == Liveness::Living && t.state != State::Waiting
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

    #[test]
    fn only_living_waiting_tickets_are_admitted(snap in snapshot()) {
        for (t, _) in admissions(&snap) {
            prop_assert_eq!(t.state, State::Waiting);
            prop_assert_eq!(t.liveness, Liveness::Living);
        }
    }

    #[test]
    fn nothing_starts_past_the_live_limits(snap in snapshot()) {
        let Some(load) = snap.load.as_ref() else { return Ok(()) };
        let over = load.mem_available_mb.is_some_and(|m| m < snap.config.mem_floor_mb)
            || load.cpu_pressure.is_some_and(|p| p > snap.config.cpu_ceiling)
            || load.gpu_utilization.is_some_and(|g| g > snap.config.gpu_ceiling);
        for (_, how) in admissions(&snap) {
            // Only the fallback, which has no reliable load to judge by, may admit past one.
            prop_assert!(!over || how.fallback);
        }
    }

    #[test]
    fn nothing_starts_beside_a_running_quiet_job(snap in snapshot()) {
        let quiet_running = snap.tickets.iter().any(|t| living_charged(t) && t.class == Class::Quiet);
        prop_assert!(!quiet_running || admissions(&snap).is_empty());
    }

    #[test]
    fn nothing_is_admitted_while_a_ticket_was_just_recreated(snap in snapshot()) {
        let recent = snap.recreated_recently();
                prop_assert!(!recent || admissions(&snap).is_empty());
    }

    #[test]
    fn at_most_one_ticket_gets_in_without_the_budget(snap in snapshot()) {
        let unbudgeted = admissions(&snap).iter().filter(|(_, how)| how.first_in || how.fallback).count();
        prop_assert!(unbudgeted <= 1);
    }

    #[test]
    fn a_ticket_let_in_without_the_budget_is_the_only_one(snap in snapshot()) {
        let admitted = admissions(&snap);
        if admitted.iter().any(|(_, how)| how.first_in || how.fallback) {
            prop_assert_eq!(admitted.len(), 1);
        }
    }

    #[test]
    fn first_in_happens_only_when_nothing_living_is_charged(snap in snapshot()) {
        let charged = snap.tickets.iter().any(living_charged);
        for (_, how) in admissions(&snap) {
            prop_assert!(!(how.first_in && charged));
        }
    }

    #[test]
    fn admissions_never_exceed_the_free_budget(snap in snapshot()) {
        let admitted = admissions(&snap);
        if admitted.iter().any(|(_, how)| how.first_in || how.fallback) {
            return Ok(());
        }
        let Some(load) = snap.load.as_ref() else { return Ok(()) };
        // ARCHITECTURE.md's free.cpu and free.mem, restated: capacity less what is in use, less
        // what each charged ticket hasn't used of its reservation yet.
        let mut free_cpu = i128::from(load.capacity.cpu_millicores) - i128::from(load.cpu_used_millicores.unwrap_or(0));
        // Less the floor, which the budget keeps back (MT-035).
        let mut free_mem = i128::from(load.mem_available_mb.unwrap_or(0)) - i128::from(snap.config.mem_floor_mb);
        let unreadable = snap.config.class_min.heavy.min(load.capacity);
        free_cpu -= i128::from(snap.unreadable) * i128::from(unreadable.cpu_millicores);
        free_mem -= i128::from(snap.unreadable) * i128::from(unreadable.mem_mb);
        for t in &snap.tickets {
            let used = match t.liveness {
                Liveness::Living if t.state != State::Waiting => load.jobs.get(&t.id).copied().unwrap_or_default(),
                Liveness::Orphaned { this_boot: true } if t.holder_alive && t.state == State::Running => Resources::default(),
                _ => continue,
            };
            let charge = reserve(&snap, t).saturating_sub(used);
            free_cpu -= i128::from(charge.cpu_millicores);
            free_mem -= i128::from(charge.mem_mb);
        }
        let cpu: i128 = admitted.iter().map(|(t, _)| i128::from(reserve(&snap, t).cpu_millicores)).sum();
        let mem: i128 = admitted.iter().map(|(t, _)| i128::from(reserve(&snap, t).mem_mb)).sum();
        prop_assert!(cpu <= free_cpu.max(0) || admitted.is_empty());
        prop_assert!(mem <= free_mem.max(0) || admitted.is_empty());
    }

    #[test]
    fn no_two_admissions_take_one_slot(snap in snapshot()) {
        let admitted = admissions(&snap);
        for (pool, cfg) in &snap.config.pools {
            // Taken as the spec counts it: distinct numbers in range, held by tickets past waiting.
            let named = snap
                .tickets
                .iter()
                .filter(|t| t.state != State::Waiting)
                .filter_map(|t| t.slots.get(pool).copied())
                .filter(|&n| cfg.contains(n))
                .collect::<std::collections::BTreeSet<u32>>()
                .len();
            let asking = admitted.iter().filter(|(t, _)| t.pools.contains(pool)).count();
            if asking > 0 {
                prop_assert!(named + asking <= cfg.size as usize);
            }
        }
    }

    #[test]
    fn heavy_class_rules_are_never_waived(snap in snapshot()) {
        let admitted = admissions(&snap);
        let heavy_in: Vec<_> = admitted.iter().filter(|(t, _)| t.class == Class::Heavy).collect();
        if heavy_in.is_empty() || admitted.iter().any(|(_, how)| how.fallback) {
            return Ok(());
        }
        // Settle: no heavy start within settle of the last, and so at most one per pass.
        prop_assert!(snap.last_heavy.is_none_or(|at| snap.now.since(at) >= snap.config.settle_ms));
        prop_assert_eq!(heavy_in.len(), 1);
        let running = snap.tickets.iter().filter(|t| living_charged(t) && t.class == Class::Heavy).count();
        prop_assert!(running < snap.config.max_running_heavy as usize);
    }

    #[test]
    fn a_quiet_job_starts_only_on_its_own(snap in snapshot()) {
        let admitted = admissions(&snap);
        if admitted.iter().any(|(t, _)| t.class == Class::Quiet) && !admitted.iter().any(|(_, how)| how.fallback) {
            prop_assert_eq!(admitted.len(), 1);
            prop_assert!(!snap.tickets.iter().any(living_charged));
        }
    }

    #[test]
    fn the_answer_depends_only_on_the_snapshot_not_its_order(snap in snapshot(), seed in any::<u64>()) {
        let mut shuffled = snap.clone();
        // A deterministic shuffle from the seed: rotate, then reverse on odd seeds.
        let n = shuffled.tickets.len().max(1);
        shuffled.tickets.rotate_left(usize::try_from(seed % n as u64).unwrap_or(0));
        if seed % 2 == 1 {
            shuffled.tickets.reverse();
        }
        for t in &snap.tickets {
            prop_assert_eq!(decide(&snap, &t.id), decide(&shuffled, &t.id));
        }
    }
}
