//! A scenario that can't be run as written is refused before either target sees it.

use maitred_scenario::{Class, Expect, Fault, Invalid, Job, MachineConfig, Scenario, ms};

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
