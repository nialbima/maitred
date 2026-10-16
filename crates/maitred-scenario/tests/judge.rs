//! Both targets report what happened as an `Outcome`, and one function judges it against the scenario.

use maitred_scenario::{
    Class, End, Expect, Failure, Job, MachineConfig, Outcome, Scenario, Trigger, ms,
};

fn two_heavy_jobs() -> Scenario {
    Scenario::new("two heavy jobs", MachineConfig::for_tests())
        .arrive(ms(0), Job::new("a", Class::Heavy))
        .arrive(ms(5_000), Job::new("b", Class::Heavy))
}

#[test]
fn an_order_that_held_passes() {
    let scenario = two_heavy_jobs().expect(Expect::admitted_before("a", "b"));
    let outcome = Outcome::new()
        .admitted("a", ms(100))
        .admitted("b", ms(9_000));

    assert_eq!(scenario.judge(&outcome), Ok(()));
}

#[test]
fn an_order_that_was_reversed_fails() {
    let scenario = two_heavy_jobs().expect(Expect::admitted_before("a", "b"));
    let outcome = Outcome::new()
        .admitted("b", ms(5_100))
        .admitted("a", ms(9_000));

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::NotBefore {
            first: "a".into(),
            second: "b".into()
        }])
    );
}

#[test]
fn an_order_whose_first_job_never_started_fails() {
    let scenario = two_heavy_jobs().expect(Expect::admitted_before("a", "b"));
    let outcome = Outcome::new().admitted("b", ms(5_100));

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::NotBefore {
            first: "a".into(),
            second: "b".into()
        }])
    );
}

#[test]
fn a_job_expected_never_to_start_that_started_fails() {
    let scenario = two_heavy_jobs().expect(Expect::never_admitted("b"));
    let outcome = Outcome::new()
        .admitted("a", ms(100))
        .admitted("b", ms(9_000));

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::Admitted("b".into())])
    );
}

#[test]
fn the_expected_victim_with_the_expected_trigger_passes() {
    let scenario = two_heavy_jobs().expect(Expect::killed("b", Trigger::Memory));
    let outcome = Outcome::new()
        .admitted("a", ms(100))
        .admitted("b", ms(9_000))
        .killed("b", Trigger::Memory, ms(12_000));

    assert_eq!(scenario.judge(&outcome), Ok(()));
}

#[test]
fn the_right_victim_killed_for_the_wrong_trigger_fails() {
    let scenario = two_heavy_jobs().expect(Expect::killed("b", Trigger::Memory));
    let outcome = Outcome::new().killed("b", Trigger::Cpu, ms(12_000));

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::NotKilled {
            label: "b".into(),
            trigger: Trigger::Memory
        }])
    );
}

#[test]
fn a_job_expected_to_survive_that_was_killed_fails() {
    let scenario = two_heavy_jobs().expect(Expect::not_killed("a"));
    let outcome = Outcome::new().killed("a", Trigger::Cpu, ms(12_000));

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::Killed("a".into())])
    );
}

#[test]
fn at_most_one_victim_per_breach_fails_on_a_second_kill_for_one_trigger() {
    let scenario = two_heavy_jobs().expect(Expect::at_most_kills(Trigger::Cpu, 1));
    let outcome =
        Outcome::new()
            .killed("b", Trigger::Cpu, ms(12_000))
            .killed("a", Trigger::Cpu, ms(15_000));

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::TooManyKills {
            trigger: Trigger::Cpu,
            allowed: 1,
            killed: 2
        }])
    );
}

#[test]
fn a_record_that_ended_as_expected_passes() {
    let scenario = two_heavy_jobs().expect(Expect::ends("a", End::Completed { status: 0 }));
    let outcome = Outcome::new().ended("a", End::Completed { status: 0 });

    assert_eq!(scenario.judge(&outcome), Ok(()));
}

#[test]
fn a_record_that_ended_differently_fails_and_says_how_it_ended() {
    let scenario = two_heavy_jobs().expect(Expect::ends("a", End::Completed { status: 0 }));
    let outcome = Outcome::new().ended("a", End::Killed(Trigger::Memory));

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::EndedDifferently {
            label: "a".into(),
            expected: End::Completed { status: 0 },
            actual: Some(End::Killed(Trigger::Memory)),
        }])
    );
}

#[test]
fn a_job_with_no_record_fails_an_end_expectation() {
    let scenario = two_heavy_jobs().expect(Expect::ends("a", End::Cancelled));
    let outcome = Outcome::new();

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::EndedDifferently {
            label: "a".into(),
            expected: End::Cancelled,
            actual: None
        }])
    );
}

#[test]
fn a_job_expected_to_start_that_did_passes() {
    let scenario = two_heavy_jobs().expect(Expect::admitted("b"));
    let outcome = Outcome::new().admitted("b", ms(9_000));

    assert_eq!(scenario.judge(&outcome), Ok(()));
}

#[test]
fn a_job_expected_to_start_that_never_did_fails() {
    let scenario = two_heavy_jobs().expect(Expect::admitted("b"));
    let outcome = Outcome::new().admitted("a", ms(100));

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::NotAdmitted("b".into())])
    );
}

#[test]
fn two_jobs_that_ran_one_after_the_other_never_ran_together() {
    let scenario = two_heavy_jobs().expect(Expect::never_together("a", "b"));
    let outcome = Outcome::new()
        .admitted("a", ms(100))
        .ended_at("a", End::Completed { status: 0 }, ms(5_000))
        .admitted("b", ms(5_200));

    assert_eq!(scenario.judge(&outcome), Ok(()));
}

#[test]
fn two_jobs_whose_runs_overlapped_ran_together() {
    let scenario = two_heavy_jobs().expect(Expect::never_together("a", "b"));
    let outcome = Outcome::new()
        .admitted("a", ms(100))
        .admitted("b", ms(5_200))
        .ended_at("a", End::Completed { status: 0 }, ms(6_000));

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::RanTogether {
            first: "a".into(),
            second: "b".into()
        }])
    );
}

#[test]
fn a_job_still_running_at_the_end_of_the_run_overlaps_anything_admitted_after_it() {
    let scenario = two_heavy_jobs().expect(Expect::never_together("a", "b"));
    let outcome = Outcome::new()
        .admitted("a", ms(100))
        .admitted("b", ms(5_200));

    assert_eq!(
        scenario.judge(&outcome),
        Err(vec![Failure::RanTogether {
            first: "a".into(),
            second: "b".into()
        }])
    );
}
