//! Random snapshots for the property tests of `decide` and `choose_victim`, tuned so that every
//! rule fires in a fair share of cases (most snapshots plausible, a minority past each limit).

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;

use crate::{
    Class, ClassMinimums, Config, Liveness, Load, Pool, PoolConfig, Resources, Snapshot, State,
    Ticket, TicketId, Wall,
};

pub(crate) const NOW: Wall = Wall(1_000_000_000);
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
        1 => any::<bool>().prop_map(|this_boot| Liveness::Orphaned { this_boot }),
    ]
}

fn resources(max_cpu: u64, max_mem: u64) -> impl Strategy<Value = Resources> {
    (0..=max_cpu, 0..=max_mem).prop_map(|(cpu, mem)| Resources::new(cpu, mem))
}

fn ticket(index: usize) -> impl Strategy<Value = Ticket> {
    (
        (
            0..6u64,
            class(),
            state(),
            liveness(),
            proptest::bool::weighted(0.8),
        ),
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
                (seq, class, state, liveness, holder_alive),
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
                    holder_alive,
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
            (0..8u32, 0..8u32),
        )
            .prop_map(
                |(
                    (fresh, age, quiet_streak),
                    (cpu_used, pressure),
                    (mem, gpu),
                    jobs,
                    (over_cpu_streak, over_gpu_streak),
                )| Load {
                    sampled_at: Wall(NOW.0 - age),
                    fresh,
                    woke_at: None,
                    capacity: Resources::new(8_000, 16_000),
                    cpu_used_millicores: cpu_used,
                    cpu_pressure: pressure,
                    mem_available_mb: mem,
                    gpu_utilization: gpu,
                    quiet_streak,
                    over_cpu_streak,
                    over_gpu_streak,
                    jobs,
                },
            ),
    )
}

pub(crate) fn snapshot() -> impl Strategy<Value = Snapshot> {
    (
        tickets(),
        load(),
        proptest::bool::weighted(0.1).prop_map(u32::from),
        proptest::option::of(0..60_000u64),
        (1..4u32, resources(2_000, 3_000), 0..4u32, 0..4u32),
        proptest::option::weighted(0.05, 0..8usize),
    )
        .prop_map(
            |(tickets, load, unreadable, last_heavy, (max_heavy, heavy_min, db, port), marker)| {
                Snapshot {
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
                        kill_floor_mb: 500,
                        mem_critical_mb: 200,
                        cpu_ceiling: 1.5,
                        gpu_ceiling: 0.9,
                        cpu_kill_samples: 3,
                        gpu_kill_samples: 3,
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
                    live_stop_markers: marker
                        .map(|i| TicketId(format!("t{i}")))
                        .into_iter()
                        .collect(),
                }
            },
        )
}
