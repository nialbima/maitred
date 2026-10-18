//! ARCHITECTURE.md's Class files, Matching: steps 1 to 7, in order.

use std::collections::BTreeSet;

use super::*;
use crate::classes::parse;

const ROOT: &str = "/work/app";

fn file(rules: &str) -> ClassFile {
    parse(rules, &BTreeSet::new()).expect("valid rules")
}

fn machine() -> Machine {
    Machine {
        wrappers: vec!["with-lock".into()],
        launchers: vec![
            vec!["bundle".into(), "exec".into()],
            vec!["bin/bundle".into(), "exec".into()],
            vec!["npx".into()],
            vec!["ruby".into(), "-S".into()],
        ],
        keep_awake: vec!["amphetamine".into()],
        root_commands: vec![],
    }
}

fn job(argv: &[&str]) -> Job {
    Job {
        argv: argv.iter().map(|a| (*a).to_owned()).collect(),
        cwd: ROOT.into(),
        root: ROOT.into(),
    }
}

/// The rule a job matches, by name, or "heavy" for no match.
fn matched(rules: &str, j: &Job) -> String {
    let f = file(rules);
    match classify(&f, j, &machine()) {
        Classified::Rule { rule, .. } => rule.name.clone(),
        Classified::NoRule { .. } => "heavy".into(),
        other => panic!("{other:?}"),
    }
}

/// The command a pattern sees.
fn command(argv: &[&str]) -> String {
    let f = file("any | ^never-matches | light | why");
    match classify(&f, &job(argv), &machine()) {
        Classified::Rule { command, .. } | Classified::NoRule { command } => command,
        other => panic!("{other:?}"),
    }
}

// 1. Wrappers.

#[test]
fn leading_assignments_are_stripped() {
    assert_eq!(
        command(&["FOO=1", "BAR=a b", "bin/x", "--fast"]),
        "bin/x --fast"
    );
}

#[test]
fn each_wrapper_and_its_options_are_stripped_wherever_it_lives() {
    for argv in [
        &["env", "-i", "-u", "HOME", "FOO=1", "bin/x"][..],
        &["/usr/bin/env", "--unset=HOME", "-v", "bin/x"],
        &["exec", "-a", "name", "bin/x"],
        &["command", "-p", "bin/x"],
        &["nohup", "bin/x"],
        &["/usr/bin/time", "-p", "bin/x"],
        &["time", "-f", "%e", "-o", "out", "bin/x"],
        &["nice", "-n", "10", "bin/x"],
        &["nice", "-10", "bin/x"],
        &["nice", "--adjustment=5", "bin/x"],
        &["timeout", "-k", "5", "--signal=INT", "30s", "bin/x"],
        &["gtimeout", "--preserve-status", "1m", "bin/x"],
        &["xargs", "-0", "-n", "1", "-P", "4", "-I", "{}", "bin/x"],
        &["xargs", "-a", "list", "-d", ",", "-J", "%", "bin/x"],
        &["setsid", "-w", "bin/x"],
        &["stdbuf", "-o", "L", "-eL", "bin/x"],
        &["ionice", "-c", "3", "bin/x"],
        &["taskpolicy", "-b", "-c", "utility", "bin/x"],
        &["arch", "-arch", "x86_64", "-e", "FOO=1", "bin/x"],
        &["arch", "-arm64", "bin/x"],
        &["with-lock", "bin/x"],
        &["nice", "-n", "5", "nohup", "env", "A=1", "time", "bin/x"],
    ] {
        assert_eq!(command(argv), "bin/x", "{argv:?}");
    }
}

#[test]
fn env_s_splits_its_string_into_the_command() {
    assert_eq!(
        command(&["env", "-S", "bin/x --fast 'a b'"]),
        "bin/x --fast 'a b'"
    );
}

#[test]
fn a_short_option_can_carry_its_value_attached() {
    assert_eq!(command(&["nice", "-n5", "bin/x"]), "bin/x");
    assert_eq!(command(&["xargs", "-n1", "-P4", "bin/x"]), "bin/x");
}

#[test]
fn a_double_dash_ends_a_wrappers_options() {
    assert_eq!(command(&["env", "--", "bin/x"]), "bin/x");
    assert_eq!(command(&["nohup", "--", "bin/x"]), "bin/x");
}

#[test]
fn command_v_looks_a_program_up_and_runs_nothing() {
    assert_eq!(command(&["command", "-v", "bin/x"]), "command -v bin/x");
    assert_eq!(command(&["command", "-V", "bin/x"]), "command -V bin/x");
}

// 2. Normalising the program.

#[test]
fn a_path_into_the_project_is_made_relative_to_its_root() {
    for argv in [
        &["./bin/x"][..],
        &["bin/x"],
        &["/work/app/bin/x"],
        &["bin/../bin/x"],
        &["./bin/./x"],
    ] {
        assert_eq!(command(argv), "bin/x", "{argv:?}");
    }
}

#[test]
fn a_relative_path_is_taken_from_the_working_directory() {
    let mut j = job(&["../bin/x"]);
    j.cwd = "/work/app/sub".into();
    let f = file("x | ^bin/x | light | why");

    assert!(matches!(
        classify(&f, &j, &machine()),
        Classified::Rule { .. }
    ));
}

#[test]
fn a_path_outside_the_project_and_a_bare_name_are_left_as_they_are() {
    assert_eq!(command(&["/usr/local/bin/mutant"]), "/usr/local/bin/mutant");
    assert_eq!(command(&["/work/app-other/bin/x"]), "/work/app-other/bin/x");
    assert_eq!(command(&["make", "release"]), "make release");
}

// 3. Launchers.

#[test]
fn launchers_are_stripped_after_normalising_and_the_program_normalised_again() {
    for argv in [
        &["bundle", "exec", "./bin/x"][..],
        &["./bin/bundle", "exec", "bin/x"],
        &["/work/app/bin/bundle", "exec", "/work/app/bin/x"],
        &["npx", "bin/x"],
        &["ruby", "-S", "bin/x"],
        &["nice", "bundle", "exec", "bin/x"],
    ] {
        assert_eq!(command(argv), "bin/x", "{argv:?}");
    }
}

#[test]
fn wrappers_after_a_launcher_are_stripped_too() {
    // A launcher runs its next word as a command, so a wrapper there wraps the program (Nick,
    // 2026-10-17, MT-049).
    for argv in [
        &["bundle", "exec", "nice", "-n", "5", "bin/x"][..],
        &["npx", "FOO=1", "timeout", "10", "./bin/x"],
        &["nice", "bundle", "exec", "with-lock", "bin/x"],
    ] {
        assert_eq!(command(argv), "bin/x", "{argv:?}");
    }
}

#[test]
fn a_launcher_only_counts_whole() {
    assert_eq!(command(&["bundle", "install"]), "bundle install");
    assert_eq!(command(&["ruby", "bin/x"]), "ruby bin/x");
}

// 4. Joining.

#[test]
fn arguments_with_whitespace_or_quotes_are_quoted() {
    assert_eq!(
        command(&["bin/x", "a b", "it's", "say \"hi\"", "tab\there", "plain"]),
        r#"bin/x 'a b' 'it'\''s' 'say "hi"' 'tab	here' plain"#
    );
}

// 5. Matching.

#[test]
fn the_first_matching_rule_wins_and_no_match_is_heavy() {
    let rules = "fast | ^bin/x --fast | light | why\nslow | ^bin/x( |$) | heavy | why\n";

    assert_eq!(matched(rules, &job(&["bin/x", "--fast"])), "fast");
    assert_eq!(matched(rules, &job(&["bin/x"])), "slow");
    assert_eq!(matched(rules, &job(&["bin/y"])), "heavy");
}

#[test]
fn a_programs_own_options_stay_for_the_pattern() {
    let rules = "release | ^make( -C [^ ]+)? +release | heavy | why";

    assert_eq!(
        matched(rules, &job(&["make", "-C", "dir", "release"])),
        "release"
    );
}

#[test]
fn an_empty_command_is_heavy() {
    assert_eq!(matched("x | ^bin/x | light | why", &job(&[])), "heavy");
    assert_eq!(
        matched("x | ^bin/x | light | why", &job(&["env", "A=1"])),
        "heavy"
    );
}

// 6. Keep-awake wrappers.

#[test]
fn a_keep_awake_program_is_refused_by_name() {
    for (argv, program) in [
        (&["caffeinate", "-i", "bin/x"][..], "caffeinate"),
        (&["/usr/bin/caffeinate", "bin/x"], "caffeinate"),
        (&["nice", "systemd-inhibit", "bin/x"], "systemd-inhibit"),
        (&["amphetamine", "bin/x"], "amphetamine"),
    ] {
        let f = file("x | ^bin/x | light | why");
        assert!(
            matches!(
                classify(&f, &job(argv), &machine()),
                Classified::KeepAwake { program: p } if p == program
            ),
            "{argv:?}"
        );
    }
}

#[test]
fn a_keep_awake_word_as_an_argument_is_not_refused() {
    assert_eq!(
        matched("rg | ^rg( |$) | light | why", &job(&["rg", "caffeinate"])),
        "rg"
    );
}

// 7. A shell as the program.

#[test]
fn a_shell_with_dash_c_hands_its_string_to_the_spotter() {
    for (argv, shell) in [
        (&["sh", "-c", "bin/x && bin/y"][..], "sh"),
        (&["/bin/bash", "-c", "bin/x && bin/y"], "bash"),
        (&["zsh", "-lc", "bin/x && bin/y"], "zsh"),
        (&["bash", "-e", "-c", "bin/x && bin/y", "arg0"], "bash"),
        (&["nice", "sh", "-c", "bin/x && bin/y"], "sh"),
        (&["bash", "-o", "pipefail", "-c", "bin/x && bin/y"], "bash"),
        (&["bash", "+o", "posix", "-c", "bin/x && bin/y"], "bash"),
        (&["bash", "-O", "extglob", "-c", "bin/x && bin/y"], "bash"),
        (&["bash", "--norc", "-c", "bin/x && bin/y"], "bash"),
        (&["bash", "--rcfile", "rc", "-c", "bin/x && bin/y"], "bash"),
        (&["bash", "-c", "--", "bin/x && bin/y"], "bash"),
    ] {
        assert_eq!(
            shell_of(argv),
            Some((shell.into(), "bin/x && bin/y".into())),
            "{argv:?}"
        );
    }
}

/// A shell and the string it runs, if the job is one.
fn shell_of(argv: &[&str]) -> Option<(String, String)> {
    let f = file("x | ^bin/x | light | why");
    match classify(&f, &job(argv), &machine()) {
        Classified::Shell { shell, script } => Some((shell, script)),
        _ => None,
    }
}

#[test]
fn a_shell_without_dash_c_is_not_running_a_string() {
    for argv in [
        &["bash", "--login", "bin/setup"][..],
        &["bash", "-o", "c", "bin/setup"],
        &["bash", "-e", "bin/setup"],
        &["zsh"],
    ] {
        assert_eq!(shell_of(argv), None, "{argv:?}");
    }
}

#[test]
fn a_shell_running_a_script_file_is_matched_like_any_program() {
    assert_eq!(command(&["bash", "bin/setup"]), "bash bin/setup");
}

// A rule applies only to its own program (Qualmaude's hold on 1852f8a): no pattern, however it is
// written, classes another program.

#[test]
fn a_rule_never_classes_another_program() {
    for (pattern, other) in [
        (r"^bin/x |evil", &["bin/evil"][..]),
        (r"^bin/x |evil", &["cargo", "build", "--evil"]),
        (r"^bin/x( |$)|evil", &["cargo", "evil"]),
        (r"^bin/x$|.*", &["anything"]),
        (r"^bin/x\b", &["bin/x.rb"]),
        (r"^bin/x\b", &["bin/x-other"]),
        (r"^bin/x(y)?", &["bin/xy"]),
        (r"^bin/x ?y", &["bin/xy"]),
        (r"^bin/x\s*evil", &["bin/xevil"]),
    ] {
        let rules = format!("x | {pattern} | light | why");
        assert_eq!(matched(&rules, &job(other)), "heavy", "{pattern} {other:?}");
    }
}

#[test]
fn a_rule_still_applies_to_its_own_program() {
    assert_eq!(
        matched(r"x | ^bin/x( |$) | light | why", &job(&["bin/x", "--fast"])),
        "x"
    );
    assert_eq!(
        matched(r"x | ^bin/x\.rb( |$) | light | why", &job(&["bin/x.rb"])),
        "x"
    );
}

#[test]
fn an_assignment_is_a_shell_name_and_a_value() {
    assert_eq!(command(&["_X=1", "A_B2=1", "bin/x"]), "bin/x");
    for argv in [
        &["1A=x", "bin/x"][..],
        &["A-B=1", "bin/x"],
        &["=1", "bin/x"],
    ] {
        assert_eq!(command(argv), argv.join(" "), "{argv:?}");
    }
}

#[test]
fn env_s_takes_its_string_attached_too() {
    assert_eq!(command(&["env", "-Sbin/x --fast"]), "bin/x --fast");
    assert_eq!(
        command(&["env", "--split-string=bin/x --fast"]),
        "bin/x --fast"
    );
    assert_eq!(
        command(&["env", "--split-string", "bin/x --fast"]),
        "bin/x --fast"
    );
}

// From Qualmaude's receipt on d29cefa.

#[test]
fn env_chdir_moves_the_directory_a_relative_program_is_resolved_from() {
    for (argv, program) in [
        (
            &["env", "-C", "/elsewhere", "bin/x"][..],
            "/elsewhere/bin/x",
        ),
        (&["env", "--chdir=/elsewhere", "bin/x"], "/elsewhere/bin/x"),
        (
            &["env", "--chdir", "/elsewhere", "bin/x"],
            "/elsewhere/bin/x",
        ),
        (&["env", "-C", "sub", "bin/x"], "sub/bin/x"),
        (&["env", "-Csub", "../bin/x"], "bin/x"),
        (
            &["env", "-C", "/elsewhere", "env", "-C", "deeper", "bin/x"],
            "/elsewhere/deeper/bin/x",
        ),
    ] {
        assert_eq!(command(argv), program, "{argv:?}");
    }
}

#[test]
fn a_rule_doesnt_class_a_program_env_ran_from_elsewhere() {
    assert_eq!(
        matched(
            "x | ^bin/x( |$) | light | why",
            &job(&["env", "-C", "/elsewhere", "bin/x"])
        ),
        "heavy"
    );
}

#[test]
fn command_with_v_anywhere_in_its_options_runs_nothing() {
    for argv in [
        &["command", "-pv", "bin/x"][..],
        &["command", "-pV", "bin/x"],
    ] {
        assert_eq!(command(argv), argv.join(" "), "{argv:?}");
    }
}

#[test]
fn command_looks_up_only_when_an_option_says_so() {
    assert_eq!(command(&["command", "bin/v"]), "bin/v");
    assert_eq!(command(&["command", "--verbose", "bin/x"]), "bin/x");
}

// sudo and doas (Nick, 2026-10-17): maitred can't supervise a job run as root, so one is refused.

#[test]
fn a_job_run_through_sudo_or_doas_is_refused() {
    let f = file("x | ^bin/x | light | why");
    for (argv, via) in [
        (&["sudo", "bin/x"][..], "sudo"),
        (&["/usr/bin/sudo", "-E", "-u", "nick", "bin/x"][..], "sudo"),
        (&["nice", "doas", "-n", "bin/x"][..], "doas"),
        (&["sudo", "-l"][..], "sudo"),
        (&["bundle", "exec", "sudo", "bin/x"][..], "sudo"),
        (&["sudo", "bundle", "exec", "bin/x"][..], "sudo"),
        (&["npx", "nice", "doas", "bin/x"][..], "doas"),
    ] {
        assert!(
            matches!(classify(&f, &job(argv), &machine()), Classified::Escalated { via: v } if v == via),
            "{argv:?}"
        );
    }
}

#[test]
fn sudos_close_from_takes_a_value_and_moves_nothing() {
    assert_eq!(
        strip_wrappers(words(&["sudo", "-C", "3", "bin/x"]), &[], ROOT).dir,
        ROOT
    );
    assert_eq!(
        strip_wrappers(words(&["sudo", "-C", "3", "bin/x"]), &[], ROOT).words,
        ["bin/x"]
    );
}

#[test]
fn sudos_chdir_moves_where_the_command_runs() {
    for argv in [
        &["sudo", "-D", "/elsewhere", "bin/x"][..],
        &["sudo", "--chdir=/elsewhere", "bin/x"][..],
    ] {
        let stripped = strip_wrappers(words(argv), &[], ROOT);
        assert_eq!(stripped.dir, "/elsewhere", "{argv:?}");
        assert_eq!(stripped.escalated.as_deref(), Some("sudo"));
    }
}

#[test]
fn a_command_with_no_escalating_wrapper_isnt_escalated() {
    assert_eq!(
        strip_wrappers(
            words(&["env", "-C", "/elsewhere", "nice", "bin/x"]),
            &[],
            ROOT
        )
        .escalated,
        None
    );
}

fn words(argv: &[&str]) -> Vec<String> {
    argv.iter().map(|a| (*a).to_owned()).collect()
}
