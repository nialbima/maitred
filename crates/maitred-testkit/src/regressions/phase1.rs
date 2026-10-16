//! Phase 1: the pure core. These scenarios run on the simulator (`sim` in FINDINGS.md).
//!
//! The machine is `MachineConfig::for_tests()`: 8 cores, 16384 MB, a 1024 MB memory floor, a light class
//! default of 1000 millicores and 512 MB, a 200 ms poll and sample interval, 3 s aging, a 3 s aging hold and a
//! 1 s stall limit. Each scenario's comment works its numbers from "Inside `decide`" (ARCHITECTURE.md).

use maitred_scenario::{Class, Expect, Job, MachineConfig, Reading, Sample, Scenario, ms, secs};

/// The admission scenarios of phase 1.
#[must_use]
pub fn admission() -> Vec<Scenario> {
    vec![
        excess_use_isnt_given_back(),
        unmeasured_job_holds_its_whole_reservation(),
        missing_memory_reading_falls_back_to_one_at_a_time(),
        job_that_can_never_fit_doesnt_hold_the_queue_for_ever(),
    ]
}

/// `a` reserves 1024 MB but uses 4096. Outside load holds 9000 MB, so while `a` runs, 16384 - 9000 - 4096 =
/// 3288 MB is available, and `a` is charged max(0, 1024 - 4096) = 0 more. `b`'s 4000 MB doesn't fit until `a`
/// ends. Charging `a` its reservation instead of its use would leave 16384 - 9000 - 1024 = 6360 MB, and `b`
/// would start beside it.
fn excess_use_isnt_given_back() -> Scenario {
    Scenario::new(
        "a job using more memory than it reserved gives none of the excess back",
        MachineConfig::for_tests(),
    )
    .checks("r2-admission A1")
    .load(Sample::at(ms(0)).background(0, 9_000))
    .arrive(
        ms(0),
        Job::new("a", Class::Light)
            .reserves(1_000, 1_024)
            .uses(500, 4_096)
            .runs_for(secs(4)),
    )
    .arrive(
        ms(1_000),
        Job::new("b", Class::Light).reserves(1_000, 4_000),
    )
    .expect(Expect::admitted_before("a", "b"))
    .expect(Expect::never_together("a", "b"))
    .expect(Expect::admitted("b"))
}

/// `a` reserves 6000 MB and uses 500, but no figure for it reaches the sample, so `used` is 0 and it is
/// charged all 6000. Free memory is 16384 - 500 - 6000 = 9884 MB, short of `b`'s 10000. Charging `a` only
/// what it uses would leave 16384 - 500 - 5500 = 10384, and `b` would start beside it.
fn unmeasured_job_holds_its_whole_reservation() -> Scenario {
    Scenario::new(
        "a job whose use isn't measured is charged its whole reservation",
        MachineConfig::for_tests(),
    )
    .checks("r2-admission B1")
    .arrive(
        ms(0),
        Job::new("a", Class::Light)
            .reserves(1_000, 6_000)
            .uses(500, 500)
            .unmeasured()
            .runs_for(secs(4)),
    )
    .arrive(
        ms(1_000),
        Job::new("b", Class::Light).reserves(1_000, 10_000),
    )
    .expect(Expect::admitted_before("a", "b"))
    .expect(Expect::never_together("a", "b"))
    .expect(Expect::admitted("b"))
}

/// The sampler runs but never reads memory, a critical reading. Admission falls back to the plain lock: one
/// job at a time, in queue order, never a permanent wait.
fn missing_memory_reading_falls_back_to_one_at_a_time() -> Scenario {
    Scenario::new(
        "a missing memory reading admits one job at a time instead of none",
        MachineConfig::for_tests(),
    )
    .checks("r2-platforms A5")
    .load(Sample::at(ms(0)).missing(Reading::Memory))
    .arrive(ms(0), Job::new("a", Class::Light).runs_for(secs(2)))
    .arrive(ms(500), Job::new("b", Class::Light))
    .expect(Expect::admitted("a"))
    .expect(Expect::admitted("b"))
    .expect(Expect::admitted_before("a", "b"))
    .expect(Expect::never_together("a", "b"))
}

/// `a` reserves 15000 MB, but outside load holds 2000, so at most 14384 MB is ever available and `a` can
/// never fit. After its aging (3 s) and aging hold (3 s more), its hold lapses. `b`, a small job behind it,
/// must still start: a job that can never fit mustn't hold the queue for ever.
///
/// As ARCHITECTURE rev 106 reads, a lapsed hold stops the walk and waits for first-in, which never waives
/// memory, so `b` would wait for ever. This scenario states the outcome the finding asks for.
fn job_that_can_never_fit_doesnt_hold_the_queue_for_ever() -> Scenario {
    Scenario::new(
        "a job that can never fit doesn't keep the jobs behind it out for ever",
        MachineConfig::for_tests(),
    )
    .checks("r3-admission A17")
    .load(Sample::at(ms(0)).background(0, 2_000))
    .arrive(ms(0), Job::new("a", Class::Light).reserves(1_000, 15_000))
    .arrive(ms(500), Job::new("b", Class::Light))
    .expect(Expect::admitted("b"))
}
