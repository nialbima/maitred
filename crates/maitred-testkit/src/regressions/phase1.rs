//! Phase 1: the pure core. These scenarios run on the simulator (`sim` in FINDINGS.md).
//!
//! The machine is `MachineConfig::for_tests()`: 8 cores, 16384 MB, a 1024 MB memory floor, a light class
//! default of 1000 millicores and 512 MB, a 200 ms poll and sample interval, 3 s aging, a 3 s aging hold and a
//! 1 s stall limit. Each scenario's comment works its numbers from "Inside `decide`" (ARCHITECTURE.md).

use maitred_scenario::{
    Class, ClassCase, Expect, Fault, Hook, Job, MachineConfig, Matched, Process, Reading, Sample,
    Scenario, Signal, ms, secs,
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

/// Three heavy jobs, each reserving 2000 millicores, so their budget (6000) fits 8 cores. Only the class limit,
/// `max_running.heavy` = 2, keeps the third out. Settle (600 ms) spaces the first two. The limit counts jobs
/// "charged or admitted on paper", so no pass admits a third beside two, and order follows arrival.
fn heavy_limit_counts_admissions_on_paper() -> Scenario {
    let heavy = |label| {
        Job::new(label, Class::Heavy)
            .reserves(2_000, 1_024)
            .runs_for(secs(3))
    };
    Scenario::new(
        "no more heavy jobs run than the class allows, in queue order",
        MachineConfig::for_tests(),
    )
    .checks("r2-admission A6")
    .arrive(ms(0), heavy("h1"))
    .arrive(ms(300), heavy("h2"))
    .arrive(ms(600), heavy("h3"))
    .expect(Expect::at_most_together(2, &["h1", "h2", "h3"]))
    .expect(Expect::admitted_before("h1", "h2"))
    .expect(Expect::admitted_before("h2", "h3"))
    .expect(Expect::admitted("h3"))
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

/// The project's class file for the classification cases: one heavy program, one light one with a slot.
const CLASSES: &str = "\
# name    | pattern            | class | why                                  | options
mutation  | ^bin/mutant( |$)   | heavy | every core for hours
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
        .expect_hook(mutant()),
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
            "git commit -F - <<'EOF'\nrun bin/mutant run tonight\nEOF",
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
        .expect_hook(mutant()),
        hook(
            "a heavy program after while",
            "while bin/mutant run; do :; done",
        )
        .checks("r4-classify C4")
        .expect_hook(mutant()),
        // Matching strips launchers the machine config lists, such as bundle exec, then matches the rule.
        argv("specs behind bundle exec", &["bundle", "exec", "bin/rspec"])
            .checks("r2-threat B3")
            .expect_match(Matched::queued(Class::Light).rule("specs").slot("test_env")),
        // Matching strips wrappers "repeatedly: leading NAME=value assignments, then wrappers".
        argv(
            "a heavy program behind env and nice",
            &["env", "FOO=1", "nice", "-n", "5", "bin/mutant", "run"],
        )
        .checks("r3-threat T10")
        .expect_match(Matched::queued(Class::Heavy).rule("mutation")),
        // Keep-awake wrappers refuse the job only "as the program after stripping; the word as an argument
        // (rg caffeinate) does not". rg matches no rule, so it is heavy.
        argv("a keep-awake word as an argument", &["rg", "caffeinate"])
            .checks("r2-threat A14")
            .expect_match(Matched::queued(Class::Heavy)),
        argv(
            "a keep-awake wrapper as the program",
            &["caffeinate", "-i", "bin/mutant", "run"],
        )
        .checks("r2-threat A14")
        .expect_match(Matched::Refused),
        // A shell string "takes the strictest class among the programs it finds, every slot any of them asks
        // for, and their largest reservation". Programs no rule names add nothing.
        argv(
            "two queued programs in one shell string",
            &["sh", "-c", "bin/rspec && bin/mutant run"],
        )
        .checks("r3-threat T12")
        .checks("r4-classify C14")
        .expect_match(Matched::queued(Class::Heavy).slot("test_env")),
        argv(
            "a light program piped into an ordinary helper",
            &["sh", "-c", "bin/rspec | tail -40"],
        )
        .checks("r4-classify C1")
        .expect_match(Matched::queued(Class::Light).slot("test_env")),
        // "An invalid file ... makes every command heavy, with a warning naming the bad line." This rule has no
        // why, which makes the file invalid.
        ClassCase::argv(
            "an invalid class file",
            "specs | ^bin/rspec( |$) | light\n",
            &["bin/rspec"],
        )
        .checks("r2-threat A9")
        .expect_match(Matched::queued(Class::Heavy)),
        // "A rule asking for a pool this machine doesn't declare is skipped with a warning instead": only that
        // rule. With no pool declared, specs is skipped and bin/rspec matches nothing, so it is heavy, while
        // mutation still matches.
        ClassCase::argv(
            "a rule asking for an undeclared pool skips only itself",
            CLASSES,
            &["bin/mutant", "run"],
        )
        .checks("r4-classify C8")
        .expect_match(Matched::queued(Class::Heavy).rule("mutation")),
        ClassCase::argv(
            "the skipped rule's program falls to heavy",
            CLASSES,
            &["bin/rspec"],
        )
        .checks("r4-classify C8")
        .expect_match(Matched::queued(Class::Heavy)),
    ]
}
