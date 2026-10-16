//! One test per rule of ARCHITECTURE.md's "Kill policy", steps 1 (Trigger) and 2 (Choose).

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::{
    ClassMinimums, Config, Liveness, Load, Resources, Snapshot, State, Ticket, TicketId, Wall,
};

const NOW: Wall = Wall(1_000_000_000);

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
        kill_floor_mb: 500,
        mem_critical_mb: 200,
        cpu_ceiling: 1.5,
        gpu_ceiling: 0.9,
        cpu_kill_samples: 3,
        gpu_kill_samples: 3,
        max_running_heavy: 2,
        quiet_streak_needed: 2,
        class_min: ClassMinimums::default(),
        pools: BTreeMap::new(),
    }
}

/// A healthy 8-core, 16 GB machine, sampled just now.
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
        over_cpu_streak: 0,
        over_gpu_streak: 0,
        jobs: BTreeMap::new(),
    }
}

/// A running job of this boot with a live holder, admitted `age_ms` ago.
fn running(id: &str, class: Class, age_ms: u64) -> Ticket {
    Ticket {
        id: TicketId(id.into()),
        seq: 1,
        class,
        state: State::Running,
        liveness: Liveness::Living,
        holder_alive: true,
        reservation: Resources::new(1_000, 1_000),
        pools: BTreeSet::new(),
        slots: BTreeMap::new(),
        waited_ms: 0,
        last_pass: NOW,
        admitted_at: Some(Wall(NOW.0 - age_ms)),
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
        moved: BTreeMap::new(),
        live_stop_markers: BTreeSet::new(),
    }
}

fn id(s: &str) -> TicketId {
    TicketId(s.into())
}

fn short_of_memory(snap: &mut Snapshot) {
    snap.load.as_mut().unwrap().mem_available_mb = Some(499);
}

// Trigger.

#[test]
fn a_healthy_machine_triggers_nothing() {
    assert_eq!(trigger(&snapshot(vec![])), None);
}

#[test]
fn memory_below_the_kill_floor_triggers_a_memory_kill() {
    let mut snap = snapshot(vec![]);
    short_of_memory(&mut snap);

    assert_eq!(trigger(&snap), Some(Trigger::Memory { critical: false }));
}

#[test]
fn memory_exactly_at_the_kill_floor_triggers_nothing() {
    let mut snap = snapshot(vec![]);
    snap.load.as_mut().unwrap().mem_available_mb = Some(500);

    assert_eq!(trigger(&snap), None);
}

#[test]
fn memory_below_the_critical_threshold_is_critical() {
    let mut snap = snapshot(vec![]);
    snap.load.as_mut().unwrap().mem_available_mb = Some(199);

    assert_eq!(trigger(&snap), Some(Trigger::Memory { critical: true }));
}

#[test]
fn memory_exactly_at_the_critical_threshold_is_not_critical() {
    let mut snap = snapshot(vec![]);
    snap.load.as_mut().unwrap().mem_available_mb = Some(200);

    assert_eq!(trigger(&snap), Some(Trigger::Memory { critical: false }));
}

#[test]
fn a_cpu_streak_reaching_its_limit_triggers_a_cpu_kill() {
    let mut snap = snapshot(vec![]);
    snap.load.as_mut().unwrap().over_cpu_streak = 3;

    assert_eq!(trigger(&snap), Some(Trigger::Cpu { critical: false }));
}

#[test]
fn a_cpu_streak_short_of_its_limit_triggers_nothing() {
    let mut snap = snapshot(vec![]);
    snap.load.as_mut().unwrap().over_cpu_streak = 2;

    assert_eq!(trigger(&snap), None);
}

#[test]
fn a_cpu_streak_at_twice_its_limit_is_critical() {
    let mut snap = snapshot(vec![]);
    snap.load.as_mut().unwrap().over_cpu_streak = 6;

    assert_eq!(trigger(&snap), Some(Trigger::Cpu { critical: true }));
}

#[test]
fn a_cpu_streak_just_short_of_twice_its_limit_is_not_critical() {
    let mut snap = snapshot(vec![]);
    snap.load.as_mut().unwrap().over_cpu_streak = 5;

    assert_eq!(trigger(&snap), Some(Trigger::Cpu { critical: false }));
}

#[test]
fn a_gpu_streak_reaching_its_limit_triggers_a_gpu_kill() {
    let mut snap = snapshot(vec![]);
    snap.load.as_mut().unwrap().over_gpu_streak = 3;

    assert_eq!(trigger(&snap), Some(Trigger::Gpu));
}

#[test]
fn a_gpu_streak_short_of_its_limit_triggers_nothing() {
    let mut snap = snapshot(vec![]);
    snap.load.as_mut().unwrap().over_gpu_streak = 2;

    assert_eq!(trigger(&snap), None);
}

#[test]
fn memory_comes_before_cpu_and_gpu() {
    let mut snap = snapshot(vec![]);
    short_of_memory(&mut snap);
    let load = snap.load.as_mut().unwrap();
    load.over_cpu_streak = 9;
    load.over_gpu_streak = 9;

    assert_eq!(trigger(&snap), Some(Trigger::Memory { critical: false }));
}

#[test]
fn cpu_comes_before_gpu() {
    let mut snap = snapshot(vec![]);
    let load = snap.load.as_mut().unwrap();
    load.over_cpu_streak = 3;
    load.over_gpu_streak = 9;

    assert_eq!(trigger(&snap), Some(Trigger::Cpu { critical: false }));
}

#[test]
fn no_load_file_triggers_nothing() {
    let mut snap = snapshot(vec![]);
    snap.load = None;

    assert_eq!(trigger(&snap), None);
}

#[test]
fn the_first_sample_after_a_sleep_triggers_nothing() {
    let mut snap = snapshot(vec![]);
    short_of_memory(&mut snap);
    snap.load.as_mut().unwrap().fresh = false;

    assert_eq!(trigger(&snap), None);
}

#[test]
fn a_stale_load_file_triggers_nothing() {
    let mut snap = snapshot(vec![]);
    short_of_memory(&mut snap);
    snap.load.as_mut().unwrap().sampled_at = Wall(NOW.0 - 6_001);

    assert_eq!(trigger(&snap), None);
}

#[test]
fn nothing_triggers_while_a_live_stop_marker_exists() {
    let mut snap = snapshot(vec![]);
    short_of_memory(&mut snap);
    snap.live_stop_markers.insert(TicketId("stopping".into()));

    assert_eq!(trigger(&snap), None);
}

#[test]
fn nothing_triggers_while_a_ticket_was_just_recreated() {
    let mut recreated = running("recreated", Class::Light, 60_000);
    recreated.recreated_at = Some(Wall(NOW.0 - 9_999));
    let mut snap = snapshot(vec![recreated]);
    short_of_memory(&mut snap);

    assert_eq!(trigger(&snap), None);
}

#[test]
fn a_recreation_older_than_the_stall_limit_no_longer_holds_the_trigger() {
    let mut recreated = running("recreated", Class::Light, 60_000);
    recreated.recreated_at = Some(Wall(NOW.0 - 10_000));
    let mut snap = snapshot(vec![recreated]);
    short_of_memory(&mut snap);

    assert_eq!(trigger(&snap), Some(Trigger::Memory { critical: false }));
}

// Choose.

#[test]
fn the_job_furthest_over_its_memory_is_chosen() {
    let a = running("a", Class::Light, 60_000);
    let b = running("b", Class::Light, 120_000);
    let mut snap = snapshot(vec![a, b]);
    let jobs = &mut snap.load.as_mut().unwrap().jobs;
    jobs.insert(TicketId("a".into()), Resources::new(500, 1_500));
    jobs.insert(TicketId("b".into()), Resources::new(500, 3_000));

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("b"))
    );
}

#[test]
fn the_job_furthest_over_its_cpu_is_chosen() {
    let a = running("a", Class::Light, 60_000);
    let b = running("b", Class::Light, 120_000);
    let mut snap = snapshot(vec![a, b]);
    let jobs = &mut snap.load.as_mut().unwrap().jobs;
    jobs.insert(TicketId("a".into()), Resources::new(4_000, 500));
    jobs.insert(TicketId("b".into()), Resources::new(2_000, 9_000));

    assert_eq!(
        choose_victim(&snap, Trigger::Cpu { critical: false }),
        Some(id("a"))
    );
}

#[test]
fn overuse_is_measured_against_the_class_minimum_when_that_is_larger() {
    // a asked for 1000 but heavy's minimum is 4000, so at 3000 it isn't over; b is.
    let a = running("a", Class::Heavy, 60_000);
    let b = running("b", Class::Light, 120_000);
    let mut snap = snapshot(vec![a, b]);
    snap.config.class_min.heavy = Resources::new(0, 4_000);
    let jobs = &mut snap.load.as_mut().unwrap().jobs;
    jobs.insert(TicketId("a".into()), Resources::new(0, 3_000));
    jobs.insert(TicketId("b".into()), Resources::new(0, 1_001));

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("b"))
    );
}

#[test]
fn a_job_exactly_at_its_reservation_is_not_over_it() {
    // Neither is over, so recency decides: the newer one, a.
    let a = running("a", Class::Heavy, 60_000);
    let b = running("b", Class::Heavy, 120_000);
    let mut snap = snapshot(vec![a, b]);
    let jobs = &mut snap.load.as_mut().unwrap().jobs;
    jobs.insert(TicketId("a".into()), Resources::new(0, 1_000));
    jobs.insert(TicketId("b".into()), Resources::new(0, 1_000));

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("a"))
    );
}

#[test]
fn equal_overuse_goes_to_the_more_recently_admitted() {
    let a = running("a", Class::Light, 60_000);
    let b = running("b", Class::Light, 120_000);
    let mut snap = snapshot(vec![b, a]);
    let jobs = &mut snap.load.as_mut().unwrap().jobs;
    jobs.insert(TicketId("a".into()), Resources::new(0, 2_000));
    jobs.insert(TicketId("b".into()), Resources::new(0, 2_000));

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("a"))
    );
}

#[test]
fn with_nobody_over_the_newest_heavy_job_is_chosen() {
    // Ids sort the other way from admission, so only recency picks "z-new-heavy".
    let snap = snapshot(vec![
        running("a-old-heavy", Class::Heavy, 600_000),
        running("z-new-heavy", Class::Heavy, 60_000),
        running("newest-light", Class::Light, 1_000),
    ]);

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("z-new-heavy"))
    );
}

#[test]
fn with_no_heavy_job_the_newest_light_job_is_chosen() {
    let snap = snapshot(vec![
        running("old-light", Class::Light, 600_000),
        running("new-light", Class::Light, 60_000),
        running("quiet", Class::Quiet, 1_000),
    ]);

    assert_eq!(
        choose_victim(&snap, Trigger::Cpu { critical: false }),
        Some(id("new-light"))
    );
}

#[test]
fn a_quiet_job_is_spared_below_the_critical_threshold() {
    let snap = snapshot(vec![running("quiet", Class::Quiet, 60_000)]);

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        None
    );
}

#[test]
fn a_quiet_job_is_chosen_past_the_critical_memory_threshold() {
    let snap = snapshot(vec![running("quiet", Class::Quiet, 60_000)]);

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: true }),
        Some(id("quiet"))
    );
}

#[test]
fn a_quiet_job_is_chosen_past_the_critical_cpu_streak() {
    let snap = snapshot(vec![running("quiet", Class::Quiet, 60_000)]);

    assert_eq!(
        choose_victim(&snap, Trigger::Cpu { critical: true }),
        Some(id("quiet"))
    );
}

#[test]
fn a_quiet_job_over_its_reservation_is_chosen_like_any_other() {
    let mut snap = snapshot(vec![
        running("quiet", Class::Quiet, 60_000),
        running("heavy", Class::Heavy, 1_000),
    ]);
    snap.load
        .as_mut()
        .unwrap()
        .jobs
        .insert(TicketId("quiet".into()), Resources::new(0, 2_000));

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("quiet"))
    );
}

#[test]
fn gpu_ignores_use_and_takes_the_newest_heavy_job() {
    let mut snap = snapshot(vec![
        running("hungry-light", Class::Light, 1_000),
        running("heavy", Class::Heavy, 60_000),
    ]);
    snap.load.as_mut().unwrap().jobs.insert(
        TicketId("hungry-light".into()),
        Resources::new(8_000, 9_000),
    );

    assert_eq!(choose_victim(&snap, Trigger::Gpu), Some(id("heavy")));
}

#[test]
fn gpu_never_takes_a_quiet_job() {
    let snap = snapshot(vec![running("quiet", Class::Quiet, 60_000)]);

    assert_eq!(choose_victim(&snap, Trigger::Gpu), None);
}

#[test]
fn equal_admission_times_are_broken_by_ticket_id() {
    let snap = snapshot(vec![
        running("b", Class::Heavy, 60_000),
        running("a", Class::Heavy, 60_000),
    ]);

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("a"))
    );
}

#[test]
fn an_orphaned_job_whose_holder_is_alive_is_a_candidate() {
    let mut orphan = running("orphan", Class::Heavy, 60_000);
    orphan.liveness = Liveness::Orphaned { this_boot: true };

    assert_eq!(
        choose_victim(&snapshot(vec![orphan]), Trigger::Memory { critical: false }),
        Some(id("orphan"))
    );
}

#[test]
fn a_job_whose_holder_is_gone_is_never_chosen() {
    let mut lost = running("lost", Class::Heavy, 1_000);
    lost.holder_alive = false;
    let snap = snapshot(vec![lost, running("other", Class::Heavy, 60_000)]);

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("other"))
    );
}

#[test]
fn an_orphan_from_another_boot_is_never_chosen() {
    let mut old = running("old", Class::Heavy, 1_000);
    old.liveness = Liveness::Orphaned { this_boot: false };
    let snap = snapshot(vec![old, running("other", Class::Heavy, 60_000)]);

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("other"))
    );
}

#[test]
fn a_job_not_yet_running_is_never_chosen() {
    let mut admitted = running("admitted", Class::Heavy, 1_000);
    admitted.state = State::Admitted;
    let snap = snapshot(vec![admitted, running("other", Class::Heavy, 60_000)]);

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("other"))
    );
}

#[test]
fn nobody_is_chosen_while_a_live_stop_marker_exists() {
    let mut snap = snapshot(vec![running("heavy", Class::Heavy, 60_000)]);
    snap.live_stop_markers.insert(TicketId("stopping".into()));

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        None
    );
}

#[test]
fn nobody_is_chosen_while_a_ticket_was_just_recreated() {
    let mut heavy = running("heavy", Class::Heavy, 60_000);
    heavy.recreated_at = Some(Wall(NOW.0 - 1));

    assert_eq!(
        choose_victim(&snapshot(vec![heavy]), Trigger::Memory { critical: false }),
        None
    );
}

#[test]
fn no_candidates_means_no_victim() {
    assert_eq!(
        choose_victim(&snapshot(vec![]), Trigger::Memory { critical: true }),
        None
    );
}

#[test]
fn a_job_measured_exactly_at_its_reservation_does_not_go_before_the_recency_order() {
    let light = running("light", Class::Light, 1_000);
    let heavy = running("heavy", Class::Heavy, 60_000);
    let mut snap = snapshot(vec![light, heavy]);
    snap.load
        .as_mut()
        .unwrap()
        .jobs
        .insert(TicketId("light".into()), Resources::new(1_000, 1_000));

    assert_eq!(
        choose_victim(&snap, Trigger::Memory { critical: false }),
        Some(id("heavy"))
    );
}
