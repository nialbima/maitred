//! Property tests: the spotter row of ARCHITECTURE.md's Testing table. Random scripts are built
//! from the shell's constructs together with the programs they run, in the order the spotter
//! reports them (a substitution's programs after its command's own).

use proptest::prelude::*;

use super::*;

#[derive(Debug, Clone)]
struct Script {
    text: String,
    programs: Vec<String>,
}

fn machine() -> Machine {
    Machine::default()
}

fn spotted(text: &str) -> Vec<String> {
    spot(text, "/work/app", &machine())
        .into_iter()
        .map(|s| s.program)
        .collect()
}

/// A simple command: a program and a few plain arguments, perhaps behind an assignment or a
/// wrapper.
fn simple() -> impl Strategy<Value = Script> {
    (
        prop_oneof![Just("bin/a"), Just("bin/b"), Just("make"), Just("./bin/c")],
        prop_oneof![
            Just(""),
            Just("FOO=1 "),
            Just("nice -n 5 "),
            Just("env A=1 ")
        ],
        proptest::collection::vec(prop_oneof![Just("x"), Just("--fast"), Just("bin/z")], 0..3),
    )
        .prop_map(|(program, before, args)| {
            let mut text = format!("{before}{program}");
            for arg in args {
                text.push(' ');
                text.push_str(arg);
            }
            Script {
                text,
                programs: vec![program.to_owned()],
            }
        })
}

/// Any script, nested a few levels: lists, pipelines, conditionals, loops, subshells, groups,
/// `case`, and command substitution, bare and in double quotes.
fn script() -> impl Strategy<Value = Script> {
    simple().prop_recursive(4, 24, 2, |inner| {
        let two = (inner.clone(), inner.clone());
        prop_oneof![
            (
                two.clone(),
                prop_oneof![
                    Just("; "),
                    Just(" && "),
                    Just(" || "),
                    Just(" | "),
                    Just(" & "),
                    Just("\n")
                ]
            )
                .prop_map(|((a, b), op)| joined(&format!("{}{op}{}", a.text, b.text), [a, b])),
            two.clone()
                .prop_map(|(a, b)| joined(&format!("if {}; then {}; fi", a.text, b.text), [a, b])),
            two.clone().prop_map(|(a, b)| joined(
                &format!("while {}; do {}; done", a.text, b.text),
                [a, b]
            )),
            two.prop_map(|(a, b)| joined(
                &format!("until {}\ndo {}\ndone", a.text, b.text),
                [a, b]
            )),
            inner
                .clone()
                .prop_map(|a| joined(&format!("( {} )", a.text), [a])),
            inner
                .clone()
                .prop_map(|a| joined(&format!("{{ {}; }}", a.text), [a])),
            inner
                .clone()
                .prop_map(|a| joined(&format!("! {}", a.text), [a])),
            inner
                .clone()
                .prop_map(|a| joined(&format!("time {}", a.text), [a])),
            inner
                .clone()
                .prop_map(|a| joined(&format!("case $v in x|y) {} ;; esac", a.text), [a])),
            inner
                .clone()
                .prop_map(|a| echoed(&format!("echo $({})", a.text), a)),
            inner.prop_map(|a| echoed(&format!("echo \"$({})\"", a.text), a)),
        ]
    })
}

fn joined<const N: usize>(text: &str, parts: [Script; N]) -> Script {
    Script {
        text: text.to_owned(),
        programs: parts.into_iter().flat_map(|p| p.programs).collect(),
    }
}

fn echoed(text: &str, inner: Script) -> Script {
    let mut programs = vec!["echo".to_owned()];
    programs.extend(inner.programs);
    Script {
        text: text.to_owned(),
        programs,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1_000, ..ProptestConfig::default() })]

    /// Every program in command position is found, in order, and nothing else.
    #[test]
    fn every_program_in_command_position_is_found(s in script()) {
        prop_assert_eq!(spotted(&s.text), s.programs, "{}", s.text);
    }

    /// Nothing inside single quotes, or inside a heredoc whose delimiter is quoted, is a program.
    #[test]
    fn nothing_quoted_is_a_program(s in script()) {
        prop_assume!(!s.text.contains('\''));
        prop_assert_eq!(spotted(&format!("echo '{}'", s.text)), ["echo"]);
        prop_assert_eq!(spotted(&format!("cat <<'EOF'\n{}\nEOF", s.text)), ["cat"]);
    }

    /// A `sh -c` string is spotted as if it were the script itself.
    #[test]
    fn a_dash_c_string_is_spotted_as_its_script(s in script()) {
        prop_assume!(!s.text.contains('\''));
        let mut expected = vec!["bash".to_owned()];
        expected.extend(s.programs);
        prop_assert_eq!(spotted(&format!("bash -c '{}'", s.text)), expected);
    }

    /// A heredoc a shell reads on stdin is spotted as if it were the script itself.
    #[test]
    fn a_heredoc_on_a_shells_stdin_is_spotted_as_its_script(s in script()) {
        let mut expected = vec!["bash".to_owned()];
        expected.extend(s.programs);
        prop_assert_eq!(spotted(&format!("bash <<'EOF'\n{}\nEOF", s.text)), expected);
    }

    /// Every simple command's span is a piece of the script that starts at a word.
    #[test]
    fn every_span_is_inside_the_script(s in script()) {
        for found in spot(&s.text, "/work/app", &machine()) {
            prop_assert!(found.span.end <= s.text.len());
            prop_assert!(found.span.start < found.span.end);
            prop_assert!(s.text.is_char_boundary(found.span.start));
        }
    }
}
