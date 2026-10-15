//! Both targets publish the same load file from a scenario: the trace's background load plus each running
//! job's declared use (ARCHITECTURE.md, Test kit: Load).

use maitred_scenario::{Amount, MachineConfig, Publisher, Reading, Sample, ms};

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
        &[("a", job(2_000, 1_024)), ("b", job(500, 512))],
        None,
    );

    assert_eq!(load_file.cpu_used_millicores, 3_500);
    assert_eq!(load_file.mem_available_mb, 16_384 - 2_048 - 1_024 - 512);
}

#[test]
fn cpu_used_stops_at_capacity_and_the_excess_shows_as_pressure() {
    let mut publisher = Publisher::new(&machine());
    let sample = Sample::at(ms(200)).background(12_000, 0);

    let load_file = publisher.publish(&sample, &[("a", job(4_000, 0))], None);

    assert_eq!(load_file.cpu_used_millicores, 8_000);
    // 16 cores of demand on 8 cores is 2 runnable tasks per core.
    assert_eq!(load_file.cpu_pressure_permille, 2_000);
}

#[test]
fn available_memory_never_goes_below_zero() {
    let mut publisher = Publisher::new(&machine());
    let sample = Sample::at(ms(200)).background(0, 16_000);

    let load_file = publisher.publish(&sample, &[("a", job(0, 4_096))], None);

    assert_eq!(load_file.mem_available_mb, 0);
}

#[test]
fn each_running_job_is_listed_with_its_declared_use() {
    let mut publisher = Publisher::new(&machine());

    let load_file = publisher.publish(&Sample::at(ms(200)), &[("a", job(2_000, 1_024))], None);

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
