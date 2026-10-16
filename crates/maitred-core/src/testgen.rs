//! Random snapshots for the property tests of `decide` and `choose_victim`, tuned so that every
//! rule fires in a fair share of cases (most snapshots plausible, a minority past each limit).

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;

use crate::{
    BootTime, Class, ClassMinimums, Config, CpuLoad, Liveness, Load, Pool, PoolConfig, Position,
    Resources, Snapshot, State, Ticket, TicketId,
};

pub(crate) const NOW: BootTime = BootTime(1_000_000_000);
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
            // How long ago a ticket past waiting was admitted, so recency varies.
            0..3_600_000u64,
        ),
    )
        .prop_map(
            move |(
                (seq, class, state, liveness, holder_alive),
                reservation,
                pools,
                (waited_ms, pass_age, recreated, admitted_ago),
            )| {
                let pools: BTreeSet<Pool> =
                    pools.into_iter().map(|p| Pool(POOLS[p].into())).collect();
                // A ticket past waiting holds one slot of each pool it asked for: its own number,
                // index + 1, so no two tickets name one number (which can't happen for real).
                let slots = if state == State::Waiting {
                    BTreeMap::new()
                } else {
                    pools
                        .iter()
                        .map(|p| (p.clone(), u32::try_from(index).unwrap_or(u32::MAX) + 1))
                        .collect()
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
                    last_pass: BootTime(NOW.0 - pass_age),
                    admitted_at: (state != State::Waiting)
                        .then_some(BootTime(NOW.0 - admitted_ago)),
                    // Rarely recent, so most snapshots get past the recreation rule.
                    recreated_at: recreated.map(|age| BootTime(NOW.0 - age * 1_000)),
                }
            },
        )
}

fn tickets() -> impl Strategy<Value = Vec<Ticket>> {
    (0..8usize).prop_flat_map(|n| (0..n).map(ticket).collect::<Vec<_>>())
}

/// Free memory: mostly comfortable, with about one sample in twenty below the kill floor (500) and
/// one in twenty between it and the admission floor (1000), so both memory branches are reached.
fn mem_available() -> impl Strategy<Value = Option<u64>> {
    // A critical reading goes missing in about 2% of samples (no reliable load).
    proptest::option::weighted(
        0.98,
        prop_oneof![
            18 => 1_000..16_000u64,
            1 => 500..1_000u64,
            1 => 0..500u64,
        ],
    )
}

fn load() -> impl Strategy<Value = Option<Load>> {
    // No load file at all in 5% of snapshots: the plain-lock fallback.
    proptest::option::weighted(
        0.95,
        (
            // Not fresh (first sample after a sleep) in 3%; an age past the 6000 ms freshness in
            // about 8%; the quiet streak 0 to 2, so it is met (2) in a third.
            (proptest::bool::weighted(0.97), 0..6_500u64, 0..3u32),
            (
                // Outside CPU use up to a quarter of the machine, so budgets often fit.
                proptest::option::weighted(0.98, 0..2_000u64),
                // Either figure, each past its own ceiling (0.3, 1.5) in a few percent of samples.
                proptest::option::weighted(
                    0.98,
                    prop_oneof![
                        (0.0..0.32f64).prop_map(CpuLoad::Stall),
                        (0.0..1.6f64).prop_map(CpuLoad::RunnablePerCore),
                    ],
                ),
            ),
            // GPU absent in half; under the 0.9 ceiling almost always when present.
            (mem_available(), proptest::option::of(0.0..0.95f64)),
            proptest::collection::btree_map(
                (0..8usize).prop_map(|i| TicketId(format!("t{i}"))),
                resources(10_000, 20_000),
                0..4,
            ),
            // Kill streaks 0 to 7 against a limit of 3: CPU and GPU triggers in about half.
            (0..8u32, 0..8u32),
            // Woke recently (within the 10 s stall limit) in about 10%, so stalls are suspended.
            proptest::option::weighted(0.1, 0..20_000u64),
            // Holder passes up to 30 s old, so some holders fall a stall limit (10 s) behind the
            // freshest; a missing one is judged from its admission.
            proptest::collection::btree_map(
                (0..8usize).prop_map(|i| TicketId(format!("t{i}"))),
                (0..30_000u64).prop_map(|ago| BootTime(NOW.0 - ago)),
                0..8,
            ),
        )
            .prop_map(
                |(
                    (fresh, age, quiet_streak),
                    (cpu_used, pressure),
                    (mem, gpu),
                    jobs,
                    (over_cpu_streak, over_gpu_streak),
                    woke,
                    holder_passes,
                )| Load {
                    sampled_at: BootTime(NOW.0 - age),
                    fresh,
                    woke_at: woke.map(|ago| BootTime(NOW.0 - ago)),
                    capacity: Resources::new(8_000, 16_000),
                    cpu_used_millicores: cpu_used,
                    cpu_load: pressure,
                    mem_available_mb: mem,
                    gpu_utilization: gpu,
                    quiet_streak,
                    over_cpu_streak,
                    over_gpu_streak,
                    jobs,
                    holder_passes,
                },
            ),
    )
}

pub(crate) fn snapshot() -> impl Strategy<Value = Snapshot> {
    (
        tickets(),
        load(),
        // One unreadable ticket in 10% of snapshots: charged the heavy minimum, blocks first-in.
        proptest::bool::weighted(0.1).prop_map(u32::from),
        proptest::option::of(0..60_000u64),
        (1..4u32, resources(2_000, 3_000), 0..4u32, 0..4u32),
        // A live stop marker in 5% of snapshots: a kill under way, so nothing is chosen.
        proptest::option::weighted(0.05, 0..8usize),
        // An administrator's move in 10%: ticket i sorts at seq s with sub -1.
        proptest::option::weighted(0.1, (0..8usize, 0..6u64)),
        // A victim already named for the breach in 10%.
        proptest::option::weighted(0.1, 0..8usize),
        // This supervisor woke (a gap in its own passes) within the last 20 s in 10%.
        proptest::option::weighted(0.1, 0..20_000u64),
    )
        .prop_map(
            |(
                tickets,
                load,
                unreadable,
                last_heavy,
                (max_heavy, heavy_min, db, port),
                marker,
                mv,
                named,
                woke,
            )| {
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
                        cpu_stall_ceiling: 0.3,
                        cpu_runnable_ceiling: 1.5,
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
                    last_heavy: last_heavy.map(|age| BootTime(NOW.0 - age)),
                    moved: mv
                        .map(|(i, seq)| (TicketId(format!("t{i}")), Position { seq, sub: -1 }))
                        .into_iter()
                        .collect(),
                    live_stop_markers: marker
                        .map(|i| TicketId(format!("t{i}")))
                        .into_iter()
                        .collect(),
                    held_outside: BTreeMap::new(),
                    named_victim: named.map(|i| TicketId(format!("t{i}"))),
                    my_wake: woke.map(|ago| BootTime(NOW.0 - ago)),
                }
            },
        )
}
