//! Phase 1: the pure core. These scenarios run on the simulator (`sim` in FINDINGS.md).
//!
//! The machine is `MachineConfig::for_tests()`: 8 cores, 16384 MB, a 1024 MB memory floor, a light class
//! default of 1000 millicores and 512 MB, a 200 ms poll and sample interval, 3 s aging, a 3 s aging hold and a
//! 1 s stall limit. Each scenario's comment works its numbers from "Inside `decide`" (ARCHITECTURE.md).

use maitred_scenario::{
    Class, ClassCase, Expect, Fault, Hook, Job, MachineConfig, MatchExpect, Process, Reading,
    Sample, Scenario, Signal, Trigger, ms, secs,
};

/// The admission scenarios of phase 1.
#[must_use]
pub fn admission() -> Vec<Scenario> {
    vec![
        excess_use_isnt_given_back(),
        unmeasured_job_holds_its_whole_reservation(),
        missing_memory_reading_falls_back_to_one_at_a_time(),
        job_that_can_never_fit_doesnt_hold_the_queue_for_ever(),
        heavy_limit_counts_admissions_on_paper(),
        aged_ticket_kept_out_by_its_class_holds_nothing(),
        orphaned_quiet_job_doesnt_stop_the_queue(),
        quiet_job_waits_for_a_quiet_machine_without_holding_up_others(),
        large_light_job_gets_in_under_steady_small_arrivals(),
        memory_breach_kills_the_job_furthest_over_and_only_one(),
        due_quiet_job_isnt_held_off_by_an_orphan(),
        nobody_jumps_the_queue_after_the_machine_wakes(),
        class_blocked_ticket_doesnt_drain_when_its_hold_lapses(),
        unreadable_ticket_doesnt_stop_the_fallback_for_good(),
        with_no_reliable_load_a_quiet_job_not_yet_due_neither_starts_nor_holds_the_line(),
        with_the_sampler_down_a_due_quiet_job_gets_in_tainted(),
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

/// `a` reserves 15000 MB, but outside load holds 2000, so at most 14384 MB is ever available and `a` can never
/// fit. It ages at 3 s and holds its 15000 MB out of free until its hold lapses at 6 s, so `c`, arriving at 4 s,
/// waits until then. After the lapse, `a` drains the machine only if it "fits once drained" (ARCHITECTURE
/// rev 111, Nick's decision on MT-019): at most 14384 MB, short of 15000. So it doesn't stop the walk, and `b`,
/// arriving at 7 s, starts at once. With the drain applied regardless of fit, `b` would wait for ever.
fn job_that_can_never_fit_doesnt_hold_the_queue_for_ever() -> Scenario {
    Scenario::new(
        "a job that can never fit doesn't keep the jobs behind it out for ever",
        MachineConfig::for_tests(),
    )
    .checks("r3-admission A17")
    .load(Sample::at(ms(0)).background(0, 2_000))
    .arrive(ms(0), Job::new("a", Class::Light).reserves(1_000, 15_000))
    .arrive(ms(4_000), Job::new("c", Class::Light))
    .arrive(ms(7_000), Job::new("b", Class::Light))
    .expect(Expect::not_admitted_before("c", secs(6)))
    .expect(Expect::admitted_by("c", secs(7)))
    .expect(Expect::admitted_by("b", secs(8)))
    .expect(Expect::never_admitted("a"))
    .run_until(secs(10))
}

/// Settle is 0 here, so it can't hide the class limit. `x1` and `x2` (heavy) fill `max_running.heavy` = 2; `x1`
/// ends at 2 s and `x2` runs on. `h1` and `h2` wait behind them on the limit. `h1`'s supervisor is stopped from 1.9 s
/// to 2.5 s (under the 1 s stall limit, so it still counts as waiting), so `h2` is certain to run the first pass
/// after the slot frees. That walk admits `h1` on paper, and the limit "charged or admitted on paper" then counts
/// `x2` and `h1`, so `h2` waits. Counting only the running jobs would let `h2` in ahead of `h1`, which
/// `admitted_before` catches. (Admission is one pass at a time under `admit.lock`, so it can't put three in at once.)
/// Each reserves 2000 millicores, so the budget (8000) never decides it.
fn heavy_limit_counts_admissions_on_paper() -> Scenario {
    let mut machine = MachineConfig::for_tests();
    machine.settle = ms(0);
    let heavy = |label, run_for| {
        Job::new(label, Class::Heavy)
            .reserves(2_000, 1_024)
            .runs_for(run_for)
    };
    Scenario::new(
        "no more heavy jobs run than the class allows, in queue order",
        machine,
    )
    .checks("r2-admission A6")
    .arrive(ms(0), heavy("x1", secs(2)))
    .arrive(ms(200), heavy("x2", secs(10)))
    .arrive(ms(500), heavy("h1", secs(3)))
    .arrive(ms(800), heavy("h2", secs(3)))
    .fault(
        ms(1_900),
        Fault::signal("h1", Process::Supervisor, Signal::Stop),
    )
    .fault(
        ms(2_500),
        Fault::signal("h1", Process::Supervisor, Signal::Cont),
    )
    .expect(Expect::at_most_together(2, &["x1", "x2", "h1", "h2"]))
    .expect(Expect::admitted_before("h1", "h2"))
    .expect(Expect::admitted("h2"))
}

/// `h1` and `h2` (heavy, 2000 millicores each, using 100) run for 6 s; `h3` waits on `max_running.heavy` and
/// is aged by 3.6 s. At 4 s, free CPU is 8000 - 200 used - 2 x (2000 - 100) = 4000 millicores. `h3` is kept out
/// by its class, so it holds nothing, and `l`'s 3000 fits. If `h3` held its 2000, only 2000 would be free, and `l`
/// would wait for a heavy job to end at about 6 s. `l` arrives before `h3`'s hold could lapse (6.6 s).
fn aged_ticket_kept_out_by_its_class_holds_nothing() -> Scenario {
    let heavy = |label| {
        Job::new(label, Class::Heavy)
            .reserves(2_000, 1_024)
            .runs_for(secs(6))
    };
    Scenario::new(
        "a ticket its class keeps out holds no budget, however long it has waited",
        MachineConfig::for_tests(),
    )
    .checks("r2-admission B6")
    .arrive(ms(0), heavy("h1"))
    .arrive(ms(300), heavy("h2"))
    .arrive(ms(600), heavy("h3"))
    .arrive(ms(4_000), Job::new("l", Class::Light).reserves(3_000, 512))
    .expect(Expect::admitted_by("l", ms(5_000)))
}

/// A quiet job starts on the idle machine (two quiet samples), and its supervisor is then killed, leaving a
/// running orphan whose holder is alive. It is charged its reservation, but "its class's barrier doesn't
/// apply", so a light job arriving after it starts within a few passes rather than when the orphan ends.
fn orphaned_quiet_job_doesnt_stop_the_queue() -> Scenario {
    Scenario::new(
        "an orphaned quiet job doesn't stop the queue",
        MachineConfig::for_tests(),
    )
    .checks("r2-admission B8")
    .arrive(ms(0), Job::new("q", Class::Quiet).runs_for(secs(20)))
    .fault(
        ms(1_000),
        Fault::signal("q", Process::Supervisor, Signal::Kill),
    )
    .arrive(ms(1_500), Job::new("l", Class::Light))
    .expect(Expect::admitted_before("q", "l"))
    .expect(Expect::admitted_by("l", ms(2_500)))
}

/// Outside load demands 4000 millicores, half a runnable task per core, over the quiet threshold, so the quiet
/// streak stays 0. `q` isn't allowed (it needs the streak) until it has waited past `quiet_max_wait` (5 s), and
/// first-in never waives a class rule, so it doesn't start early. Waiting at the head of the queue before it is
/// due, it doesn't stop the walk, so `l` starts at once. Once due, the machine drains for it and it gets in.
fn quiet_job_waits_for_a_quiet_machine_without_holding_up_others() -> Scenario {
    Scenario::new(
        "a quiet job neither starts on a busy machine nor holds others up before it is due",
        MachineConfig::for_tests(),
    )
    .checks("r2-admission A3")
    .load(Sample::at(ms(0)).background(4_000, 0))
    .arrive(ms(0), Job::new("q", Class::Quiet))
    .arrive(ms(500), Job::new("l", Class::Light))
    .expect(Expect::admitted_by("l", ms(1_500)))
    .expect(Expect::not_admitted_before("q", secs(5)))
    .expect(Expect::admitted("q"))
}

/// Small light jobs, each reserving 4000 MB and using 64, arrive every 500 ms from 0 to 11.5 s and run for 2 s,
/// so four run at once. Each is charged 4000 - 64 = 3936 MB. `big` (8000 MB) arrives at 1.1 s, with three
/// running: free is 16384 - 3 x 64 - 3 x 3936 = 4384, so it doesn't fit, and it never fits on its own while the
/// arrivals go on (at most 4320 is free between them). Once aged (4.1 s), it holds its 8000 MB out of free, so
/// the small ones fill only around it and it starts as their memory comes free, by 8 s. Without the hold, it
/// waits until the arrivals stop, after 12 s.
fn large_light_job_gets_in_under_steady_small_arrivals() -> Scenario {
    let mut scenario = Scenario::new(
        "a large job gets in while small ones keep arriving",
        MachineConfig::for_tests(),
    )
    .checks("r2-admission A5")
    .arrive(
        ms(1_100),
        Job::new("big", Class::Light).reserves(1_000, 8_000),
    );
    for i in 0..24_u64 {
        let label = format!("s{i}");
        scenario = scenario.arrive(
            ms(500 * i),
            Job::new(&label, Class::Light)
                .reserves(250, 4_000)
                .runs_for(secs(2)),
        );
    }
    scenario
        .expect(Expect::not_admitted_before("big", ms(4_100)))
        .expect(Expect::admitted_by("big", secs(8)))
}

/// `h` (heavy, reserving 2048 MB, using 1000) and `l` (light, reserving 1000 MB, using 1100) run. At 3 s, outside
/// load takes 14500 MB, so available memory is 16384 - 14500 - 2100, below zero and below `kill_floor` (512).
/// The victim is "the candidate furthest over its reservation on that resource": `l`, by 100 MB; `h` is under.
/// Once `l` is gone, 16384 - 14500 - 1000 = 884 MB is available, over the floor, so the breach costs one job.
fn memory_breach_kills_the_job_furthest_over_and_only_one() -> Scenario {
    Scenario::new(
        "a memory breach kills the job furthest over its reservation, and only one",
        MachineConfig::for_tests(),
    )
    .checks("r2-admission B7")
    .load(Sample::at(ms(0)))
    .load(Sample::at(secs(3)).background(0, 14_500))
    .arrive(
        ms(0),
        Job::new("h", Class::Heavy)
            .reserves(2_000, 2_048)
            .uses(100, 1_000)
            .runs_for(secs(10)),
    )
    .arrive(
        ms(1_000),
        Job::new("l", Class::Light)
            .reserves(500, 1_000)
            .uses(100, 1_100)
            .runs_for(secs(10)),
    )
    .expect(Expect::killed("l", Trigger::Memory))
    .expect(Expect::not_killed("h"))
    .expect(Expect::at_most_kills(Trigger::Memory, 1))
}

/// Outside load demands 4000 millicores, half a runnable task per core, over the quiet threshold, so the quiet
/// streak stays 0 and `q` isn't allowed until it is due, after `quiet_max_wait` (5 s, so at 6 s). `o` runs for a
/// long time and its supervisor is killed, leaving a running orphan whose holder is alive. When `q` is due, the
/// machine drains for it, and a quiet job is allowed when "no living ticket [is] charged". An orphan isn't living,
/// so `q` gets in at once rather than when the orphan ends. `l`, queued behind `q` while it drains, then runs once
/// `q` (1 s) has finished, instead of waiting out the orphan too.
fn due_quiet_job_isnt_held_off_by_an_orphan() -> Scenario {
    Scenario::new(
        "a due quiet job isn't held off by an orphan that never ends",
        MachineConfig::for_tests(),
    )
    .checks("r3-admission A16")
    .load(Sample::at(ms(0)).background(4_000, 0))
    .arrive(
        ms(0),
        Job::new("o", Class::Light).uses(500, 64).runs_for(secs(60)),
    )
    .fault(
        ms(500),
        Fault::signal("o", Process::Supervisor, Signal::Kill),
    )
    .arrive(ms(1_000), Job::new("q", Class::Quiet).runs_for(secs(1)))
    .arrive(ms(6_200), Job::new("l", Class::Light))
    .expect(Expect::not_admitted_before("q", secs(6)))
    .expect(Expect::admitted_by("q", secs(7)))
    .expect(Expect::admitted_before("q", "l"))
    .expect(Expect::admitted_by("l", secs(9)))
}

/// Outside load holds 8000 MB, so only 8384 is free and neither `w1` nor `w2` (9000 MB each) fits; first-in never
/// waives memory. The machine sleeps from 1.5 s to 9.5 s and the outside load is gone when it wakes, leaving room for
/// exactly one of them (16384 - 9000 = 7384 is left for the other). The first sample after waking isn't fresh, so
/// the first passes run the plain-lock fallback; the stall rule matters on the passes after it. `w1`'s supervisor is
/// stopped from 1.4 s to 10 s, so on those passes `w1`'s last pass is more than 8 s old and `w2` is certain to look
/// first. "Nobody is judged stalled ... until the stall limit has passed since the machine last woke" (1 s, so
/// 10.5 s), so `w2` still sees `w1` ahead, and `w1` gets the room once it runs again. Judging `w1` stalled would
/// let `w2` jump it.
fn nobody_jumps_the_queue_after_the_machine_wakes() -> Scenario {
    Scenario::new(
        "after the machine wakes, no waiter is skipped as stalled",
        MachineConfig::for_tests(),
    )
    .checks("r2-concurrency B7")
    .checks("r3-admission A14")
    .load(Sample::at(ms(0)).background(0, 8_000))
    .load(Sample::at(ms(9_500)))
    .arrive(
        ms(0),
        Job::new("w1", Class::Light)
            .reserves(1_000, 9_000)
            .runs_for(secs(1)),
    )
    .arrive(ms(500), Job::new("w2", Class::Light).reserves(1_000, 9_000))
    .fault(
        ms(1_400),
        Fault::signal("w1", Process::Supervisor, Signal::Stop),
    )
    .sleep(ms(1_500), ms(9_500))
    .fault(
        ms(10_000),
        Fault::signal("w1", Process::Supervisor, Signal::Cont),
    )
    .expect(Expect::admitted_before("w1", "w2"))
    .expect(Expect::never_together("w1", "w2"))
}

/// `h1` and `h2` (heavy) run for 12 s, so `h3` waits on `max_running.heavy` past `aging` and `aging_hold` (its hold
/// would lapse at 6.6 s). A lapsed hold drains the machine only if `allowed(t, ...) and fits_once_drained(t)`
/// (ARCHITECTURE rev 111), and `h3` isn't allowed, so it doesn't stop the walk, and `l`, arriving after the lapse,
/// starts at once.
fn class_blocked_ticket_doesnt_drain_when_its_hold_lapses() -> Scenario {
    let heavy = |label| {
        Job::new(label, Class::Heavy)
            .reserves(2_000, 1_024)
            .runs_for(secs(12))
    };
    Scenario::new(
        "a ticket its class keeps out doesn't drain the machine when its hold lapses",
        MachineConfig::for_tests(),
    )
    .checks("r2-admission B6")
    .arrive(ms(0), heavy("h1"))
    .arrive(ms(300), heavy("h2"))
    .arrive(ms(600), heavy("h3"))
    .arrive(ms(7_000), Job::new("l", Class::Light))
    .expect(Expect::admitted_by("l", ms(8_000)))
    .expect(Expect::admitted_before("l", "h3"))
}

/// The sampler never reads memory, so admission is the plain-lock fallback: one job at a time, "admit me only if
/// no living ticket is charged and I am first in waiting". `b` and `c` wait behind `a`, which ends at about 1.2 s.
/// `b`'s ticket is corrupted at 1.1 s, and `b`'s supervisor is stopped from 1.1 s to 1.8 s (inside the 1 s stall
/// limit), so its ticket stays unreadable while `a` ends and `c` runs its passes. A locked ticket with no readable
/// slot is charged at the heavy minimum, so `c`'s fallback sees a living charged ticket and waits: `c` can't jump
/// `b`. Not charging it would let `c` in first. When `b` runs again it rewrites its ticket and gets in, then `c`.
fn unreadable_ticket_doesnt_stop_the_fallback_for_good() -> Scenario {
    Scenario::new(
        "with no reliable load, an unreadable ticket doesn't stop the queue for good",
        MachineConfig::for_tests(),
    )
    .checks("r3-admission A19")
    .load(Sample::at(ms(0)).missing(Reading::Memory))
    .arrive(ms(0), Job::new("a", Class::Light).runs_for(secs(1)))
    .arrive(ms(300), Job::new("b", Class::Light))
    .arrive(ms(600), Job::new("c", Class::Light))
    .fault(ms(1_100), Fault::corrupt_ticket("b"))
    .fault(
        ms(1_100),
        Fault::signal("b", Process::Supervisor, Signal::Stop),
    )
    .fault(
        ms(1_800),
        Fault::signal("b", Process::Supervisor, Signal::Cont),
    )
    .expect(Expect::admitted("b"))
    .expect(Expect::admitted("c"))
    .expect(Expect::admitted_before("b", "c"))
    .expect(Expect::never_together("a", "b"))
    .expect(Expect::never_together("b", "c"))
}

/// Nick's decision on MT-024 (ARCHITECTURE rev 124): with no reliable load, "if I am quiet and not due ...
/// wait(quiet needs idle)", and the fallback admits the first in waiting "skipping quiet tickets not yet due".
/// Outside CPU load keeps the quiet streak at 0, so `q` isn't allowed before it is due. The machine sleeps from 1 s to
/// 5 s, and the sampler stays down until 8 s, so the load is unreliable from the wake on. `l` arrives as the machine
/// wakes: the fallback skips `q` and admits `l` at once. If it didn't skip `q`, `l` would wait for the load to come
/// back at 8 s. `q` is due after 5 s awake (1 s before the sleep and 4 s after), at 9 s, and doesn't start before.
fn with_no_reliable_load_a_quiet_job_not_yet_due_neither_starts_nor_holds_the_line() -> Scenario {
    Scenario::new(
        "with no reliable load, a quiet job not yet due neither starts nor holds up the fallback",
        MachineConfig::for_tests(),
    )
    .checks("MT-024")
    .load(Sample::at(ms(0)).background(4_000, 0))
    .arrive(ms(0), Job::new("q", Class::Quiet))
    .sleep(ms(1_000), ms(5_000))
    .sampler_down(ms(5_000), Some(ms(8_000)))
    .arrive(ms(5_000), Job::new("l", Class::Light))
    .expect(Expect::admitted_by("l", secs(6)))
    .expect(Expect::not_admitted_before("q", secs(9)))
}

/// Nick's decision on MT-024: the sampler is down from the start, so there is never a load file. `q` (quiet) isn't
/// due until it has waited `quiet_max_wait` (5 s), and until then it waits. Once due, with nothing charged, the
/// fallback admits it, and having got in without the quiet streak, its record is marked `tainted`.
fn with_the_sampler_down_a_due_quiet_job_gets_in_tainted() -> Scenario {
    Scenario::new(
        "with the sampler down, a quiet job gets in once due, tainted, and not before",
        MachineConfig::for_tests(),
    )
    .checks("MT-024")
    .sampler_down(ms(0), None)
    .arrive(ms(0), Job::new("q", Class::Quiet).runs_for(secs(1)))
    .expect(Expect::not_admitted_before("q", secs(5)))
    .expect(Expect::admitted_by("q", secs(6)))
    .expect(Expect::record_flag("q", "tainted"))
    .run_until(secs(8))
}

/// The project's class file for the classification cases: one heavy program, one light one with a slot.
const CLASSES: &str = "\
# name    | pattern            | class | why                                  | options
mutation  | ^bin/mutant( |$)   | heavy | every core for hours                 | cpu=8 mem=6G
specs     | ^bin/rspec( |$)    | light | its databases come from a slot       | slot=test_env
";

/// The classification cases of phase 1: the hook's spotter and argv matching (ARCHITECTURE.md: Class files).
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one list of cases, each with the doc text it rests on"
)]
pub fn classification() -> Vec<ClassCase> {
    let hook = |name, command| ClassCase::hook(name, CLASSES, command).pool("test_env");
    let argv = |name, words: &[&str]| ClassCase::argv(name, CLASSES, words).pool("test_env");
    let mutant = || Hook::refuse("bin/mutant");
    vec![
        // "When it finds a program that a rule other than none names ... it refuses"; anything else runs.
        hook(
            "ordinary commands no rule names run as they are",
            "ls -la && git status",
        )
        .checks("r2-threat B2")
        .expect_hook(Hook::Allow),
        // "It never touches a command whose program is maitred itself, judged command by command ... while any
        // other heavy program in the same string is still refused."
        hook(
            "a heavy program chained after a queued one",
            "maitred -- bin/rspec && bin/mutant run",
        )
        .checks("r2-threat A3")
        .expect_hook(mutant()),
        hook(
            "a heavy program after a maitred command",
            "maitred status && bin/mutant run",
        )
        .checks("r4-classify C5")
        .expect_hook(mutant()),
        // Matching: "a path into the project (./bin/x, bin/x, or its absolute path) is rewritten relative to
        // the project root"; the spotter compares "by its normalised path".
        hook("a heavy program run as ./bin/x", "./bin/mutant run")
            .checks("r2-threat B3")
            .expect_hook(mutant()),
        // The spotter "skips assignments and the wrappers above": nice is one of them.
        hook(
            "a heavy program behind nice and an assignment",
            "FOO=1 nice -n 5 bin/mutant run",
        )
        .checks("r2-threat B3")
        .expect_hook(mutant()),
        // The spotter's paragraph names wrappers but not launchers, so as the doc reads, bundle exec hides the
        // program from the hook. The finding asks for a refusal; this fails until the doc says launchers too.
        hook(
            "a heavy program behind bundle exec",
            "bundle exec bin/mutant run",
        )
        .checks("r2-threat B3")
        .expect_hook(mutant())
        .fails_until("MT-012"),
        // Command position includes "$(", "(" and "a newline", and the spotter never looks inside "a heredoc's
        // body", so a command after the body is seen.
        hook(
            "a heavy program in command substitution",
            "echo $(bin/mutant run)",
        )
        .checks("r3-threat T8")
        .expect_hook(mutant()),
        hook("a heavy program in a subshell", "(bin/mutant run)")
            .checks("r3-threat T8")
            .expect_hook(mutant()),
        hook(
            "a heavy program after a heredoc",
            "cat <<EOF\nhello\nEOF\nbin/mutant run",
        )
        .checks("r3-threat T8")
        .expect_hook(mutant()),
        hook(
            "a heredoc body that mentions a heavy program",
            "git commit -F - <<'EOF'\nbin/mutant run tonight\nEOF",
        )
        .checks("r4-classify C2")
        .expect_hook(Hook::Allow),
        // Command position includes "|&", "&|" and "&!" (the last two zsh's).
        hook("a heavy program after |&", "echo hi |& bin/mutant run")
            .checks("r4-classify C3")
            .expect_hook(mutant()),
        hook("a heavy program after zsh's &|", "true &| bin/mutant run")
            .checks("r4-classify C3")
            .expect_hook(mutant()),
        hook("a heavy program after zsh's &!", "true &! bin/mutant run")
            .checks("r4-classify C3")
            .expect_hook(mutant()),
        // "do" is a command position, so a loop's body is seen.
        hook(
            "a heavy program inside a for loop",
            "for f in a b; do bin/mutant run; done",
        )
        .checks("r4-classify C4")
        .expect_hook(mutant()),
        // "if" and "while" aren't in the doc's list, so these fail until MT-002 is fixed.
        hook(
            "a heavy program after if",
            "if bin/mutant run; then echo ok; fi",
        )
        .checks("r4-classify C4")
        .expect_hook(mutant())
        .fails_until("MT-002"),
        hook(
            "a heavy program after while",
            "while bin/mutant run; do :; done",
        )
        .checks("r4-classify C4")
        .expect_hook(mutant())
        .fails_until("MT-002"),
        // A case pattern's ")" isn't in the doc's list either (MT-002); "{" is.
        hook(
            "a heavy program in a case branch",
            "case x in x) bin/mutant run;; esac",
        )
        .checks("r4-classify C4")
        .expect_hook(mutant())
        .fails_until("MT-002"),
        hook("a heavy program in braces", "{ bin/mutant run; }")
            .checks("r4-classify C4")
            .expect_hook(mutant()),
        // "&>" redirects; it doesn't end a command, and what follows "&&" is in command position.
        hook(
            "a heavy program after a command redirected with &>",
            "echo hi &> log && bin/mutant run",
        )
        .checks("r4-classify C3")
        .expect_hook(mutant()),
        // Matching strips launchers the machine config lists, such as bundle exec, then matches the rule.
        argv("specs behind bundle exec", &["bundle", "exec", "bin/rspec"])
            .checks("r2-threat B3")
            .expect_match(
                MatchExpect::queued(Class::Light)
                    .rule("specs")
                    .slot("test_env"),
            ),
        // Matching strips wrappers "repeatedly: leading NAME=value assignments, then wrappers".
        argv(
            "a heavy program behind env and nice",
            &["env", "FOO=1", "nice", "-n", "5", "bin/mutant", "run"],
        )
        .checks("r3-threat T10")
        .expect_match(MatchExpect::queued(Class::Heavy).rule("mutation")),
        // Keep-awake wrappers refuse the job only "as the program after stripping; the word as an argument
        // (rg caffeinate) does not". rg matches no rule, so it is heavy.
        argv("a keep-awake word as an argument", &["rg", "caffeinate"])
            .checks("r2-threat A14")
            .expect_match(MatchExpect::queued(Class::Heavy).no_rule()),
        argv(
            "a keep-awake wrapper as the program",
            &["caffeinate", "-i", "bin/mutant", "run"],
        )
        .checks("r2-threat A14")
        .expect_match(MatchExpect::refused()),
        // A shell string "takes the strictest class among the programs it finds, every slot any of them asks
        // for, and their largest reservation". Programs no rule names add nothing.
        argv(
            "two queued programs in one shell string",
            &["sh", "-c", "bin/rspec && bin/mutant run"],
        )
        .checks("r3-threat T12")
        .checks("r4-classify C14")
        .expect_match(MatchExpect::queued(Class::Heavy).slot("test_env")),
        argv(
            "a light program piped into an ordinary helper",
            &["sh", "-c", "bin/rspec | tail -40"],
        )
        .checks("r4-classify C1")
        .expect_match(MatchExpect::queued(Class::Light).slot("test_env")),
        // "An invalid file ... makes every command heavy, with a warning naming the bad line." The second rule has no
        // why, which makes the file invalid, so even the valid first rule's program is heavy: skipping only the bad
        // line would leave bin/rspec light.
        ClassCase::argv(
            "an invalid class file",
            "specs | ^bin/rspec( |$) | light | runs the suite\nbroken | ^bin/x( |$) | light\n",
            &["bin/rspec"],
        )
        .checks("r2-threat A9")
        .expect_match(MatchExpect::queued(Class::Heavy).no_rule()),
        // "A rule asking for a pool this machine doesn't declare is skipped with a warning instead": only that
        // rule. With no pool declared, specs is skipped and bin/rspec matches nothing, so it is heavy, while
        // mutation still matches.
        ClassCase::argv(
            "a rule asking for an undeclared pool skips only itself",
            CLASSES,
            &["bin/mutant", "run"],
        )
        .checks("r4-classify C8")
        .expect_match(MatchExpect::queued(Class::Heavy).rule("mutation")),
        ClassCase::argv(
            "the skipped rule's program falls to heavy",
            CLASSES,
            &["bin/rspec"],
        )
        .checks("r4-classify C8")
        .expect_match(MatchExpect::queued(Class::Heavy).no_rule()),
        // A rule after the skipped one still matches, so skipping the rest of the file isn't enough.
        ClassCase::argv(
            "a rule after the skipped one still matches",
            &format!("{CLASSES}bench     | ^bin/bench( |$)    | quiet | its output is a timing\n"),
            &["bin/bench"],
        )
        .checks("r4-classify C8")
        .expect_match(MatchExpect::queued(Class::Quiet).rule("bench")),
        // ARCHITECTURE rev 106: "a job takes at most one slot per pool", however many of its programs ask.
        argv(
            "two programs asking for one pool take one slot",
            &["sh", "-c", "bin/rspec && bin/rspec"],
        )
        .checks("r4-classify C14")
        .expect_match(MatchExpect::queued(Class::Light).slot("test_env")),
        // A shell string takes "their largest reservation": mutation's cpu=8 mem=6G.
        argv(
            "a shell string takes its programs' largest reservation",
            &["sh", "-c", "bin/rspec && bin/mutant run"],
        )
        .checks("r3-threat T12")
        .checks("r4-classify C14")
        .expect_match(
            MatchExpect::queued(Class::Heavy)
                .slot("test_env")
                .reserves(8_000, 6_144),
        ),
        // The wrappers' option grammars: "nice (including -n N and -N)", "timeout and gtimeout (including -k ...)",
        // "env (including ... -C ...)", "exec", "xargs (including ... -n ...)".
        argv(
            "a heavy program behind nice -5",
            &["nice", "-5", "bin/mutant", "run"],
        )
        .checks("r3-threat T10")
        .expect_match(MatchExpect::queued(Class::Heavy).rule("mutation")),
        argv(
            "a heavy program behind timeout -k",
            &["timeout", "-k", "5", "60", "bin/mutant", "run"],
        )
        .checks("r3-threat T10")
        .expect_match(MatchExpect::queued(Class::Heavy).rule("mutation")),
        argv(
            "a heavy program behind gtimeout",
            &["gtimeout", "60", "bin/mutant", "run"],
        )
        .checks("r3-threat T10")
        .expect_match(MatchExpect::queued(Class::Heavy).rule("mutation")),
        argv(
            "a heavy program behind env -C",
            &["env", "-C", ".", "bin/mutant", "run"],
        )
        .checks("r3-threat T10")
        .expect_match(MatchExpect::queued(Class::Heavy).rule("mutation")),
        // "command (but not command -v or -V, which look a command up rather than run it)": the program is command
        // itself, which no rule names.
        argv(
            "command -v looks a program up and doesn't run it",
            &["command", "-v", "bin/mutant"],
        )
        .checks("r3-threat T10")
        .expect_match(MatchExpect::queued(Class::Heavy).no_rule()),
        // A program's own options are the rule's to match; only wrappers and launchers are stripped (SK, for the
        // doc's Matching section, 2026-10-16). So a class file that runs `make -C dir release` writes them in.
        ClassCase::argv(
            "a rule written for make -C matches it",
            "release | ^make( -C [^ ]+)? +release( |$) | heavy | a full optimised build\n",
            &["make", "-C", "app", "release"],
        )
        .checks("r4-classify C14")
        .expect_match(MatchExpect::queued(Class::Heavy).rule("release")),
        argv(
            "a heavy program behind exec",
            &["exec", "bin/mutant", "run"],
        )
        .checks("r2-threat B3")
        .expect_match(MatchExpect::queued(Class::Heavy).rule("mutation")),
        argv(
            "a heavy program behind xargs",
            &["xargs", "-n", "1", "bin/mutant", "run"],
        )
        .checks("r2-threat B3")
        .expect_match(MatchExpect::queued(Class::Heavy).rule("mutation")),
    ]
}
