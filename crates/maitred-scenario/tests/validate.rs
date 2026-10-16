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
            Invalid::Contradicts("b".into()),
            // The order cycle (a before b, b before a) is reported after the per-expectation checks.
            Invalid::Contradicts("a".into()),
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

fn two_jobs(name: &str) -> Scenario {
    Scenario::new(name, machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .arrive(ms(2_000), Job::new("b", Class::Light))
        .run_until(secs(60))
}

#[test]
fn an_admitted_by_earlier_than_a_not_admitted_before_on_one_job_is_refused() {
    let scenario = two_jobs("windows that miss")
        .expect(Expect::admitted_by("a", secs(1)))
        .expect(Expect::not_admitted_before("a", secs(2)));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::Contradicts("a".into())])
    );
}

#[test]
fn never_admitted_with_anything_that_needs_the_job_admitted_is_refused() {
    let first_of_a_pair = two_jobs("never, yet first")
        .expect(Expect::never_admitted("a"))
        .expect(Expect::admitted_before("a", "b"));
    let killed = two_jobs("never, yet killed")
        .expect(Expect::never_admitted("a"))
        .expect(Expect::killed("a", maitred_scenario::Trigger::Memory));

    assert_eq!(
        first_of_a_pair.validate(),
        Err(vec![Invalid::Contradicts("a".into())])
    );
    assert_eq!(
        killed.validate(),
        Err(vec![Invalid::Contradicts("a".into())])
    );
}

#[test]
fn a_kill_and_at_most_no_kills_for_its_trigger_are_refused() {
    let scenario = two_jobs("killed, yet no kills")
        .expect(Expect::killed("a", maitred_scenario::Trigger::Memory))
        .expect(Expect::at_most_kills(maitred_scenario::Trigger::Memory, 0));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::Contradicts("a".into())])
    );
}

#[test]
fn an_order_its_jobs_windows_make_impossible_is_refused() {
    let scenario = two_jobs("a must come first, but can't")
        .expect(Expect::admitted_before("a", "b"))
        .expect(Expect::not_admitted_before("a", secs(5)))
        .expect(Expect::admitted_by("b", secs(3)));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::Contradicts("a".into())])
    );
}

#[test]
fn a_time_named_past_the_last_event_needs_an_end() {
    let scenario = Scenario::new("a window past every event", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .expect(Expect::not_admitted_before("a", secs(60)));

    assert_eq!(scenario.validate(), Err(vec![Invalid::NeedsAnEnd]));
}

#[test]
fn never_together_with_a_job_never_admitted_is_refused_as_unable_to_fail() {
    let scenario = two_jobs("together with nothing")
        .expect(Expect::never_admitted("a"))
        .expect(Expect::never_together("a", "b"));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::NeverFails("a".into())])
    );
}

#[test]
fn an_admitted_by_before_the_job_arrives_is_refused() {
    let scenario = two_jobs("by a time before arrival").expect(Expect::admitted_by("b", secs(1)));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::Contradicts("b".into())])
    );
}

#[test]
fn a_cycle_of_admitted_before_of_any_length_is_refused() {
    let scenario = Scenario::new("a cycle of three", machine())
        .arrive(ms(0), Job::new("a", Class::Light))
        .arrive(ms(1_000), Job::new("b", Class::Light))
        .arrive(ms(2_000), Job::new("c", Class::Light))
        .expect(Expect::admitted_before("a", "b"))
        .expect(Expect::admitted_before("b", "c"))
        .expect(Expect::admitted_before("c", "a"));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::Contradicts("a".into())])
    );
}

#[test]
fn at_most_together_that_never_admitted_jobs_make_unbreakable_is_refused() {
    let scenario = two_jobs("a bound over a job that never runs")
        .expect(Expect::never_admitted("a"))
        .expect(Expect::at_most_together(1, &["a", "b"]));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::NeverFails("a".into())])
    );
}

#[test]
fn one_killed_by_and_at_most_no_kills_for_its_trigger_are_refused() {
    let scenario = two_jobs("one killed, yet no kills")
        .expect(Expect::one_killed_by(
            &["a", "b"],
            maitred_scenario::Trigger::Memory,
            secs(1),
        ))
        .expect(Expect::at_most_kills(maitred_scenario::Trigger::Memory, 0))
        .run_until(secs(2));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::Contradicts("a".into())])
    );
}

#[test]
fn one_killed_by_naming_a_job_twice_is_refused() {
    let scenario = two_jobs("a twice")
        .expect(Expect::one_killed_by(
            &["a", "a"],
            maitred_scenario::Trigger::Memory,
            secs(1),
        ))
        .run_until(secs(2));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::DuplicateLabel("a".into())])
    );
}

#[test]
fn one_killed_by_a_time_after_the_run_stops_is_refused() {
    let scenario = two_jobs("killed after the end")
        .expect(Expect::one_killed_by(
            &["a", "b"],
            maitred_scenario::Trigger::Memory,
            secs(5),
        ))
        .run_until(secs(3));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::StopsBeforeExpectation {
            until: secs(3),
            named: secs(5)
        }])
    );
}

#[test]
fn a_job_alternating_with_no_period_is_refused() {
    let scenario = Scenario::new("no period", machine()).arrive(
        ms(0),
        Job::new("a", Class::Light).alternates_with(ms(0), 100, 64),
    );

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::NoPeriod("a".into())])
    );
}

#[test]
fn a_storm_that_ends_before_it_starts_is_refused() {
    let scenario = two_jobs("backwards storm").storm(secs(3), secs(2), secs(2));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::StormEndsBeforeItStarts {
            at: secs(3),
            until: secs(2)
        }])
    );
}

#[test]
fn a_storm_no_slower_than_the_poll_interval_slows_nothing_and_is_refused() {
    let scenario = two_jobs("gentle storm").storm(secs(1), secs(3), ms(200));

    assert_eq!(
        scenario.validate(),
        Err(vec![Invalid::StormSlowsNothing {
            holder_every: ms(200)
        }])
    );
}
