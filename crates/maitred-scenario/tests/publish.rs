//! Both targets publish the same load file from a scenario: the trace's background load plus each running
//! job's declared use (ARCHITECTURE.md, Test kit: Load).

use maitred_scenario::{Amount, MachineConfig, Publisher, Reading, Running, Sample, ms};

fn machine() -> MachineConfig {
    // 8 cores, 16 GB; quiet under 100 thousandths of a runnable task per core; CPU kill over 4 per core.
    MachineConfig::for_tests()
}

fn job(cpu_millicores: u32, mem_mb: u32) -> Amount {
    Amount {
        cpu_millicores,
        mem_mb,
    }
}

#[test]
fn running_jobs_add_their_use_to_the_background_load() {
    let mut publisher = Publisher::new(&machine());
    let sample = Sample::at(ms(200)).background(1_000, 2_048);

    let load_file = publisher.publish(
        &sample,
        &[
            Running::measured("a", job(2_000, 1_024)),
            Running::measured("b", job(500, 512)),
        ],
        None,
    );

    assert_eq!(load_file.cpu_used_millicores, 3_500);
    assert_eq!(load_file.mem_available_mb, 16_384 - 2_048 - 1_024 - 512);
}

#[test]
fn cpu_used_stops_at_capacity_and_the_excess_shows_as_pressure() {
    let mut publisher = Publisher::new(&machine());
    let sample = Sample::at(ms(200)).background(12_000, 0);

    let load_file = publisher.publish(&sample, &[Running::measured("a", job(4_000, 0))], None);

    assert_eq!(load_file.cpu_used_millicores, 8_000);
    // 16 cores of demand on 8 cores is 2 runnable tasks per core.
    assert_eq!(load_file.cpu_pressure_permille, 2_000);
}

#[test]
fn available_memory_never_goes_below_zero() {
    let mut publisher = Publisher::new(&machine());
    let sample = Sample::at(ms(200)).background(0, 16_000);

    let load_file = publisher.publish(&sample, &[Running::measured("a", job(0, 4_096))], None);

    assert_eq!(load_file.mem_available_mb, 0);
}

#[test]
fn each_running_job_is_listed_with_its_declared_use() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(
        &Sample::at(ms(200)),
        &[Running::measured("a", job(2_000, 1_024))],
        None,
    );

    assert_eq!(load_file.jobs, vec![("a".to_string(), job(2_000, 1_024))]);
}

#[test]
fn the_quiet_streak_counts_consecutive_quiet_samples_and_restarts_on_a_busy_one() {
    let mut publisher = Publisher::new(&machine());

    let streaks: Vec<u32> = [0, 0, 4_000, 0]
        .iter()
        .enumerate()
        .map(|(i, cpu)| {
            let sample = Sample::at(ms(200 * (i as u64 + 1))).background(*cpu, 0);
            publisher.publish(&sample, &[], None).quiet_streak
        })
        .collect();

    assert_eq!(streaks, vec![1, 2, 0, 1]);
}

#[test]
fn the_cpu_streak_counts_consecutive_samples_over_the_kill_ceiling() {
    let mut publisher = Publisher::new(&machine());

    // 40 cores of demand on 8 is 5 per core, over the ceiling of 4.
    let streaks: Vec<u32> = [40_000, 40_000, 8_000, 40_000]
        .iter()
        .enumerate()
        .map(|(i, cpu)| {
            let sample = Sample::at(ms(200 * (i as u64 + 1))).background(*cpu, 0);
            publisher.publish(&sample, &[], None).over_cpu_streak
        })
        .collect();

    assert_eq!(streaks, vec![1, 2, 0, 1]);
}

#[test]
fn after_a_kill_the_cpu_streak_counts_only_samples_that_began_after_the_victim_was_gone() {
    let mut publisher = Publisher::new(&machine());
    let over = |at| Sample::at(ms(at)).background(40_000, 0);

    publisher.publish(&over(200), &[], None);
    publisher.publish(&over(400), &[], None);
    // The victim is gone at 450. The sample at 600 began at 400, before that, so it doesn't count.
    let straddling = publisher.publish(&over(600), &[], Some(ms(450)));
    let after = publisher.publish(&over(800), &[], Some(ms(450)));

    assert_eq!((straddling.over_cpu_streak, after.over_cpu_streak), (0, 1));
}

#[test]
fn the_first_sample_after_a_sleep_is_not_fresh_and_restarts_every_streak() {
    let mut publisher = Publisher::new(&machine());
    publisher.publish(&Sample::at(ms(200)).background(40_000, 0), &[], None);

    let woken = publisher.publish(
        &Sample::at(ms(400)).background(40_000, 0).woke_at(ms(350)),
        &[],
        None,
    );
    let next = publisher.publish(&Sample::at(ms(600)).background(40_000, 0), &[], None);

    assert_eq!(
        (woken.fresh, woken.over_cpu_streak, woken.woke_at),
        (false, 0, Some(ms(350)))
    );
    assert_eq!(
        (next.fresh, next.over_cpu_streak, next.woke_at),
        (true, 1, Some(ms(350)))
    );
}

#[test]
fn a_reading_the_sample_marks_missing_is_published_as_missing() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(&Sample::at(ms(200)).missing(Reading::Memory), &[], None);

    assert_eq!(load_file.missing, vec![Reading::Memory]);
}

#[test]
fn a_job_whose_use_cant_be_measured_loads_the_machine_but_isnt_listed() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(
        &Sample::at(ms(200)),
        &[Running::unmeasured("a", job(2_000, 1_024))],
        None,
    );

    assert_eq!(load_file.cpu_used_millicores, 2_000);
    assert_eq!(load_file.mem_available_mb, 16_384 - 1_024);
    assert_eq!(load_file.jobs, vec![]);
}

#[test]
fn maitreds_running_threads_sum_the_measured_jobs_threads() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(
        &Sample::at(ms(200)),
        &[
            Running::measured("a", job(2_000, 0)).threads(3),
            Running::measured("b", job(500, 0)).threads(1),
            Running::unmeasured("c", job(4_000, 0)).threads(4),
        ],
        None,
    );

    assert_eq!(load_file.maitred_running_threads, 4);
}

#[test]
fn a_samples_interval_begins_at_the_previous_sample_not_a_nominal_length_before() {
    let mut publisher = Publisher::new(&machine());
    let over = |at| Sample::at(ms(at)).background(40_000, 0);

    publisher.publish(&over(200), &[], None);
    // A gap in sampling: this sample's interval began at 200, before the victim was gone at 1000.
    let after_gap = publisher.publish(&over(2_000), &[], Some(ms(1_000)));

    assert_eq!(
        (after_gap.interval, after_gap.over_cpu_streak),
        (ms(1_800), 0)
    );
}

#[test]
fn a_sample_that_began_exactly_when_the_victim_was_gone_doesnt_count() {
    let mut publisher = Publisher::new(&machine());
    let over = |at| Sample::at(ms(at)).background(40_000, 0);

    publisher.publish(&over(200), &[], None);
    publisher.publish(&over(400), &[], None);
    let at_the_boundary = publisher.publish(&over(600), &[], Some(ms(400)));
    let after = publisher.publish(&over(800), &[], Some(ms(400)));

    assert_eq!(
        (at_the_boundary.over_cpu_streak, after.over_cpu_streak),
        (0, 1)
    );
}

#[test]
fn a_missing_pressure_reading_counts_toward_no_quiet_or_cpu_streak() {
    let mut publisher = Publisher::new(&machine());
    publisher.publish(&Sample::at(ms(200)), &[], None);

    let quiet_but_unread = publisher.publish(
        &Sample::at(ms(400)).missing(Reading::CpuPressure),
        &[],
        None,
    );
    let over_but_unread = publisher.publish(
        &Sample::at(ms(600))
            .background(40_000, 0)
            .missing(Reading::CpuPressure),
        &[],
        None,
    );

    assert_eq!(quiet_but_unread.quiet_streak, 0);
    assert_eq!(over_but_unread.over_cpu_streak, 0);
}

#[test]
fn a_missing_gpu_reading_counts_toward_no_gpu_streak() {
    let mut publisher = Publisher::new(&machine());
    publisher.publish(&Sample::at(ms(200)).gpu(990), &[], None);

    let unread = publisher.publish(
        &Sample::at(ms(400)).gpu(990).missing(Reading::Gpu),
        &[],
        None,
    );

    assert_eq!(unread.over_gpu_streak, 0);
}

#[test]
fn on_macos_the_cpu_streak_counts_maitreds_running_threads_while_the_cpu_is_saturated() {
    let mut publisher = Publisher::new(&machine()).counting_threads_for_the_cpu_streak();
    // 40 threads on 8 cores is 5 per core, over the kill ceiling of 4, and the jobs saturate the CPU.
    let busy = [Running::measured("a", job(8_000, 0)).threads(40)];
    // The same threads with the CPU not saturated don't count.
    let idle_cpu = [Running::measured("a", job(2_000, 0)).threads(40)];

    let saturated = publisher.publish(&Sample::at(ms(200)), &busy, None);
    let unsaturated = publisher.publish(&Sample::at(ms(400)), &idle_cpu, None);

    assert_eq!(
        (saturated.over_cpu_streak, unsaturated.over_cpu_streak),
        (1, 0)
    );
}

#[test]
fn a_missing_pressure_reading_publishes_no_made_up_figure() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(
        &Sample::at(ms(200))
            .background(40_000, 0)
            .missing(Reading::CpuPressure),
        &[],
        None,
    );

    assert_eq!(load_file.cpu_pressure_permille, 0);
}

#[test]
fn a_job_counts_its_cpu_use_as_running_threads_in_thousandths() {
    let mut publisher = Publisher::new(&machine()).counting_threads_for_the_cpu_streak();
    // 1100 millicores is about 1.1 running threads, not 2; eight such jobs are 8.8 threads on 8 cores.
    let jobs: Vec<Running> = ["a", "b", "c", "d", "e", "f", "g", "h"]
        .into_iter()
        .map(|label| Running::measured(label, job(1_100, 0)))
        .collect();

    let load_file = publisher.publish(&Sample::at(ms(200)), &jobs, None);

    assert_eq!(load_file.maitred_running_threads, 9);
    assert_eq!(load_file.over_cpu_streak, 0);
}

#[test]
fn demand_just_over_the_kill_ceiling_counts_as_over() {
    let mut publisher = Publisher::new(&machine());
    // 32007 millicores on 8000 is 4.000875 runnable per core, just over the ceiling of 4.
    let load_file = publisher.publish(&Sample::at(ms(200)).background(32_007, 0), &[], None);

    assert_eq!(load_file.over_cpu_streak, 1);
}

#[test]
fn on_macos_a_missing_cpu_reading_counts_toward_no_cpu_streak() {
    let mut publisher = Publisher::new(&machine()).counting_threads_for_the_cpu_streak();
    let busy = [Running::measured("a", job(8_000, 0)).threads(40)];

    let load_file = publisher.publish(&Sample::at(ms(200)).missing(Reading::Cpu), &busy, None);

    assert_eq!(load_file.over_cpu_streak, 0);
}

#[test]
fn a_gap_in_sampling_longer_than_load_freshness_restarts_every_streak() {
    let mut publisher = Publisher::new(&machine());
    let over = |at| Sample::at(ms(at)).background(40_000, 0);

    publisher.publish(&over(200), &[], None);
    publisher.publish(&over(400), &[], None);
    // load_freshness is 3 intervals (600 ms); this sample comes 10 s after the last.
    let after_gap = publisher.publish(&over(10_400), &[], None);
    let next = publisher.publish(&over(10_600), &[], None);

    assert_eq!((after_gap.over_cpu_streak, next.over_cpu_streak), (0, 1));
}

#[test]
fn a_gap_within_load_freshness_keeps_the_streak() {
    let mut publisher = Publisher::new(&machine());
    let over = |at| Sample::at(ms(at)).background(40_000, 0);

    publisher.publish(&over(200), &[], None);
    let after_short_gap = publisher.publish(&over(700), &[], None);

    assert_eq!(after_short_gap.over_cpu_streak, 2);
}
