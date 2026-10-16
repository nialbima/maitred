//! A scenario that can't be run as written is refused before either target sees it.

use maitred_scenario::{Class, Expect, Fault, Invalid, Job, MachineConfig, Scenario, ms, secs};

fn machine() -> MachineConfig {
    MachineConfig::for_tests()
}

#[test]
fn a_well_formed_scenario_is_accepted() {
    let scenario = Scenario::new("two light jobs fit side by side", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .arrive(ms(1_000), Job::new("b", Class::Light))
        .expect(Expect::admitted_before("a", "b"));

    assert_eq!(scenario.validate(), Ok(()));
}

#[test]
fn two_jobs_with_one_label_are_refused() {
    let scenario = Scenario::new("duplicate label", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .arrive(ms(1_000), Job::new("a", Class::Heavy));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::DuplicateLabel("a".into())])
    );
}

#[test]
fn an_expectation_naming_a_job_that_never_arrives_is_refused() {
    let scenario = Scenario::new("unknown label in an expectation", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .expect(Expect::admitted_before("a", "ghost"));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::UnknownLabel("ghost".into())])
    );
}

#[test]
fn a_fault_naming_a_job_that_never_arrives_is_refused() {
    let scenario = Scenario::new("unknown label in a fault", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .fault(ms(500), Fault::delete_ticket("ghost"));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::UnknownLabel("ghost".into())])
    );
}

#[test]
fn a_fault_on_a_job_before_it_arrives_is_refused() {
    let scenario = Scenario::new("fault before arrival", machine())
        .arrive(ms(2_000), Job::new("a", Class::Light))
        .fault(ms(500), Fault::delete_ticket("a"));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::BeforeArrival {
            label: "a".into(),
            at: ms(500)
        }])
    );
}

#[test]
fn a_job_asking_for_an_undeclared_pool_is_refused() {
    let scenario = Scenario::new("undeclared pool", machine())
        .arrive(ms(0), Job::new("a", Class::Light).slot("no_such_pool"));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::UndeclaredPool {
            label: "a".into(),
            pool: "no_such_pool".into()
        }])
    );
}

#[test]
fn a_job_asking_for_a_declared_pool_is_accepted() {
    let scenario = Scenario::new(
        "declared pool",
        machine().pool("test_env", 2, "TEST_ENV_NUMBER"),
    )
    .arrive(ms(0), Job::new("a", Class::Light).slot("test_env"));

    assert_eq!(scenario.validate(), Ok(()));
}

#[test]
fn a_strict_order_between_arrivals_closer_than_a_poll_interval_is_refused() {
    let machine = machine().poll_interval(ms(200));
    let scenario = Scenario::new("order the queue can't promise", machine)
        .arrive(ms(0), Job::new("a", Class::Heavy))
        .arrive(ms(150), Job::new("b", Class::Heavy))
        .expect(Expect::admitted_before("a", "b"));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::OrderTooClose {
            first: "a".into(),
            second: "b".into(),
            apart: ms(150)
        }])
    );
}

#[test]
fn arrivals_a_full_poll_interval_apart_may_be_strictly_ordered() {
    let machine = machine().poll_interval(ms(200));
    let scenario = Scenario::new("order the queue can promise", machine)
        .arrive(ms(0), Job::new("a", Class::Heavy))
        .arrive(ms(200), Job::new("b", Class::Heavy))
        .expect(Expect::admitted_before("a", "b"));

    assert_eq!(scenario.validate(), Ok(()));
}

#[test]
fn every_problem_is_reported_not_just_the_first() {
    let scenario = Scenario::new("two problems", machine())
        .arrive(ms(0), Job::new("a", Class::Light).slot("no_such_pool"))
        .expect(Expect::admitted_before("a", "ghost"));

    assert_eq!(
        scenario.validate(),
        Err(vec![
            Invalid::UndeclaredPool {
                label: "a".into(),
                pool: "no_such_pool".into()
            },
            Invalid::UnknownLabel("ghost".into()),
        ])
    );
}

#[test]
fn a_machine_sleep_that_ends_after_it_starts_is_accepted() {
    let scenario = Scenario::new("a night's sleep", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .sleep(ms(1_000), ms(9_000));

    assert_eq!(scenario.validate(), Ok(()));
}

#[test]
fn a_machine_sleep_that_ends_before_it_starts_is_refused() {
    let scenario = Scenario::new("a sleep backwards", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .sleep(ms(9_000), ms(1_000));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::SleepEndsBeforeItStarts {
            at: ms(9_000),
            wakes_at: ms(1_000)
        }])
    );
}

#[test]
fn a_jobs_running_threads_default_to_its_cpu_use_in_thousandths() {
    assert_eq!(
        Job::new("a", Class::Light).uses(2_500, 0).threads_milli(),
        2_500
    );
    assert_eq!(Job::new("b", Class::Light).uses(0, 0).threads_milli(), 0);
    assert_eq!(
        Job::new("c", Class::Light)
            .uses(2_500, 0)
            .with_threads(8)
            .threads_milli(),
        8_000
    );
}

#[test]
fn an_at_most_together_that_can_never_fail_is_refused() {
    let scenario = Scenario::new("a bound that can't be broken", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .arrive(ms(1_000), Job::new("b", Class::Light))
        .expect(Expect::at_most_together(2, &["a", "b"]));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::CantFail {
            allowed: 2,
            jobs: 2
        }])
    );
}

#[test]
fn an_at_most_together_naming_one_job_twice_is_refused() {
    let scenario = Scenario::new("a job listed twice", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .arrive(ms(1_000), Job::new("b", Class::Light))
        .arrive(ms(2_000), Job::new("c", Class::Light))
        .expect(Expect::at_most_together(1, &["a", "a", "b"]));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::DuplicateLabel("a".into())])
    );
}

#[test]
fn a_run_that_stops_after_its_last_event_is_accepted() {
    let scenario = Scenario::new("stops in time", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .arrive(ms(1_000), Job::new("b", Class::Light))
        .run_until(secs(10));

    assert_eq!(scenario.validate(), Ok(()));
}

#[test]
fn a_run_that_stops_before_its_last_event_is_refused() {
    let scenario = Scenario::new("stops too soon", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .arrive(ms(5_000), Job::new("b", Class::Light))
        .run_until(secs(2));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::StopsBeforeLastEvent {
            until: secs(2),
            last: ms(5_000)
        }])
    );
}

#[test]
fn a_sampler_outage_is_accepted_open_ended_or_with_an_end_after_its_start() {
    let scenario = Scenario::new("the sampler stops", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .sampler_down(ms(0), None)
        .sampler_down(ms(5_000), Some(ms(6_000)));

    assert_eq!(scenario.validate(), Ok(()));
}

#[test]
fn a_sampler_outage_that_ends_before_it_starts_is_refused() {
    let scenario = Scenario::new("an outage backwards", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .sampler_down(ms(5_000), Some(ms(1_000)));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::OutageEndsBeforeItStarts {
            at: ms(5_000),
            until: ms(1_000)
        }])
    );
}

#[test]
fn never_together_naming_one_job_twice_is_refused() {
    let scenario = Scenario::new("a job never together with itself", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .expect(Expect::never_together("a", "a"));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::DuplicateLabel("a".into())])
    );
}

#[test]
fn expectations_that_contradict_each_other_are_refused() {
    let scenario = Scenario::new("contradictions", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .arrive(ms(1_000), Job::new("b", Class::Light))
        .expect(Expect::never_admitted("a"))
        .expect(Expect::admitted_by("a", ms(500)))
        .expect(Expect::admitted_before("a", "b"))
        .expect(Expect::admitted_before("b", "a"))
        .expect(Expect::killed("b", maitred_scenario::Trigger::Memory))
        .expect(Expect::not_killed("b"))
        .run_until(secs(10));

    assert_eq!(
        scenario.validate(),
        Err(vec![
            Invalid::Contradicts("a".into()),
            Invalid::Contradicts("a".into()),
            Invalid::Contradicts("b".into()),
        ])
    );
}

#[test]
fn a_scenario_expecting_a_job_never_to_start_must_say_when_the_run_stops() {
    let scenario = Scenario::new("never, with no end", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .expect(Expect::never_admitted("a"));

    assert_eq!(scenario.validate(), Err(vec![Invalid::NeedsAnEnd]));
}

#[test]
fn a_run_that_stops_before_a_time_an_expectation_names_is_refused() {
    let scenario = Scenario::new("stops before the expectation can be judged", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .expect(Expect::not_admitted_before("a", secs(60)))
        .run_until(secs(2));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::StopsBeforeExpectation {
            until: secs(2),
            named: secs(60)
        }])
    );
}
