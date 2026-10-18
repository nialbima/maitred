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
            Running::new("a", job(2_000, 1_024)),
            Running::new("b", job(500, 512)),
        ],
        None,
    );

    assert_eq!(load_file.cpu_used_millicores, 3_500);
    assert_eq!(load_file.mem_available_mb, 16_384 - 2_048 - 1_024 - 512);
}

#[test]
fn cpu_used_stops_at_capacity_and_the_excess_shows_as_runnable_work_per_core() {
    let mut publisher = Publisher::new(&machine());
    let sample = Sample::at(ms(200)).background(12_000, 0);

    let load_file = publisher.publish(&sample, &[Running::new("a", job(4_000, 0))], None);

    assert_eq!(load_file.cpu_used_millicores, 8_000);
    // 16 cores of demand on 8 cores is 2 runnable tasks per core.
    assert_eq!(load_file.cpu_runnable_per_core_permille, 2_000);
}

#[test]
fn available_memory_never_goes_below_zero() {
    let mut publisher = Publisher::new(&machine());
    let sample = Sample::at(ms(200)).background(0, 16_000);

    let load_file = publisher.publish(&sample, &[Running::new("a", job(0, 4_096))], None);

    assert_eq!(load_file.mem_available_mb, 0);
}

#[test]
fn a_jobs_figure_is_what_its_holder_last_measured_not_what_it_uses_now() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(
        &Sample::at(ms(200)),
        &[Running::new("a", job(2_000, 3_000)).holder_passed(ms(150), job(2_000, 1_024))],
        None,
    );

    assert_eq!(load_file.jobs, vec![("a".to_string(), job(2_000, 1_024))]);
    // The machine's own figures see what it uses now.
    assert_eq!(load_file.mem_available_mb, 16_384 - 3_000);
}

#[test]
fn a_holders_last_measurement_stays_in_the_sample_however_old() {
    let mut publisher = Publisher::new(&machine());

    // Ten sample intervals since the holder's last pass (Nick, MT-046: until it writes the next one).
    let load_file = publisher.publish(
        &Sample::at(ms(2_100)),
        &[Running::new("a", job(500, 64)).holder_passed(ms(100), job(500, 64))],
        None,
    );

    assert_eq!(load_file.jobs, vec![("a".to_string(), job(500, 64))]);
    assert_eq!(load_file.holder_passes, vec![("a".to_string(), ms(100))]);
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

    // Stall of 950 thousandths is over the stall kill ceiling of 600.
    let streaks: Vec<u32> = [950, 950, 100, 950]
        .iter()
        .enumerate()
        .map(|(i, stall)| {
            let sample = Sample::at(ms(200 * (i as u64 + 1))).stall(*stall);
            publisher.publish(&sample, &[], None).over_cpu_streak
        })
        .collect();

    assert_eq!(streaks, vec![1, 2, 0, 1]);
}

#[test]
fn after_a_kill_the_cpu_streak_counts_only_samples_that_began_after_the_victim_was_gone() {
    let mut publisher = Publisher::new(&machine());
    let over = |at| Sample::at(ms(at)).stall(950);

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
    publisher.publish(&Sample::at(ms(200)).stall(950), &[], None);

    let woken = publisher.publish(&Sample::at(ms(400)).stall(950).woke_at(ms(350)), &[], None);
    let next = publisher.publish(&Sample::at(ms(600)).stall(950), &[], None);

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
fn a_job_whose_holder_hasnt_passed_loads_the_machine_but_isnt_listed() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(
        &Sample::at(ms(200)),
        &[Running::new("a", job(2_000, 1_024))],
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
            Running::new("a", job(2_000, 0))
                .holder_passed(ms(100), job(2_000, 0))
                .threads(3),
            Running::new("b", job(500, 0))
                .holder_passed(ms(100), job(500, 0))
                .threads(1),
            Running::new("c", job(4_000, 0)).threads(4),
        ],
        None,
    );

    assert_eq!(load_file.maitred_running_threads, 4);
}

#[test]
fn a_samples_interval_begins_at_the_previous_sample_not_a_nominal_length_before() {
    let mut publisher = Publisher::new(&machine());
    let over = |at| Sample::at(ms(at)).stall(950);

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
    let over = |at| Sample::at(ms(at)).stall(950);

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
fn a_missing_stall_reading_counts_toward_no_cpu_streak_but_leaves_the_quiet_streak_alone() {
    let mut publisher = Publisher::new(&machine());
    publisher.publish(&Sample::at(ms(200)), &[], None);

    let quiet_and_unread =
        publisher.publish(&Sample::at(ms(400)).missing(Reading::CpuStall), &[], None);
    let over_but_unread = publisher.publish(
        &Sample::at(ms(600)).stall(950).missing(Reading::CpuStall),
        &[],
        None,
    );

    // The quiet streak reads CPU use, not stall (ARCHITECTURE rev 190).
    assert_eq!(quiet_and_unread.quiet_streak, 2);
    assert_eq!(over_but_unread.over_cpu_streak, 0);
}

#[test]
fn a_missing_cpu_use_reading_counts_toward_no_quiet_streak() {
    let mut publisher = Publisher::new(&machine());
    publisher.publish(&Sample::at(ms(200)), &[], None);

    let unread = publisher.publish(&Sample::at(ms(400)).missing(Reading::Cpu), &[], None);

    assert_eq!(unread.quiet_streak, 0);
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
    let mut publisher = Publisher::new(&machine().on_macos());
    // 40 threads on 8 cores is 5 per core, over the threads kill ceiling of 2, and the jobs saturate the CPU.
    let busy = [Running::new("a", job(8_000, 0))
        .holder_passed(ms(100), job(8_000, 0))
        .threads(40)];
    // The same threads with the CPU not saturated don't count.
    let idle_cpu = [Running::new("a", job(2_000, 0))
        .holder_passed(ms(100), job(2_000, 0))
        .threads(40)];

    let saturated = publisher.publish(&Sample::at(ms(200)), &busy, None);
    let unsaturated = publisher.publish(&Sample::at(ms(400)), &idle_cpu, None);

    assert_eq!(
        (saturated.over_cpu_streak, unsaturated.over_cpu_streak),
        (1, 0)
    );
}

#[test]
fn a_missing_stall_reading_publishes_no_made_up_figure() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(
        &Sample::at(ms(200)).stall(950).missing(Reading::CpuStall),
        &[],
        None,
    );

    assert_eq!(load_file.cpu_stall_permille, Some(0));
}

#[test]
fn a_linux_machine_publishes_the_traces_stall_and_its_jobs_add_none() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(
        &Sample::at(ms(200)).stall(500),
        &[Running::new("a", job(16_000, 0))],
        None,
    );

    assert_eq!(load_file.cpu_stall_permille, Some(500));
    assert_eq!(load_file.cpu_runnable_per_core_permille, 2_000);
}

#[test]
fn a_macos_machine_has_no_stall_figure_whatever_the_trace_says() {
    let mut publisher = Publisher::new(&machine().on_macos());

    let load_file = publisher.publish(&Sample::at(ms(200)).stall(500), &[], None);

    assert_eq!(load_file.cpu_stall_permille, None);
}

#[test]
fn demand_alone_never_starts_the_cpu_streak_on_linux() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(&Sample::at(ms(200)).background(40_000, 0), &[], None);

    assert_eq!(load_file.over_cpu_streak, 0);
}

#[test]
fn on_macos_a_missing_runnable_reading_leaves_the_quiet_streak_alone() {
    let mut publisher = Publisher::new(&machine().on_macos());
    publisher.publish(&Sample::at(ms(200)), &[], None);

    let unread = publisher.publish(
        &Sample::at(ms(400)).missing(Reading::CpuRunnable),
        &[],
        None,
    );

    assert_eq!(unread.quiet_streak, 2);
}

#[test]
fn a_job_counts_its_cpu_use_as_running_threads_in_thousandths() {
    let mut publisher = Publisher::new(&machine().on_macos());
    // 1100 millicores is about 1.1 running threads, not 2; eight such jobs are 8.8 threads on 8 cores.
    let jobs: Vec<Running> = ["a", "b", "c", "d", "e", "f", "g", "h"]
        .into_iter()
        .map(|label| Running::new(label, job(1_100, 0)).holder_passed(ms(100), job(1_100, 0)))
        .collect();

    let load_file = publisher.publish(&Sample::at(ms(200)), &jobs, None);

    assert_eq!(load_file.maitred_running_threads, 9);
    assert_eq!(load_file.over_cpu_streak, 0);
}

#[test]
fn stall_just_over_the_kill_ceiling_counts_as_over() {
    let mut publisher = Publisher::new(&machine());
    // 601 thousandths, just over the stall kill ceiling of 600.
    let load_file = publisher.publish(&Sample::at(ms(200)).stall(601), &[], None);

    assert_eq!(load_file.over_cpu_streak, 1);
}

#[test]
fn on_macos_a_missing_cpu_reading_counts_toward_no_cpu_streak() {
    let mut publisher = Publisher::new(&machine().on_macos());
    let busy = [Running::new("a", job(8_000, 0))
        .holder_passed(ms(100), job(8_000, 0))
        .threads(40)];

    let load_file = publisher.publish(&Sample::at(ms(200)).missing(Reading::Cpu), &busy, None);

    assert_eq!(load_file.over_cpu_streak, 0);
}

#[test]
fn a_gap_in_sampling_longer_than_load_freshness_restarts_every_streak() {
    let mut publisher = Publisher::new(&machine());
    let over = |at| Sample::at(ms(at)).stall(950);

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
    let over = |at| Sample::at(ms(at)).stall(950);

    publisher.publish(&over(200), &[], None);
    let after_short_gap = publisher.publish(&over(700), &[], None);

    assert_eq!(after_short_gap.over_cpu_streak, 2);
}

#[test]
fn every_holder_that_has_passed_is_in_the_sample() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(
        &Sample::at(ms(400)),
        &[
            Running::new("a", job(500, 64)).holder_passed(ms(350), job(500, 64)),
            Running::new("b", job(500, 64)).holder_passed(ms(120), job(500, 64)),
            Running::new("c", job(500, 64)),
        ],
        None,
    );

    assert_eq!(
        load_file.holder_passes,
        vec![("a".to_string(), ms(350)), ("b".to_string(), ms(120))]
    );
}

#[test]
fn the_first_quiet_sample_after_a_gap_isnt_counted_as_quiet() {
    let mut publisher = Publisher::new(&machine());
    publisher.publish(&Sample::at(ms(200)), &[], None);

    // 1000 ms since the last sample is past load_freshness (600 ms).
    let after_gap = publisher.publish(&Sample::at(ms(1_200)), &[], None);
    let next = publisher.publish(&Sample::at(ms(1_400)), &[], None);

    assert_eq!((after_gap.quiet_streak, next.quiet_streak), (0, 1));
}
