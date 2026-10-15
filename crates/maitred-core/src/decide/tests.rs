//! One test per rule of ARCHITECTURE.md's "Inside `decide`", in the order the pseudo-code states
//! them, then the terms of its table.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::{
    ClassMinimums, Config, Liveness, Load, Pool, Resources, Snapshot, State, Ticket, TicketId, Wall,
};

const NOW: Wall = Wall(1_000_000_000);

/// A machine config with short, round numbers; each test that depends on one names it.
fn config() -> Config {
    Config {
        poll_interval_ms: 2_000,
        sample_interval_ms: 2_000,
        settle_ms: 30_000,
        aging_ms: 600_000,
        aging_hold_ms: 600_000,
        quiet_max_wait_ms: 3_600_000,
        stall_polls: 5,
        load_freshness_samples: 3,
        mem_floor_mb: 1_000,
        cpu_ceiling: 1.5,
        gpu_ceiling: 0.9,
        max_running_heavy: 2,
        quiet_streak_needed: 2,
        class_min: ClassMinimums::default(),
        pools: BTreeMap::new(),
    }
}

/// An idle 8-core, 16 GB machine, sampled just now.
fn load() -> Load {
    Load {
        sampled_at: NOW,
        fresh: true,
        woke_at: None,
        capacity: Resources::new(8_000, 16_000),
        cpu_used_millicores: Some(0),
        cpu_pressure: Some(0.0),
        mem_available_mb: Some(16_000),
        gpu_utilization: None,
        quiet_streak: 0,
        jobs: BTreeMap::new(),
    }
}

fn ticket(id: &str, seq: u64, class: Class, state: State) -> Ticket {
    Ticket {
        id: TicketId(id.into()),
        seq,
        class,
        state,
        liveness: Liveness::Living,
        reservation: Resources::new(1_000, 1_000),
        pools: BTreeSet::new(),
        slots: BTreeMap::new(),
        waited_ms: 0,
        last_pass: NOW,
        admitted_at: (state != State::Waiting).then_some(Wall(NOW.0 - 3_600_000)),
        recreated_at: None,
    }
}

fn snapshot(tickets: Vec<Ticket>) -> Snapshot {
    Snapshot {
        now: NOW,
        config: config(),
        load: Some(load()),
        tickets,
        unreadable: 0,
        last_heavy: None,
    }
}

fn me() -> TicketId {
    TicketId("me".into())
}

fn admitted() -> Decision {
    Decision::Admit(Admission::default())
}

#[test]
fn a_ticket_that_is_not_waiting_is_never_admitted() {
    let snap = snapshot(vec![ticket("me", 1, Class::Light, State::Running)]);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NotWaiting));
}

#[test]
fn a_ticket_missing_from_the_snapshot_is_never_admitted() {
    let snap = snapshot(vec![]);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NotWaiting));
}

#[test]
fn a_lone_light_ticket_on_an_idle_machine_is_admitted() {
    let snap = snapshot(vec![ticket("me", 1, Class::Light, State::Waiting)]);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn nobody_is_admitted_while_a_ticket_was_recreated_within_the_stall_limit() {
    let mut other = ticket("other", 1, Class::Light, State::Running);
    other.recreated_at = Some(Wall(NOW.0 - 9_999));
    let snap = snapshot(vec![other, ticket("me", 2, Class::Light, State::Waiting)]);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Wait(Reason::TicketsRecreated)
    );
}

#[test]
fn a_recreation_older_than_the_stall_limit_no_longer_holds_admission() {
    let mut other = ticket("other", 1, Class::Light, State::Running);
    other.recreated_at = Some(Wall(NOW.0 - 10_001));
    let snap = snapshot(vec![other, ticket("me", 2, Class::Light, State::Waiting)]);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn with_no_load_file_the_first_waiter_is_admitted_when_nothing_is_charged() {
    let mut snap = snapshot(vec![
        ticket("me", 1, Class::Heavy, State::Waiting),
        ticket("later", 2, Class::Light, State::Waiting),
    ]);
    snap.load = None;

    assert_eq!(
        decide(&snap, &me()),
        Decision::Admit(Admission {
            fallback: true,
            ..Admission::default()
        })
    );
}

#[test]
fn with_no_load_file_a_waiter_behind_the_first_waits() {
    let mut snap = snapshot(vec![
        ticket("first", 1, Class::Light, State::Waiting),
        ticket("me", 2, Class::Light, State::Waiting),
    ]);
    snap.load = None;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoReliableLoad));
}

#[test]
fn with_no_load_file_nobody_is_admitted_beside_a_charged_ticket() {
    let mut snap = snapshot(vec![
        ticket("running", 1, Class::Light, State::Running),
        ticket("me", 2, Class::Light, State::Waiting),
    ]);
    snap.load = None;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoReliableLoad));
}

#[test]
fn with_no_load_file_an_unreadable_ticket_counts_as_charged() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Light, State::Waiting)]);
    snap.load = None;
    snap.unreadable = 1;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoReliableLoad));
}

#[test]
fn with_no_load_file_a_charged_orphan_does_not_keep_everyone_out() {
    // As for first in: an orphan that never ends must not hold the queue for ever.
    let mut orphan = ticket("orphan", 1, Class::Light, State::Running);
    orphan.liveness = Liveness::Orphaned {
        holder_alive: true,
        this_boot: true,
    };
    let mut snap = snapshot(vec![orphan, ticket("me", 2, Class::Light, State::Waiting)]);
    snap.load = None;

    assert_eq!(
        decide(&snap, &me()),
        Decision::Admit(Admission {
            fallback: true,
            ..Admission::default()
        })
    );
}

#[test]
fn with_no_load_file_nobody_counts_as_stalled() {
    let mut stuck = ticket("stuck", 1, Class::Light, State::Waiting);
    stuck.last_pass = Wall(NOW.0 - 3_600_000);
    let mut snap = snapshot(vec![stuck, ticket("me", 2, Class::Light, State::Waiting)]);
    snap.load = None;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoReliableLoad));
}

#[test]
fn a_sample_marked_not_fresh_is_no_reliable_load() {
    let mut snap = snapshot(vec![
        ticket("running", 1, Class::Light, State::Running),
        ticket("me", 2, Class::Light, State::Waiting),
    ]);
    snap.load.as_mut().unwrap().fresh = false;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoReliableLoad));
}

#[test]
fn a_sample_older_than_the_load_freshness_is_no_reliable_load() {
    let mut snap = snapshot(vec![
        ticket("running", 1, Class::Light, State::Running),
        ticket("me", 2, Class::Light, State::Waiting),
    ]);
    snap.load.as_mut().unwrap().sampled_at = Wall(NOW.0 - 6_001);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoReliableLoad));
}

#[test]
fn a_sample_exactly_at_the_load_freshness_still_counts() {
    let mut snap = snapshot(vec![
        ticket("running", 1, Class::Light, State::Running),
        ticket("me", 2, Class::Light, State::Waiting),
    ]);
    snap.load.as_mut().unwrap().sampled_at = Wall(NOW.0 - 6_000);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn a_missing_cpu_reading_is_no_reliable_load() {
    let mut snap = snapshot(vec![
        ticket("running", 1, Class::Light, State::Running),
        ticket("me", 2, Class::Light, State::Waiting),
    ]);
    snap.load.as_mut().unwrap().cpu_used_millicores = None;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoReliableLoad));
}

#[test]
fn a_missing_cpu_pressure_reading_is_no_reliable_load() {
    let mut snap = snapshot(vec![
        ticket("running", 1, Class::Light, State::Running),
        ticket("me", 2, Class::Light, State::Waiting),
    ]);
    snap.load.as_mut().unwrap().cpu_pressure = None;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoReliableLoad));
}

#[test]
fn a_missing_memory_reading_is_no_reliable_load() {
    let mut snap = snapshot(vec![
        ticket("running", 1, Class::Light, State::Running),
        ticket("me", 2, Class::Light, State::Waiting),
    ]);
    snap.load.as_mut().unwrap().mem_available_mb = None;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoReliableLoad));
}

#[test]
fn memory_below_the_floor_holds_everyone() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Light, State::Waiting)]);
    snap.load.as_mut().unwrap().mem_available_mb = Some(999);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Wait(Reason::LiveLimit(Limit::Memory))
    );
}

#[test]
fn memory_exactly_at_the_floor_is_not_below_it() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Light, State::Waiting)]);
    snap.load.as_mut().unwrap().mem_available_mb = Some(1_000);
    snap.config.class_min.light = Resources::new(0, 0);
    snap.tickets[0].reservation = Resources::new(0, 0);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn cpu_pressure_above_the_ceiling_holds_everyone() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Light, State::Waiting)]);
    snap.load.as_mut().unwrap().cpu_pressure = Some(1.51);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Wait(Reason::LiveLimit(Limit::Cpu))
    );
}

#[test]
fn gpu_use_above_the_ceiling_holds_everyone() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Light, State::Waiting)]);
    snap.load.as_mut().unwrap().gpu_utilization = Some(0.95);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Wait(Reason::LiveLimit(Limit::Gpu))
    );
}

#[test]
fn a_machine_with_no_readable_gpu_has_no_gpu_limit() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Light, State::Waiting)]);
    snap.load.as_mut().unwrap().gpu_utilization = None;

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn nothing_starts_beside_a_running_quiet_job() {
    let snap = snapshot(vec![
        ticket("quiet", 1, Class::Quiet, State::Running),
        ticket("me", 2, Class::Light, State::Waiting),
    ]);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Wait(Reason::QuietJobRunning)
    );
}

#[test]
fn an_orphaned_quiet_job_does_not_stop_the_queue() {
    let mut quiet = ticket("quiet", 1, Class::Quiet, State::Running);
    quiet.liveness = Liveness::Orphaned {
        holder_alive: true,
        this_boot: true,
    };
    let snap = snapshot(vec![quiet, ticket("me", 2, Class::Light, State::Waiting)]);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn a_ticket_that_fits_the_free_budget_is_admitted_beside_a_running_one() {
    let mut running = ticket("running", 1, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 8_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(4_000, 8_000);

    assert_eq!(decide(&snapshot(vec![running, mine]), &me()), admitted());
}

#[test]
fn a_ticket_over_the_free_cpu_waits_for_budget() {
    let mut running = ticket("running", 1, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(4_001, 1_000);

    assert_eq!(
        decide(&snapshot(vec![running, mine]), &me()),
        Decision::Wait(Reason::Budget)
    );
}

#[test]
fn a_ticket_over_the_free_memory_waits_for_budget() {
    let mut running = ticket("running", 1, Class::Light, State::Running);
    running.reservation = Resources::new(1_000, 8_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(1_000, 8_001);

    assert_eq!(
        decide(&snapshot(vec![running, mine]), &me()),
        Decision::Wait(Reason::Budget)
    );
}

#[test]
fn cpu_already_in_use_comes_off_the_free_budget() {
    let mut running = ticket("running", 1, Class::Light, State::Running);
    running.reservation = Resources::new(1_000, 1_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(4_000, 1_000);
    let mut snap = snapshot(vec![running, mine]);
    snap.load.as_mut().unwrap().cpu_used_millicores = Some(4_001);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Budget));
}

#[test]
fn a_measured_job_is_charged_only_what_it_has_not_used_yet() {
    // 8000 capacity, 3000 in use (all the running job's), so 5000 free before the charge. The job
    // reserves 4000 and uses 3000, so it is charged 1000 more: 4000 free, and 4000 fits.
    let mut running = ticket("running", 1, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(4_000, 1_000);
    let mut snap = snapshot(vec![running, mine]);
    let load = snap.load.as_mut().unwrap();
    load.cpu_used_millicores = Some(3_000);
    load.jobs
        .insert(TicketId("running".into()), Resources::new(3_000, 1_000));

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn an_unmeasured_job_holds_its_whole_reservation() {
    // The same machine, but the running job has no use in the sample: it holds all 4000.
    let mut running = ticket("running", 1, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(4_000, 1_000);
    let mut snap = snapshot(vec![running, mine]);
    snap.load.as_mut().unwrap().cpu_used_millicores = Some(3_000);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Budget));
}

#[test]
fn a_job_over_its_reservation_is_charged_nothing_more() {
    let mut running = ticket("running", 1, Class::Light, State::Running);
    running.reservation = Resources::new(1_000, 1_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(5_000, 1_000);
    let mut snap = snapshot(vec![running, mine]);
    let load = snap.load.as_mut().unwrap();
    load.cpu_used_millicores = Some(3_000);
    load.jobs
        .insert(TicketId("running".into()), Resources::new(3_000, 1_000));

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn a_reservation_below_its_class_minimum_is_raised_to_it() {
    let mut running = ticket("running", 1, Class::Light, State::Running);
    running.reservation = Resources::new(6_000, 1_000);
    let mut mine = ticket("me", 2, Class::Heavy, State::Waiting);
    mine.reservation = Resources::new(500, 500);
    let mut snap = snapshot(vec![running, mine]);
    snap.config.class_min.heavy = Resources::new(2_001, 0);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Budget));
}

#[test]
fn an_orphan_whose_holder_is_alive_is_charged_its_reservation() {
    let mut orphan = ticket("orphan", 1, Class::Light, State::Running);
    orphan.liveness = Liveness::Orphaned {
        holder_alive: true,
        this_boot: true,
    };
    orphan.reservation = Resources::new(5_000, 1_000);
    // A living job that reserves nothing, so first-in doesn't apply and the budget decides.
    let mut living = ticket("living", 0, Class::Light, State::Running);
    living.reservation = Resources::new(0, 0);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(4_000, 1_000);
    let mut snap = snapshot(vec![living, orphan, mine]);
    snap.config.class_min.light = Resources::new(0, 0);
    // Its use in the sample is ignored: an orphan counts as using nothing.
    snap.load
        .as_mut()
        .unwrap()
        .jobs
        .insert(TicketId("orphan".into()), Resources::new(5_000, 1_000));

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Budget));
}

#[test]
fn an_orphan_whose_holder_is_gone_is_not_charged() {
    let mut orphan = ticket("orphan", 1, Class::Light, State::Running);
    orphan.liveness = Liveness::Orphaned {
        holder_alive: false,
        this_boot: true,
    };
    orphan.reservation = Resources::new(5_000, 1_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(4_000, 1_000);

    assert_eq!(decide(&snapshot(vec![orphan, mine]), &me()), admitted());
}

#[test]
fn an_orphan_from_another_boot_is_not_charged() {
    let mut orphan = ticket("orphan", 1, Class::Light, State::Running);
    orphan.liveness = Liveness::Orphaned {
        holder_alive: true,
        this_boot: false,
    };
    orphan.reservation = Resources::new(5_000, 1_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(4_000, 1_000);

    assert_eq!(decide(&snapshot(vec![orphan, mine]), &me()), admitted());
}

#[test]
fn an_unreadable_ticket_is_charged_the_heavy_minimum() {
    let mut mine = ticket("me", 1, Class::Light, State::Waiting);
    mine.reservation = Resources::new(4_000, 1_000);
    let mut snap = snapshot(vec![mine]);
    snap.config.class_min.heavy = Resources::new(4_001, 0);
    snap.unreadable = 1;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Budget));
}

#[test]
fn first_in_admits_a_job_bigger_than_the_free_budget_when_nothing_is_charged() {
    // A reservation is capped at capacity, so on an idle machine anything fits. First-in matters
    // when load from outside maitred takes part of the budget.
    let mut mine = ticket("me", 1, Class::Heavy, State::Waiting);
    mine.reservation = Resources::new(64_000, 64_000);
    let mut snap = snapshot(vec![mine]);
    snap.load.as_mut().unwrap().cpu_used_millicores = Some(1_000);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Admit(Admission {
            first_in: true,
            ..Admission::default()
        })
    );
}

#[test]
fn first_in_never_waives_memory() {
    // Outside load leaves 4 GB; a job that reserves 12 GB waits, even on an empty queue, because
    // over-committing memory is what takes a machine down.
    let mut mine = ticket("me", 1, Class::Heavy, State::Waiting);
    mine.reservation = Resources::new(1_000, 12_000);
    let mut snap = snapshot(vec![mine]);
    snap.load.as_mut().unwrap().mem_available_mb = Some(4_000);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Budget));
}

#[test]
fn a_drained_ticket_whose_memory_does_not_fit_still_waits() {
    let mut lapsed = ticket("me", 1, Class::Light, State::Waiting);
    lapsed.reservation = Resources::new(1_000, 12_000);
    lapsed.waited_ms = 600_000 + 600_001;
    let mut snap = snapshot(vec![lapsed]);
    snap.load.as_mut().unwrap().mem_available_mb = Some(4_000);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Budget));
}

#[test]
fn first_in_admits_only_the_first_allowed_ticket() {
    let mut first = ticket("first", 1, Class::Heavy, State::Waiting);
    first.reservation = Resources::new(64_000, 64_000);
    let mut snap = snapshot(vec![first, ticket("me", 2, Class::Light, State::Waiting)]);
    snap.load.as_mut().unwrap().cpu_used_millicores = Some(1_000);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Wait(Reason::BehindFirstIn(TicketId("first".into())))
    );
}

#[test]
fn first_in_does_not_apply_once_a_ticket_was_admitted_earlier_in_the_walk() {
    let mut big = ticket("me", 2, Class::Light, State::Waiting);
    big.reservation = Resources::new(64_000, 64_000);
    let snap = snapshot(vec![ticket("small", 1, Class::Light, State::Waiting), big]);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Budget));
}

#[test]
fn a_charged_orphan_does_not_block_first_in() {
    let mut orphan = ticket("orphan", 1, Class::Light, State::Running);
    orphan.liveness = Liveness::Orphaned {
        holder_alive: true,
        this_boot: true,
    };
    let mut mine = ticket("me", 2, Class::Heavy, State::Waiting);
    mine.reservation = Resources::new(64_000, 1_000);
    let snap = snapshot(vec![orphan, mine]);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Admit(Admission {
            first_in: true,
            ..Admission::default()
        })
    );
}

#[test]
fn first_in_never_waives_a_slot() {
    let pool = Pool("test_env".into());
    let mut holder = ticket("holder", 1, Class::Light, State::Running);
    holder.liveness = Liveness::Orphaned {
        holder_alive: false,
        this_boot: true,
    };
    holder.slots.insert(pool.clone(), 1);
    let mut mine = ticket("me", 2, Class::Heavy, State::Waiting);
    mine.reservation = Resources::new(64_000, 64_000);
    mine.pools.insert(pool.clone());
    let mut snap = snapshot(vec![holder, mine]);
    snap.config.pools.insert(pool.clone(), 1);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoSlot(pool)));
}

#[test]
fn first_in_never_waives_settle() {
    let mut mine = ticket("me", 1, Class::Heavy, State::Waiting);
    mine.reservation = Resources::new(64_000, 64_000);
    let mut snap = snapshot(vec![mine]);
    snap.last_heavy = Some(Wall(NOW.0 - 29_999));

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Settle));
}

#[test]
fn a_heavy_ticket_waits_out_settle_after_the_last_heavy_admission() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Heavy, State::Waiting)]);
    snap.last_heavy = Some(Wall(NOW.0 - 29_999));

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Settle));
}

#[test]
fn a_heavy_ticket_is_admitted_once_settle_has_passed() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Heavy, State::Waiting)]);
    snap.last_heavy = Some(Wall(NOW.0 - 30_000));

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn a_heavy_admitted_earlier_in_the_walk_starts_settle() {
    let snap = snapshot(vec![
        ticket("first", 1, Class::Heavy, State::Waiting),
        ticket("me", 2, Class::Heavy, State::Waiting),
    ]);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Settle));
}

#[test]
fn settle_does_not_hold_a_light_ticket() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Light, State::Waiting)]);
    snap.last_heavy = Some(Wall(NOW.0 - 1));

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn no_more_heavy_jobs_than_max_running_heavy() {
    let mut snap = snapshot(vec![
        ticket("a", 1, Class::Heavy, State::Running),
        ticket("b", 2, Class::Heavy, State::Admitted),
        ticket("me", 3, Class::Heavy, State::Waiting),
    ]);
    snap.config.max_running_heavy = 2;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::MaxRunning));
}

#[test]
fn an_orphaned_heavy_job_does_not_count_toward_max_running_heavy() {
    let mut orphan = ticket("orphan", 1, Class::Heavy, State::Running);
    orphan.liveness = Liveness::Orphaned {
        holder_alive: true,
        this_boot: true,
    };
    let mut snap = snapshot(vec![orphan, ticket("me", 2, Class::Heavy, State::Waiting)]);
    snap.config.max_running_heavy = 1;

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn a_quiet_ticket_waits_while_anything_living_is_charged() {
    let mut snap = snapshot(vec![
        ticket("light", 1, Class::Light, State::Running),
        ticket("me", 2, Class::Quiet, State::Waiting),
    ]);
    snap.load.as_mut().unwrap().quiet_streak = 5;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::QuietNeedsIdle));
}

#[test]
fn a_quiet_ticket_waits_for_the_quiet_streak() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Quiet, State::Waiting)]);
    snap.load.as_mut().unwrap().quiet_streak = 1;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::QuietNeedsIdle));
}

#[test]
fn a_quiet_ticket_is_admitted_on_an_idle_machine_with_the_streak() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Quiet, State::Waiting)]);
    snap.load.as_mut().unwrap().quiet_streak = 2;

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn a_quiet_ticket_past_quiet_max_wait_is_admitted_tainted_without_the_streak() {
    let mut mine = ticket("me", 1, Class::Quiet, State::Waiting);
    mine.waited_ms = 3_600_001;
    let mut snap = snapshot(vec![mine]);
    snap.load.as_mut().unwrap().quiet_streak = 0;

    assert_eq!(
        decide(&snap, &me()),
        Decision::Admit(Admission {
            tainted: true,
            ..Admission::default()
        })
    );
}

#[test]
fn a_quiet_ticket_waits_behind_one_admitted_earlier_in_the_walk() {
    let mut snap = snapshot(vec![
        ticket("light", 1, Class::Light, State::Waiting),
        ticket("me", 2, Class::Quiet, State::Waiting),
    ]);
    snap.load.as_mut().unwrap().quiet_streak = 5;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::QuietNeedsIdle));
}

#[test]
fn a_due_quiet_ticket_stops_the_walk_so_the_machine_drains() {
    let mut quiet = ticket("quiet", 1, Class::Quiet, State::Waiting);
    quiet.waited_ms = 3_600_001;
    let snap = snapshot(vec![
        ticket("running", 0, Class::Light, State::Running),
        quiet,
        ticket("me", 2, Class::Light, State::Waiting),
    ]);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Wait(Reason::DrainingFor(TicketId("quiet".into())))
    );
}

#[test]
fn a_due_quiet_ticket_keeps_draining_the_machine_until_it_gets_in() {
    let mut quiet = ticket("quiet", 1, Class::Quiet, State::Waiting);
    quiet.waited_ms = 365 * 24 * 3_600_000;
    let snap = snapshot(vec![
        ticket("running", 0, Class::Light, State::Running),
        quiet,
        ticket("me", 2, Class::Light, State::Waiting),
    ]);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Wait(Reason::DrainingFor(TicketId("quiet".into())))
    );
}

#[test]
fn a_quiet_ticket_not_yet_due_is_passed_over() {
    let mut quiet = ticket("quiet", 1, Class::Quiet, State::Waiting);
    quiet.waited_ms = 3_600_000;
    let snap = snapshot(vec![
        ticket("running", 0, Class::Light, State::Running),
        quiet,
        ticket("me", 2, Class::Light, State::Waiting),
    ]);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn an_aged_ticket_that_does_not_fit_holds_its_reservation_out_of_the_budget() {
    // 4000 free. The aged ticket wants 6000 and doesn't fit, but holds what it would take: the
    // 3000 behind it, which would fit on its own, has to wait.
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut aged = ticket("aged", 1, Class::Light, State::Waiting);
    aged.reservation = Resources::new(6_000, 1_000);
    aged.waited_ms = 600_001;
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);

    assert_eq!(
        decide(&snapshot(vec![running, aged, mine]), &me()),
        Decision::Wait(Reason::Budget)
    );
}

#[test]
fn a_ticket_not_yet_aged_holds_nothing() {
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut young = ticket("young", 1, Class::Light, State::Waiting);
    young.reservation = Resources::new(6_000, 1_000);
    young.waited_ms = 600_000;
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);

    assert_eq!(
        decide(&snapshot(vec![running, young, mine]), &me()),
        admitted()
    );
}

#[test]
fn a_ticket_whose_hold_lapsed_drains_the_machine() {
    // Past aging_hold it stops holding budget and the walk stops at it, so nothing new starts until
    // nothing living is charged and it gets in first.
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut lapsed = ticket("lapsed", 1, Class::Light, State::Waiting);
    lapsed.reservation = Resources::new(6_000, 1_000);
    lapsed.waited_ms = 600_000 + 600_001;
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);

    assert_eq!(
        decide(&snapshot(vec![running, lapsed, mine]), &me()),
        Decision::Wait(Reason::DrainingFor(TicketId("lapsed".into())))
    );
}

#[test]
fn a_ticket_exactly_at_the_end_of_its_hold_still_holds_and_does_not_drain() {
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut held = ticket("held", 1, Class::Light, State::Waiting);
    held.reservation = Resources::new(6_000, 1_000);
    held.waited_ms = 600_000 + 600_000;
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);

    assert_eq!(
        decide(&snapshot(vec![running, held, mine]), &me()),
        Decision::Wait(Reason::Budget)
    );
}

#[test]
fn a_ticket_whose_hold_lapsed_gets_in_first_once_the_machine_has_drained() {
    let mut lapsed = ticket("me", 1, Class::Light, State::Waiting);
    lapsed.reservation = Resources::new(8_000, 1_000);
    lapsed.waited_ms = 600_000 + 600_001;
    let mut snap = snapshot(vec![lapsed]);
    snap.load.as_mut().unwrap().cpu_used_millicores = Some(3_000);

    assert_eq!(
        decide(&snap, &me()),
        Decision::Admit(Admission {
            first_in: true,
            ..Admission::default()
        })
    );
}

#[test]
fn an_aged_ticket_its_class_rules_keep_out_holds_nothing() {
    let mut running = ticket("running", 0, Class::Heavy, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut aged = ticket("aged", 1, Class::Heavy, State::Waiting);
    aged.reservation = Resources::new(3_000, 1_000);
    aged.waited_ms = 600_001;
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);
    let mut snap = snapshot(vec![running, aged, mine]);
    snap.config.max_running_heavy = 1;

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn waiters_are_walked_in_seq_order_not_snapshot_order() {
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut ahead = ticket("ahead", 1, Class::Light, State::Waiting);
    ahead.reservation = Resources::new(3_000, 1_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);

    assert_eq!(
        decide(&snapshot(vec![mine, running, ahead]), &me()),
        Decision::Wait(Reason::Budget)
    );
}

#[test]
fn equal_seqs_are_ordered_by_ticket_id() {
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut ahead = ticket("a-ahead", 1, Class::Light, State::Waiting);
    ahead.reservation = Resources::new(3_000, 1_000);
    let mut mine = ticket("me", 1, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);

    assert_eq!(
        decide(&snapshot(vec![mine, running, ahead]), &me()),
        Decision::Wait(Reason::Budget)
    );
}

#[test]
fn a_stalled_waiter_neither_takes_budget_nor_blocks_anyone() {
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut stalled = ticket("stalled", 1, Class::Light, State::Waiting);
    stalled.reservation = Resources::new(3_000, 1_000);
    stalled.last_pass = Wall(NOW.0 - 10_001);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);

    assert_eq!(
        decide(&snapshot(vec![running, stalled, mine]), &me()),
        admitted()
    );
}

#[test]
fn a_waiter_whose_last_pass_is_exactly_at_the_stall_limit_is_not_stalled() {
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut slow = ticket("slow", 1, Class::Light, State::Waiting);
    slow.reservation = Resources::new(3_000, 1_000);
    slow.last_pass = Wall(NOW.0 - 10_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);

    assert_eq!(
        decide(&snapshot(vec![running, slow, mine]), &me()),
        Decision::Wait(Reason::Budget)
    );
}

#[test]
fn nobody_is_stalled_until_the_stall_limit_has_passed_since_the_machine_woke() {
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut sleeper = ticket("sleeper", 1, Class::Light, State::Waiting);
    sleeper.reservation = Resources::new(3_000, 1_000);
    sleeper.last_pass = Wall(NOW.0 - 3_600_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);
    let mut snap = snapshot(vec![running, sleeper, mine]);
    snap.load.as_mut().unwrap().woke_at = Some(Wall(NOW.0 - 9_999));

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::Budget));
}

#[test]
fn a_ticket_waits_when_every_slot_of_its_pool_is_named() {
    let pool = Pool("test_env".into());
    let mut holder = ticket("holder", 1, Class::Light, State::Running);
    holder.slots.insert(pool.clone(), 1);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.pools.insert(pool.clone());
    let mut snap = snapshot(vec![holder, mine]);
    snap.config.pools.insert(pool.clone(), 1);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoSlot(pool)));
}

#[test]
fn a_ticket_gets_a_free_slot() {
    let pool = Pool("test_env".into());
    let mut holder = ticket("holder", 1, Class::Light, State::Running);
    holder.slots.insert(pool.clone(), 1);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.pools.insert(pool.clone());
    let mut snap = snapshot(vec![holder, mine]);
    snap.config.pools.insert(pool, 2);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn a_slot_taken_on_paper_earlier_in_the_walk_is_not_free() {
    let pool = Pool("test_env".into());
    let mut ahead = ticket("ahead", 1, Class::Light, State::Waiting);
    ahead.pools.insert(pool.clone());
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.pools.insert(pool.clone());
    let mut snap = snapshot(vec![ahead, mine]);
    snap.config.pools.insert(pool.clone(), 1);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoSlot(pool)));
}

#[test]
fn a_slot_named_by_a_ticket_out_of_any_charge_is_still_not_free() {
    // "A slot no ticket names": an orphan whose holder is gone still names its slot.
    let pool = Pool("test_env".into());
    let mut orphan = ticket("orphan", 1, Class::Light, State::Running);
    orphan.liveness = Liveness::Orphaned {
        holder_alive: false,
        this_boot: false,
    };
    orphan.slots.insert(pool.clone(), 1);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.pools.insert(pool.clone());
    let mut snap = snapshot(vec![orphan, mine]);
    snap.config.pools.insert(pool.clone(), 1);

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NoSlot(pool)));
}

#[test]
fn a_ticket_ahead_that_fills_the_budget_on_paper_keeps_me_out() {
    let mut ahead = ticket("ahead", 1, Class::Light, State::Waiting);
    ahead.reservation = Resources::new(6_000, 1_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);

    assert_eq!(
        decide(&snapshot(vec![ahead, mine]), &me()),
        Decision::Wait(Reason::Budget)
    );
}

#[test]
fn a_reservation_is_capped_at_capacity() {
    // A request bigger than the machine is charged as the whole machine, so first-in can let it in
    // and it doesn't hold more than exists.
    let mut aged = ticket("aged", 1, Class::Light, State::Waiting);
    aged.reservation = Resources::new(64_000, 1_000);
    aged.waited_ms = 600_001;
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(0, 0);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(0, 0);
    let mut snap = snapshot(vec![running, aged, mine]);
    snap.config.class_min.light = Resources::new(0, 0);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn a_running_ticket_is_not_waiting_even_with_no_load_file() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Light, State::Running)]);
    snap.load = None;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::NotWaiting));
}

#[test]
fn a_recreation_exactly_at_the_stall_limit_no_longer_holds_admission() {
    let mut other = ticket("other", 1, Class::Light, State::Running);
    other.recreated_at = Some(Wall(NOW.0 - 10_000));
    let snap = snapshot(vec![other, ticket("me", 2, Class::Light, State::Waiting)]);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn cpu_pressure_exactly_at_the_ceiling_is_not_over_it() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Light, State::Waiting)]);
    snap.load.as_mut().unwrap().cpu_pressure = Some(1.5);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn gpu_use_exactly_at_the_ceiling_is_not_over_it() {
    let mut snap = snapshot(vec![ticket("me", 1, Class::Light, State::Waiting)]);
    snap.load.as_mut().unwrap().gpu_utilization = Some(0.9);

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn stalls_count_again_once_the_stall_limit_has_passed_since_waking() {
    let mut running = ticket("running", 0, Class::Light, State::Running);
    running.reservation = Resources::new(4_000, 1_000);
    let mut sleeper = ticket("sleeper", 1, Class::Light, State::Waiting);
    sleeper.reservation = Resources::new(3_000, 1_000);
    sleeper.last_pass = Wall(NOW.0 - 3_600_000);
    let mut mine = ticket("me", 2, Class::Light, State::Waiting);
    mine.reservation = Resources::new(3_000, 1_000);
    let mut snap = snapshot(vec![running, sleeper, mine]);
    snap.load.as_mut().unwrap().woke_at = Some(Wall(NOW.0 - 10_000));

    assert_eq!(decide(&snap, &me()), admitted());
}

#[test]
fn a_heavy_admitted_earlier_in_the_walk_counts_toward_max_running_heavy() {
    let mut snap = snapshot(vec![
        ticket("first", 1, Class::Heavy, State::Waiting),
        ticket("me", 2, Class::Heavy, State::Waiting),
    ]);
    snap.config.max_running_heavy = 1;
    snap.config.settle_ms = 0;

    assert_eq!(decide(&snap, &me()), Decision::Wait(Reason::MaxRunning));
}

#[test]
fn nothing_starts_beside_a_quiet_job_admitted_earlier_in_the_walk() {
    let mut snap = snapshot(vec![
        ticket("quiet", 1, Class::Quiet, State::Waiting),
        ticket("me", 2, Class::Light, State::Waiting),
    ]);
    snap.load.as_mut().unwrap().quiet_streak = 2;

    assert_eq!(
        decide(&snap, &me()),
        Decision::Wait(Reason::QuietJobRunning)
    );
}
