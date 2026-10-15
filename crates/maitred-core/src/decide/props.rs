//! Property tests over random snapshots: the `decide` row of ARCHITECTURE.md's Testing table. Each
//! property asks `decide` for every waiting ticket of one snapshot, as every waiter does in one pass,
//! and checks what would be admitted together.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;

use super::*;
use crate::{
    ClassMinimums, Config, Liveness, Load, Pool, PoolConfig, Resources, Snapshot, State, Ticket,
    TicketId, Wall,
};

const NOW: Wall = Wall(1_000_000_000);
const POOLS: [&str; 2] = ["db", "port"];

fn class() -> impl Strategy<Value = Class> {
    prop_oneof![6 => Just(Class::Light), 4 => Just(Class::Heavy), 1 => Just(Class::Quiet)]
}

fn state() -> impl Strategy<Value = State> {
    prop_oneof![4 => Just(State::Waiting), 1 => Just(State::Admitted), 2 => Just(State::Running)]
}

fn liveness() -> impl Strategy<Value = Liveness> {
    prop_oneof![
        4 => Just(Liveness::Living),
        1 => (any::<bool>(), any::<bool>())
            .prop_map(|(holder_alive, this_boot)| Liveness::Orphaned { holder_alive, this_boot }),
    ]
}

fn resources(max_cpu: u64, max_mem: u64) -> impl Strategy<Value = Resources> {
    (0..=max_cpu, 0..=max_mem).prop_map(|(cpu, mem)| Resources::new(cpu, mem))
}

fn ticket(index: usize) -> impl Strategy<Value = Ticket> {
    (
        (0..6u64, class(), state(), liveness()),
        resources(2_000, 4_000),
        proptest::collection::btree_set(0..POOLS.len(), 0..=2),
        (
            0..1_000_000u64,
            0..12_000u64,
            proptest::option::weighted(0.02, 0..20u64),
        ),
    )
        .prop_map(
            move |(
                (seq, class, state, liveness),
                reservation,
                pools,
                (waited_ms, pass_age, recreated),
            )| {
                let pools: BTreeSet<Pool> =
                    pools.into_iter().map(|p| Pool(POOLS[p].into())).collect();
                // A ticket past waiting holds one slot of each pool it asked for (numbers don't matter
                // to decide, only which tickets name a pool).
                let slots = if state == State::Waiting {
                    BTreeMap::new()
                } else {
                    pools.iter().map(|p| (p.clone(), 1)).collect()
                };
                Ticket {
                    id: TicketId(format!("t{index}")),
                    seq,
                    class,
                    state,
                    liveness,
                    reservation,
                    pools,
                    slots,
                    waited_ms,
                    last_pass: Wall(NOW.0 - pass_age),
                    admitted_at: (state != State::Waiting).then_some(Wall(NOW.0 - 60_000)),
                    // Rarely recent, so most snapshots get past the recreation rule.
                    recreated_at: recreated.map(|age| Wall(NOW.0 - age * 1_000)),
                }
            },
        )
}

fn tickets() -> impl Strategy<Value = Vec<Ticket>> {
    (0..8usize).prop_flat_map(|n| (0..n).map(ticket).collect::<Vec<_>>())
}

fn load() -> impl Strategy<Value = Option<Load>> {
    proptest::option::weighted(
        0.95,
        (
            (proptest::bool::weighted(0.97), 0..6_500u64, 0..3u32),
            (
                proptest::option::weighted(0.98, 0..2_000u64),
                proptest::option::weighted(0.98, 0.0..1.6f64),
            ),
            (
                proptest::option::weighted(0.98, 0..16_000u64)
                    .prop_map(|m| m.map(|m| m.max(2_000) - 2_000 + 1_000)),
                proptest::option::of(0.0..0.95f64),
            ),
            proptest::collection::btree_map(
                (0..8usize).prop_map(|i| TicketId(format!("t{i}"))),
                resources(10_000, 20_000),
                0..4,
            ),
        )
            .prop_map(
                |((fresh, age, quiet_streak), (cpu_used, pressure), (mem, gpu), jobs)| Load {
                    sampled_at: Wall(NOW.0 - age),
                    fresh,
                    woke_at: None,
                    capacity: Resources::new(8_000, 16_000),
                    cpu_used_millicores: cpu_used,
                    cpu_pressure: pressure,
                    mem_available_mb: mem,
                    gpu_utilization: gpu,
                    quiet_streak,
                    jobs,
                },
            ),
    )
}

fn snapshot() -> impl Strategy<Value = Snapshot> {
    (
        tickets(),
        load(),
        proptest::bool::weighted(0.1).prop_map(u32::from),
        proptest::option::of(0..60_000u64),
        (1..4u32, resources(2_000, 3_000), 0..4u32, 0..4u32),
    )
        .prop_map(
            |(tickets, load, unreadable, last_heavy, (max_heavy, heavy_min, db, port))| Snapshot {
                now: NOW,
                config: Config {
                    poll_interval_ms: 2_000,
                    sample_interval_ms: 2_000,
                    settle_ms: 30_000,
                    aging_ms: 300_000,
                    aging_hold_ms: 300_000,
                    quiet_max_wait_ms: 600_000,
                    stall_polls: 5,
                    load_freshness_samples: 3,
                    mem_floor_mb: 1_000,
                    cpu_ceiling: 1.5,
                    gpu_ceiling: 0.9,
                    max_running_heavy: max_heavy,
                    quiet_streak_needed: 2,
                    class_min: ClassMinimums {
                        light: Resources::new(100, 100),
                        heavy: heavy_min,
                        quiet: Resources::default(),
                    },
                    pools: [
                        (Pool(POOLS[0].into()), PoolConfig { first: 1, size: db }),
                        (
                            Pool(POOLS[1].into()),
                            PoolConfig {
                                first: 1,
                                size: port,
                            },
                        ),
                    ]
                    .into(),
                },
                load,
                tickets,
                unreadable,
                last_heavy: last_heavy.map(|age| Wall(NOW.0 - age)),
                moved: BTreeMap::new(),
            },
        )
}

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
        let limit = snap.config.stall_limit_ms();
        let recent = snap.tickets.iter().filter_map(|t| t.recreated_at).any(|at| snap.now.since(at) < limit);
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
        let charged = snap.unreadable > 0 || snap.tickets.iter().any(living_charged);
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
        let mut free_mem = i128::from(load.mem_available_mb.unwrap_or(0));
        let unreadable = snap.config.class_min.heavy.min(load.capacity);
        free_cpu -= i128::from(snap.unreadable) * i128::from(unreadable.cpu_millicores);
        free_mem -= i128::from(snap.unreadable) * i128::from(unreadable.mem_mb);
        for t in &snap.tickets {
            let used = match t.liveness {
                Liveness::Living if t.state != State::Waiting => load.jobs.get(&t.id).copied().unwrap_or_default(),
                Liveness::Orphaned { holder_alive: true, this_boot: true } if t.state == State::Running => Resources::default(),
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
            let named = snap.tickets.iter().filter(|t| t.slots.get(pool).is_some_and(|&n| cfg.contains(n))).count();
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
            prop_assert!(snap.unreadable == 0 && !snap.tickets.iter().any(living_charged));
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
