//! A job's use over time, as both targets read it.

use maitred_scenario::{Amount, Class, Job, ms};

#[test]
fn a_job_uses_the_same_all_run_unless_told_otherwise() {
    let job = Job::new("a", Class::Light).uses(500, 1_000);

    assert_eq!(
        [job.uses_at(ms(0)), job.uses_at(ms(10_000))],
        [Amount {
            cpu_millicores: 500,
            mem_mb: 1_000
        }; 2]
    );
}

#[test]
fn an_alternating_job_switches_every_period_of_scenario_time() {
    let job = Job::new("a", Class::Light)
        .uses(500, 3_000)
        .alternates_with(ms(200), 500, 3_100);

    let mems: Vec<u32> = [0, 199, 200, 399, 400, 650]
        .into_iter()
        .map(|at| job.uses_at(ms(at)).mem_mb)
        .collect();

    assert_eq!(mems, [3_000, 3_000, 3_100, 3_100, 3_000, 3_100]);
}
