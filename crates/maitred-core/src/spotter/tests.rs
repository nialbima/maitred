//! ARCHITECTURE.md's "The hook's spotter" and "The hook" (the refusal's suggestion).

use super::*;

const CWD: &str = "/work/app";

fn machine() -> Machine {
    Machine {
        wrappers: vec![],
        launchers: vec![vec!["bundle".into(), "exec".into()]],
        keep_awake: vec![],
        root_commands: vec!["repo-root".into()],
    }
}

/// The programs a string runs, in order.
fn programs(script: &str) -> Vec<String> {
    spot(script, CWD, &machine())
        .into_iter()
        .map(|s| s.program)
        .collect()
}

/// The programs and the directory each runs in.
fn programs_in(script: &str) -> Vec<(String, String)> {
    spot(script, CWD, &machine())
        .into_iter()
        .map(|s| (s.program, s.dir))
        .collect()
}

/// The refusal's suggestion for the first program named `program`.
fn suggestion(script: &str, program: &str, queue: Option<&str>) -> String {
    let spotted = spot(script, CWD, &machine());
    let s = spotted
        .iter()
        .find(|s| s.program == program)
        .unwrap_or_else(|| panic!("{program} not spotted in {script}: {spotted:?}"));
    suggest(script, s, queue)
}

// Command positions.

#[test]
fn the_first_word_and_every_word_after_an_operator_is_a_program() {
    assert_eq!(
        programs("a; b && c || d | e |& f & g &| h &! i\nj"),
        ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
    );
}

#[test]
fn operators_need_no_spaces() {
    assert_eq!(programs("a;b&&c||d|e"), ["a", "b", "c", "d", "e"]);
}

#[test]
fn a_subshell_a_group_and_substitution_start_a_command() {
    assert_eq!(
        programs("(a); { b; }; echo $(c) `d`"),
        ["a", "b", "echo", "c", "d"]
    );
}

#[test]
fn keywords_lead_into_a_command() {
    assert_eq!(
        programs("for x in y; do a; done; if b; then c; elif d; then e; else f; fi"),
        ["a", "b", "c", "d", "e", "f"]
    );
    assert_eq!(programs("! a; time b; time -p c"), ["a", "b", "c"]);
}

#[test]
fn if_while_and_until_lead_into_a_command() {
    // MT-002.
    assert_eq!(
        programs("if bin/mutant run; then :; fi; while a; do :; done; until b; do :; done"),
        ["bin/mutant", ":", "a", ":", "b", ":"]
    );
}

#[test]
fn a_case_branch_runs_a_command_and_its_pattern_doesnt() {
    // MT-002.
    assert_eq!(
        programs("case $x in a|b) bin/x ;; (c) bin/y ;& *) bin/z ;;& esac"),
        ["bin/x", "bin/y", "bin/z"]
    );
}

#[test]
fn zsh_repeat_runs_its_command() {
    // MT-002.
    assert_eq!(programs("repeat 3 bin/x"), ["bin/x"]);
}

#[test]
fn arguments_are_never_programs() {
    assert_eq!(programs("git commit -m bin/mutant"), ["git"]);
    assert_eq!(programs("maitred -- bin/mutant run"), ["maitred"]);
}

#[test]
fn a_redirection_target_is_never_a_program() {
    assert_eq!(
        programs("> out a; b 2> err; c < in; >> log d"),
        ["a", "b", "c", "d"]
    );
    assert_eq!(programs("a &> out; b >| out; c 2>&1"), ["a", "b", "c"]);
}

#[test]
fn a_substitution_in_a_redirections_target_runs_with_its_command() {
    assert_eq!(programs("a > $(bin/x); b"), ["a", "bin/x", "b"]);
}

#[test]
fn a_redirection_with_no_target_takes_nothing() {
    // Not valid shell, but the spotter reads it without failing.
    assert_eq!(programs("a >"), ["a"]);
    assert_eq!(programs("a > ; b"), ["a", "b"]);
}

// Assignments, wrappers and launchers.

#[test]
fn assignments_wrappers_and_launchers_are_skipped() {
    assert_eq!(
        programs("FOO=1 nice -n 5 bin/x; bundle exec bin/y; env -i A=1 bin/z"),
        ["bin/x", "bin/y", "bin/z"]
    );
}

#[test]
fn an_assignment_alone_runs_nothing() {
    assert_eq!(programs("FOO=1; a"), ["a"]);
}

// Quoting.

#[test]
fn quoted_text_is_never_a_program() {
    assert_eq!(programs("git commit -m 'bin/mutant run; bin/x'"), ["git"]);
    assert_eq!(programs("echo \"bin/mutant; bin/x\""), ["echo"]);
    assert_eq!(programs("echo $'bin/x\\n'"), ["echo"]);
}

#[test]
fn a_quoted_program_is_still_a_program() {
    assert_eq!(programs("'bin/x' --fast; \"bin/y\""), ["bin/x", "bin/y"]);
}

#[test]
fn a_comment_runs_nothing() {
    assert_eq!(programs("a # b; c\nd"), ["a", "d"]);
    assert_eq!(programs("a#b; c"), ["a#b", "c"]);
}

#[test]
fn a_backslash_newline_joins_two_lines() {
    assert_eq!(programs("nice \\\n  bin/x"), ["bin/x"]);
}

#[test]
fn a_heredocs_body_is_never_read_as_words() {
    assert_eq!(
        programs("cat <<EOF\nbin/mutant run\nEOF\nbin/x"),
        ["cat", "bin/x"]
    );
    assert_eq!(
        programs("cat <<-'EOF'\n\tbin/mutant run\n\tEOF\nbin/x"),
        ["cat", "bin/x"]
    );
}

// Command substitution wherever the shell would run it (Nick, 2026-10-16).

#[test]
fn substitution_inside_double_quotes_is_spotted() {
    assert_eq!(
        programs("echo \"$(bin/mutant run)\""),
        ["echo", "bin/mutant"]
    );
    assert_eq!(
        programs("echo \"`bin/mutant run`\""),
        ["echo", "bin/mutant"]
    );
    assert_eq!(
        programs("git commit -m \"fix $(bin/mutant) bug\""),
        ["git", "bin/mutant"]
    );
}

#[test]
fn substitution_inside_single_quotes_is_not() {
    assert_eq!(programs("echo '$(bin/mutant run)'"), ["echo"]);
}

#[test]
fn substitution_in_an_unquoted_heredoc_is_spotted_and_in_a_quoted_one_is_not() {
    assert_eq!(
        programs("cat <<EOF\nresult: $(bin/mutant run)\nEOF"),
        ["cat", "bin/mutant"]
    );
    assert_eq!(
        programs("cat <<'EOF'\nresult: $(bin/mutant run)\nEOF"),
        ["cat"]
    );
}

#[test]
fn substitution_nests() {
    assert_eq!(programs("a $(b $(c) \"$(d)\")"), ["a", "b", "c", "d"]);
}

#[test]
fn a_shell_dash_c_string_is_spotted_recursively() {
    assert_eq!(
        programs("bash -c 'bin/x && bin/y'"),
        ["bash", "bin/x", "bin/y"]
    );
    assert_eq!(
        programs("/bin/sh -o pipefail -c \"bin/x | tail\""),
        ["/bin/sh", "bin/x", "tail"]
    );
}

// cd (Nick, 2026-10-16).

#[test]
fn a_literal_cd_moves_where_later_programs_run() {
    assert_eq!(
        programs_in("bin/x; cd ../other && bin/y; cd sub; bin/z"),
        [
            ("bin/x".into(), "/work/app".into()),
            ("cd".into(), "/work/app".into()),
            ("bin/y".into(), "/work/other".into()),
            ("cd".into(), "/work/other".into()),
            ("bin/z".into(), "/work/other/sub".into()),
        ]
    );
}

#[test]
fn pushd_moves_like_cd() {
    let spotted = programs_in("pushd /elsewhere; bin/x");
    assert_eq!(spotted[1], ("bin/x".into(), "/elsewhere".into()));
}

#[test]
fn a_cd_it_cant_read_leaves_the_directory_as_it_was() {
    for script in [
        "cd \"$DIR\"; bin/x",
        "cd -; bin/x",
        "cd ~; bin/x",
        "cd; bin/x",
        "cd $(pwd); bin/x",
    ] {
        let spotted = programs_in(script);
        let last = spotted.last().unwrap();
        assert_eq!(last, &("bin/x".into(), "/work/app".into()), "{script}");
    }
}

#[test]
fn a_subshells_cd_ends_with_the_subshell() {
    let spotted = programs_in("(cd /elsewhere; bin/x); bin/y");
    assert_eq!(spotted[1], ("bin/x".into(), "/elsewhere".into()));
    assert_eq!(spotted[2], ("bin/y".into(), "/work/app".into()));
}

#[test]
fn a_dash_c_string_starts_in_the_directory_of_its_shell() {
    let spotted = programs_in("cd /elsewhere && bash -c 'bin/x'");
    assert_eq!(
        spotted.last().unwrap(),
        &("bin/x".into(), "/elsewhere".into())
    );
}

// The refusal's suggestion (Nick, 2026-10-16).

#[test]
fn the_suggestion_wraps_the_programs_simple_command_and_keeps_the_rest() {
    assert_eq!(
        suggestion("bin/mutant run | tail", "bin/mutant", None),
        "maitred -- bin/mutant run | tail"
    );
    assert_eq!(
        suggestion("cd ../other && bin/mutant run", "bin/mutant", None),
        "cd ../other && maitred -- bin/mutant run"
    );
}

#[test]
fn the_suggestion_moves_assignments_after_the_dashes_with_env() {
    assert_eq!(
        suggestion("FOO=1 nice -n 5 bin/mutant run | tail", "bin/mutant", None),
        "maitred -- env FOO=1 nice -n 5 bin/mutant run | tail"
    );
}

#[test]
fn the_suggestion_names_the_container_queue() {
    assert_eq!(
        suggestion(
            "FOO=1 bin/mutant run",
            "bin/mutant",
            Some("docker compose exec app")
        ),
        "docker compose exec app maitred -- env FOO=1 bin/mutant run"
    );
}

#[test]
fn the_suggestion_replaces_a_substitution_where_it_stands() {
    assert_eq!(
        suggestion("echo \"$(bin/mutant run)\"", "bin/mutant", None),
        "echo \"$(maitred -- bin/mutant run)\""
    );
}

#[test]
fn the_suggestion_wraps_a_whole_shell_command_for_a_program_in_its_string() {
    assert_eq!(
        suggestion("bash -c 'bin/mutant run' && b", "bin/mutant", None),
        "maitred -- bash -c 'bin/mutant run' && b"
    );
}

#[test]
fn the_suggestion_keeps_the_keywords_before_the_command() {
    assert_eq!(
        suggestion("if bin/mutant run; then :; fi", "bin/mutant", None),
        "if maitred -- bin/mutant run; then :; fi"
    );
    assert_eq!(
        suggestion("time bin/mutant run", "bin/mutant", None),
        "time maitred -- bin/mutant run"
    );
}

// A program path from the project's root (ARCHITECTURE, Testing: `$(repo-root)/bin/x` names bin/x).

#[test]
fn a_root_command_at_the_start_of_a_path_is_the_projects_root() {
    for script in [
        "$(repo-root)/bin/x run",
        "\"$(repo-root)\"/bin/x run",
        "$(git rev-parse --show-toplevel)/bin/x run",
        "`repo-root`/bin/x run",
    ] {
        let spotted = spot(script, CWD, &machine());
        let x = spotted
            .iter()
            .find(|s| s.rooted)
            .unwrap_or_else(|| panic!("{script}: {spotted:?}"));
        assert_eq!(x.program, "bin/x", "{script}");
    }
}

#[test]
fn any_other_substitution_in_a_path_is_not() {
    let spotted = spot("$(pwd)/bin/x", CWD, &machine());

    assert!(spotted.iter().all(|s| !s.rooted), "{spotted:?}");
}

#[test]
fn a_substitution_that_opens_with_a_subshell_is_spotted() {
    // Bash reads $(( as arithmetic and falls back to a command when it isn't; so does the spotter,
    // which spots the inside either way.
    assert_eq!(programs("echo $(( bin/a ))"), ["echo", "bin/a"]);
    assert_eq!(programs("echo \"$( (bin/a) )\""), ["echo", "bin/a"]);
}

#[test]
fn a_case_inside_a_substitution_doesnt_close_it() {
    assert_eq!(
        programs("echo $(case $v in x|y) bin/a ;; esac) && bin/b"),
        ["echo", "bin/a", "bin/b"]
    );
}

#[test]
fn an_escaped_character_starts_a_word() {
    assert_eq!(programs("\\bin/x"), ["bin/x"]);
}

#[test]
fn and_redirection_takes_one_target_and_the_rest_are_arguments() {
    assert_eq!(programs("a &> out b"), ["a"]);
    assert_eq!(programs("a &>> out b"), ["a"]);
}

#[test]
fn a_comment_can_end_the_string() {
    assert_eq!(programs("a # the end"), ["a"]);
}

#[test]
fn a_heredocs_delimiter_may_follow_a_space_and_its_tabs_may_be_stripped() {
    assert_eq!(programs("cat << EOF\nbin/x\nEOF\nbin/y"), ["cat", "bin/y"]);
    assert_eq!(
        programs("cat <<-EOF\n\tbin/x\n\tEOF\nbin/y"),
        ["cat", "bin/y"]
    );
}

#[test]
fn and_redirection_works_wherever_it_falls() {
    assert_eq!(programs("make &> out b"), ["make"]);
}

#[test]
fn only_digits_touching_a_redirection_are_its_descriptor() {
    assert_eq!(programs("2>err a"), ["a"]);
    assert_eq!(programs("1 > out"), ["1"]);
}

#[test]
fn a_here_strings_word_is_not_a_program() {
    assert_eq!(programs("cat <<< bin/x; d"), ["cat", "d"]);
}

#[test]
fn and_redirection_late_in_a_long_command_leaves_the_next_one() {
    assert_eq!(programs("abcdefgh &> o; z"), ["abcdefgh", "z"]);
}

// Qualmaude's receipt on e432b6e.

#[test]
fn a_function_defined_without_parentheses_runs_its_body() {
    assert_eq!(programs("function f { bin/x; }; bin/y"), ["bin/x", "bin/y"]);
    assert_eq!(
        programs("a; function f { bin/x; }; bin/y"),
        ["a", "bin/x", "bin/y"]
    );
    assert_eq!(programs("function f ( bin/x )"), ["bin/x"]);
    assert_eq!(programs("function f() { bin/x; }"), ["bin/x"]);
}

#[test]
fn coproc_runs_its_command() {
    assert_eq!(programs("coproc bin/x run"), ["bin/x"]);
    assert_eq!(programs("coproc job { bin/x; }"), ["bin/x"]);
}

#[test]
fn popd_returns_to_the_directory_pushd_left() {
    assert_eq!(
        programs_in("pushd sub; popd; bin/x").last().unwrap(),
        &("bin/x".into(), "/work/app".into())
    );
    assert_eq!(
        programs_in("pushd a; pushd /b; popd; bin/x")
            .last()
            .unwrap(),
        &("bin/x".into(), "/work/app/a".into())
    );
}

#[test]
fn a_cd_pushd_or_popd_whose_destination_it_cant_tell_leaves_the_directory_be() {
    // A bare cd goes home and a bare pushd swaps the top two; `cd ''` stays put; `popd +1` drops
    // a stack entry without moving.
    for (script, dir) in [
        ("cd; bin/x", "/work/app"),
        ("pushd; bin/x", "/work/app"),
        ("cd ''; bin/x", "/work/app"),
        ("pushd /a; popd +1; bin/x", "/a"),
    ] {
        assert_eq!(
            programs_in(script).last().unwrap(),
            &("bin/x".into(), dir.into()),
            "{script}"
        );
    }
}

#[test]
fn an_empty_launcher_in_the_machine_config_matches_nothing() {
    let mut machine = machine();
    machine.launchers.insert(0, vec![]);
    let found: Vec<String> = spot("bundle exec bin/x", CWD, &machine)
        .into_iter()
        .map(|s| s.program)
        .collect();
    assert_eq!(found, ["bin/x"]);
}

#[test]
fn popd_with_nothing_pushed_leaves_the_directory_be() {
    assert_eq!(
        programs_in("cd sub; popd; bin/x").last().unwrap(),
        &("bin/x".into(), "/work/app/sub".into())
    );
}

#[test]
fn a_subshells_pushd_ends_with_the_subshell() {
    assert_eq!(
        programs_in("pushd a; (popd); popd; bin/x").last().unwrap(),
        &("bin/x".into(), "/work/app".into())
    );
}

#[test]
fn a_rooted_path_is_normalised_and_one_leaving_the_project_is_not_rooted() {
    for path in [
        "$(repo-root)/lib/../bin/x",
        "$(repo-root)/./bin/x",
        "$(repo-root)//bin//x",
    ] {
        let inside = spot(path, CWD, &machine());
        assert!(inside[0].rooted, "{path}");
        assert_eq!(inside[0].program, "bin/x", "{path}");
    }

    let outside = spot("$(repo-root)/../other/bin/x", CWD, &machine());
    assert!(!outside[0].rooted, "{outside:?}");
    assert_eq!(outside[0].program, "$(repo-root)/../other/bin/x");
}

// A heredoc a shell reads as its script (Nick, 2026-10-17).

#[test]
fn a_heredoc_fed_to_a_shell_is_spotted_as_its_script() {
    assert_eq!(
        programs("bash <<'EOF'\nbin/x run\ncd sub && bin/y\nEOF\nbin/z"),
        ["bash", "bin/x", "cd", "bin/y", "bin/z"]
    );
    assert_eq!(
        programs("sh -e <<EOF\nbin/x $(bin/y)\nEOF"),
        ["sh", "bin/x", "bin/y"]
    );
    assert_eq!(
        programs("zsh -s arg <<-EOF\n\tbin/x\n\tEOF"),
        ["zsh", "bin/x"]
    );
}

#[test]
fn a_heredoc_feeding_a_shells_script_file_or_dash_c_is_its_input_only() {
    assert_eq!(programs("bash run.sh <<'EOF'\nbin/x\nEOF"), ["bash"]);
    assert_eq!(
        programs("bash -c 'bin/y' <<'EOF'\nbin/x\nEOF"),
        ["bash", "bin/y"]
    );
}

#[test]
fn a_heredoc_belongs_to_the_command_it_follows_not_the_next_one() {
    assert_eq!(programs("cat <<EOF; bash\nbin/x\nEOF"), ["cat", "bash"]);
    assert_eq!(
        programs("bash <<A; cat <<B\nbin/x\nA\nbin/y\nB"),
        ["bash", "bin/x", "cat"]
    );
}

#[test]
fn a_heredoc_script_starts_in_the_directory_of_its_shell() {
    assert_eq!(
        programs_in("cd /elsewhere; bash <<'EOF'\nbin/x\nEOF")
            .last()
            .unwrap(),
        &("bin/x".into(), "/elsewhere".into())
    );
}

#[test]
fn the_suggestion_wraps_the_shell_reading_a_heredoc() {
    assert_eq!(
        suggestion("bash <<'EOF'\nbin/mutant run\nEOF", "bin/mutant", None),
        "maitred -- bash <<'EOF'\nbin/mutant run\nEOF"
    );
}

#[test]
fn only_a_heredoc_on_stdin_is_a_shells_script() {
    assert_eq!(programs("bash 3<<'EOF'\nbin/x\nEOF"), ["bash"]);
    assert_eq!(programs("bash 0<<'EOF'\nbin/x\nEOF"), ["bash", "bin/x"]);
}

// Mutants.

#[test]
fn a_here_strings_substitution_is_spotted_wherever_it_falls() {
    assert_eq!(programs("a; cat <<< \"$(bin/x)\""), ["a", "cat", "bin/x"]);
}

#[test]
fn a_heredoc_with_no_delimiter_line_runs_to_the_end() {
    assert_eq!(programs("bash <<EOF\nbin/x\n"), ["bash", "bin/x"]);
}

#[test]
fn an_escaped_dollar_in_a_heredoc_starts_no_substitution() {
    assert_eq!(
        programs("cat <<EOF\n\\$(bin/x) $(bin/y)\nEOF"),
        ["cat", "bin/y"]
    );
}

#[test]
fn a_substitution_over_several_lines_of_a_heredoc_is_read_once() {
    assert_eq!(
        programs("cat <<EOF\n$(echo a\necho $(bin/x))\nEOF"),
        ["cat", "echo", "echo", "bin/x"]
    );
}

#[test]
fn a_variable_in_a_heredoc_starts_no_substitution() {
    assert_eq!(
        programs("cat <<EOF\n$v bin/x\n$(bin/y)\nEOF"),
        ["cat", "bin/y"]
    );
}

#[test]
fn an_unclosed_substitution_ends_with_its_heredoc() {
    // The delimiter line ends the heredoc whatever is open in it, so bin/y is a command of its own.
    assert_eq!(
        programs("cat <<EOF\n$(bin/x\nEOF\nbin/y"),
        ["cat", "bin/x", "bin/y"]
    );
}

#[test]
fn backquotes_in_a_heredoc_pair_up_and_start_where_they_stand() {
    assert_eq!(
        programs("cat <<EOF\n`bin/x` `bin/y` x`bin/z`\nEOF"),
        ["cat", "bin/x", "bin/y", "bin/z"]
    );
}

#[test]
fn the_suggestion_replaces_a_substitution_in_a_heredoc_where_it_stands() {
    assert_eq!(
        suggestion("cat <<EOF\n$(bin/mutant run)\nEOF", "bin/mutant", None),
        "cat <<EOF\n$(maitred -- bin/mutant run)\nEOF"
    );
}

#[test]
fn the_suggestion_finds_assignments_after_the_start_of_the_string() {
    assert_eq!(
        suggestion("a; FOO=1 bin/mutant run", "bin/mutant", None),
        "a; maitred -- env FOO=1 bin/mutant run"
    );
}

#[test]
fn a_shell_with_options_and_a_script_file_reads_no_heredoc_as_its_script() {
    assert_eq!(programs("bash -e run.sh <<'EOF'\nbin/x\nEOF"), ["bash"]);
}

#[test]
fn eval_is_spotted_and_its_script_is_not() {
    // The hook notes it to the agent rather than reading the script (Nick, 2026-10-17).
    assert_eq!(programs("eval 'bin/x run'; eval bin/y"), ["eval", "eval"]);
}

// Coverage.

#[test]
fn a_substitution_in_a_case_pattern_is_spotted() {
    // The shell runs it to make the pattern.
    assert_eq!(
        programs("case $v in $(bin/x)) bin/y ;; \"`bin/z`\") ;; esac"),
        ["bin/x", "bin/y", "bin/z"]
    );
}

#[test]
fn a_backslash_in_double_quotes_escapes_only_what_the_shell_says() {
    assert_eq!(programs("\"bin/\\x\""), ["bin/\\x"]);
    assert_eq!(programs("\"bin/\\$x\" \"a\\\nb\""), ["bin/$x"]);
    assert_eq!(programs("echo \"\\$(bin/x)\" \"\\`bin/y\\`\""), ["echo"]);
}

#[test]
fn quotes_and_escapes_inside_a_substitution_dont_close_it() {
    assert_eq!(
        programs("echo $(bin/x ')' \")\" \\) ) && bin/y"),
        ["echo", "bin/x", "bin/y"]
    );
    assert_eq!(
        programs("echo `bin/x \\` b` && bin/y"),
        ["echo", "bin/x", "bin/y"]
    );
}

#[test]
fn a_substitution_inside_double_quotes_ends_where_the_shell_ends_it() {
    // What follows its `)` is quoted text, so a substitution that ran on would spot bin/y.
    assert_eq!(programs("echo \"$('a') ; bin/y\""), ["echo", "a"]);
    assert_eq!(programs("echo \"$(echo \\') ; bin/y\""), ["echo", "echo"]);
}

#[test]
fn a_backslash_before_a_character_of_several_bytes_escapes_all_of_it() {
    assert_eq!(programs("echo \"\\é\"; bin/x"), ["echo", "bin/x"]);
}

#[test]
fn an_unterminated_quote_or_substitution_runs_to_the_end() {
    assert_eq!(programs("echo \"$(bin/x"), ["echo", "bin/x"]);
    assert_eq!(programs("echo `bin/x"), ["echo", "bin/x"]);
    assert_eq!(programs("echo \"bin/x"), ["echo"]);
    assert_eq!(programs("bin/x\\"), ["bin/x"]);
}

#[test]
fn a_case_inside_parentheses_in_a_substitution_doesnt_end_it_early() {
    // MT-056: the pattern's `)` closes the pattern, never the subshell around the case. In double
    // quotes, anything a substitution ended too early would leave out is text, not commands.
    assert_eq!(
        programs("echo \"$( (case $v in x) bin/a ;; esac); bin/b)\""),
        ["echo", "bin/a", "bin/b"]
    );
    assert_eq!(
        programs("echo \"$(case $v in x) (bin/a) ;; esac; bin/b)\""),
        ["echo", "bin/a", "bin/b"]
    );
}

#[test]
fn a_substitution_where_a_case_pattern_has_no_room_for_one_is_still_spotted() {
    // Not valid shell, but the spotter errs toward spotting.
    assert_eq!(programs("case $v in a <<< $(bin/x)) ;; esac"), ["bin/x"]);
    assert_eq!(
        programs("case $v in a <<EOF\n$(bin/x)\nEOF\n) ;; esac"),
        ["bin/x"]
    );
}

#[test]
fn a_case_pattern_on_the_line_of_its_in_still_ends_at_its_paren() {
    assert_eq!(
        programs("case $x in a) bin/y ;; b) bin/z ;; esac"),
        ["bin/y", "bin/z"]
    );
    assert_eq!(
        programs("case $x in *) bin/y ;; esac; bin/z"),
        ["bin/y", "bin/z"]
    );
}

#[test]
fn a_here_string_fed_to_a_shell_is_spotted_as_its_script() {
    assert_eq!(
        programs("bash <<< 'bin/x run && bin/y'"),
        ["bash", "bin/x", "bin/y"]
    );
    assert_eq!(programs("sh <<< \"$(bin/x)\""), ["sh", OUTPUT, "bin/x"]);
    assert_eq!(programs("bash run.sh <<< 'bin/x'"), ["bash"]);
    assert_eq!(
        suggestion("bash <<< 'bin/mutant run'", "bin/mutant", None),
        "maitred -- bash <<< 'bin/mutant run'"
    );
}

// A shell's script is built by the shell that starts it, which runs the substitutions in it first: those are the
// caller's, and the script is read with each one's output standing in (Nick, 2026-10-17, MT-050).

/// Each program with the text of the simple command a refusal would replace, and its escalation.
fn credited(script: &str) -> Vec<(String, String, Option<String>)> {
    spot(script, CWD, &machine())
        .into_iter()
        .map(|s| (s.program, script[s.span].to_owned(), s.escalated))
        .collect()
}

#[test]
fn a_substitution_in_an_unquoted_heredoc_a_shell_reads_is_the_callers() {
    let script = "bash <<EOF\necho $(bin/x)\nEOF";
    assert_eq!(
        credited(script),
        [
            ("bash".into(), "bash".into(), None),
            ("echo".into(), "bash".into(), None),
            ("bin/x".into(), "bin/x".into(), None),
        ]
    );
    assert_eq!(
        suggestion(script, "bin/x", None),
        "bash <<EOF\necho $(maitred -- bin/x)\nEOF"
    );
    // The output stands in for the whole substitution, its `)` too, so `arg` stays its argument.
    assert_eq!(
        programs("bash <<EOF\n$(bin/x) arg\nEOF"),
        ["bash", OUTPUT, "bin/x"]
    );
}

#[test]
fn a_substitution_the_caller_runs_isnt_escalated_by_the_sudo_its_output_goes_to() {
    for script in [
        "sudo bash <<EOF\n$(bin/x)\nEOF",
        "sudo bash -c \"$(bin/x)\"",
        "sudo bash <<< \"$(bin/x)\"",
    ] {
        let x = spot(script, CWD, &machine())
            .into_iter()
            .find(|s| s.program == "bin/x")
            .unwrap();
        assert_eq!(x.escalated, None, "{script}");
        assert_eq!(&script[x.span], "bin/x", "{script}");
    }
}

#[test]
fn a_dash_c_strings_substitution_is_spotted_once_as_the_callers() {
    let script = "bash -c \"$(bin/x) --flag\"";
    assert_eq!(
        credited(script),
        [
            ("bash".into(), script.into(), None),
            (OUTPUT.into(), script.into(), None),
            ("bin/x".into(), "bin/x".into(), None),
        ]
    );
}

#[test]
fn a_script_with_an_expansion_in_it_is_still_read() {
    for script in [
        "bash -c \"cd $DIR && bin/x\"",
        "bash <<< \"cd $DIR && bin/x\"",
    ] {
        assert_eq!(programs(script), ["bash", "cd", "bin/x"], "{script}");
    }
    // `$v` is a program nobody can name until it runs; bin/x is its argument.
    assert_eq!(programs("bash -c \"$v bin/x\""), ["bash", "$v"]);
}

#[test]
fn a_substitution_the_outer_shell_doesnt_run_is_left_to_the_script() {
    // Single quotes, a quoted delimiter and an escaped dollar all hand `$(…)` to the inner shell as text.
    for script in [
        "bash -c 'echo $(bin/x)'",
        "bash <<'EOF'\necho $(bin/x)\nEOF",
        "bash -c \"echo \\$(bin/x)\"",
    ] {
        let spotted = spot(script, CWD, &machine());
        let span_of = |program: &str| {
            spotted
                .iter()
                .find(|s| s.program == program)
                .unwrap()
                .span
                .clone()
        };
        assert_eq!(span_of("bin/x"), span_of("bash"), "{script}");
    }
}

// sudo and doas (Nick, 2026-10-17).

#[test]
fn a_program_behind_sudo_or_doas_is_spotted_and_marked_escalated() {
    let spotted = spot("sudo -u nick bin/x run; doas bin/y; bin/z", CWD, &machine());
    let found: Vec<(&str, Option<&str>)> = spotted
        .iter()
        .map(|s| (s.program.as_str(), s.escalated.as_deref()))
        .collect();
    assert_eq!(
        found,
        [
            ("bin/x", Some("sudo")),
            ("bin/y", Some("doas")),
            ("bin/z", None)
        ]
    );
}

#[test]
fn wrappers_after_a_launcher_are_skipped_and_sudo_there_still_escalates() {
    // Nick, 2026-10-17 (MT-049).
    let spotted = spot(
        "bundle exec nice bin/x; bundle exec sudo bin/y",
        CWD,
        &machine(),
    );
    let found: Vec<(&str, Option<&str>)> = spotted
        .iter()
        .map(|s| (s.program.as_str(), s.escalated.as_deref()))
        .collect();
    assert_eq!(found, [("bin/x", None), ("bin/y", Some("sudo"))]);
}

#[test]
fn a_program_in_the_script_of_an_escalated_shell_is_escalated_too() {
    for script in ["sudo bash -c 'bin/x'", "sudo bash <<'EOF'\nbin/x\nEOF"] {
        let x = spot(script, CWD, &machine())
            .into_iter()
            .find(|s| s.program == "bin/x")
            .unwrap();
        assert_eq!(x.escalated.as_deref(), Some("sudo"), "{script}");
    }
}
