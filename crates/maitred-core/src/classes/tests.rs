//! ARCHITECTURE.md's Class files: Reading the class file (5, 6) and Format.

use super::*;

fn pools(names: &[&str]) -> BTreeSet<Pool> {
    names.iter().map(|n| Pool((*n).into())).collect()
}

fn invalid(text: &str) -> Invalid {
    parse(text, &pools(&["test_env"])).expect_err("the file is invalid")
}

fn one_rule(text: &str) -> Rule {
    let file = parse(text, &pools(&["test_env"])).expect("the file is valid");
    assert_eq!(file.rules.len(), 1, "{file:?}");
    file.rules.into_iter().next().unwrap()
}

#[test]
fn the_documented_example_parses() {
    let text = "\
# name      | pattern                         | class   | why                                        | options
e2e         | ^bin/e2e( |$)                   | heavy   | boots the app server and drives a browser  | leftovers=keep mem=3G slot=test_env
specs       | ^bin/rspec( |$)                 | light   | its cache database and ports come from a slot | slot=test_env
dataset     | ^bin/process-dataset( |$)       | heavy   | cost depends on the dataset                | checker=bin/dataset-cost
";
    let file = parse(text, &pools(&["test_env"])).unwrap();

    let names: Vec<_> = file.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["e2e", "specs", "dataset"]);
    let e2e = &file.rules[0];
    assert_eq!(e2e.line, 2);
    assert_eq!(e2e.program, "bin/e2e");
    assert_eq!(e2e.class, RuleClass::Heavy);
    assert_eq!(e2e.why, "boots the app server and drives a browser");
    assert_eq!(e2e.options.leftovers, Leftovers::Keep);
    assert_eq!(e2e.options.mem_mb, Some(3_072));
    assert_eq!(e2e.options.slots, pools(&["test_env"]));
    assert_eq!(
        file.rules[2].options.checker.as_deref(),
        Some("bin/dataset-cost")
    );
    assert_eq!(file.queue, None);
    assert!(file.skipped.is_empty());
}

#[test]
fn a_bar_inside_a_pattern_is_not_a_separator() {
    let rule = one_rule("e2e | ^bin/e2e( |$) | heavy | boots a browser");

    assert!(rule.pattern.is_match("bin/e2e"));
    assert!(rule.pattern.is_match("bin/e2e --headless"));
    assert!(!rule.pattern.is_match("bin/e2e-old"));
}

#[test]
fn options_default_to_none_and_stop() {
    let rule = one_rule("specs | ^bin/rspec( |$) | light | fast");

    assert_eq!(rule.options, Options::default());
    assert_eq!(rule.options.leftovers, Leftovers::Stop);
}

#[test]
fn every_class_and_none_are_accepted() {
    for (word, class) in [
        ("light", RuleClass::Light),
        ("heavy", RuleClass::Heavy),
        ("quiet", RuleClass::Quiet),
        ("none", RuleClass::None),
    ] {
        let rule = one_rule(&format!("x | ^bin/x | {word} | why"));
        assert_eq!(rule.class, class, "{word}");
    }
}

#[test]
fn cpu_is_in_cores_and_mem_in_megabytes_or_gigabytes() {
    let rule = one_rule("x | ^bin/x | heavy | why | cpu=0.5 mem=1200M");
    assert_eq!(rule.options.cpu_millicores, Some(500));
    assert_eq!(rule.options.mem_mb, Some(1_200));

    let rule = one_rule("x | ^bin/x | heavy | why | cpu=2 mem=2G");
    assert_eq!(rule.options.cpu_millicores, Some(2_000));
    assert_eq!(rule.options.mem_mb, Some(2_048));
}

#[test]
fn comments_and_blank_lines_are_skipped_and_lines_counted() {
    let text = "\n# a comment\n   # an indented one\n\nx | ^bin/x | light | why\n";

    assert_eq!(one_rule(text).line, 5);
}

#[test]
fn a_queue_line_first_names_where_the_queue_lives() {
    let text = "# the app's container\nqueue: docker compose exec app\nx | ^bin/x | light | why\n";
    let file = parse(text, &pools(&[])).unwrap();

    assert_eq!(file.queue.as_deref(), Some("docker compose exec app"));
    assert_eq!(file.rules.len(), 1);
}

#[test]
fn a_rule_asking_for_an_undeclared_pool_is_skipped_with_a_warning() {
    let text = "a | ^bin/a | light | why | slot=gpu\nb | ^bin/b | light | why\n";
    let file = parse(text, &pools(&["test_env"])).unwrap();

    let names: Vec<_> = file.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["b"]);
    assert_eq!(
        file.skipped,
        vec![Skipped {
            line: 1,
            rule: "a".into(),
            pool: Pool("gpu".into()),
        }]
    );
}

#[test]
fn a_rule_may_take_slots_from_several_pools() {
    let file = parse(
        "x | ^bin/x | light | why | slot=db slot=port",
        &pools(&["db", "port"]),
    )
    .unwrap();

    assert_eq!(file.rules[0].options.slots, pools(&["db", "port"]));
}

#[test]
fn the_program_is_the_literal_path_the_pattern_starts_with() {
    for (pattern, program) in [
        ("^bin/x", "bin/x"),
        ("^bin/x( |$)", "bin/x"),
        ("^make( -C [^ ]+)? +release", "make"),
        ("^bin/x\\.rb( |$)", "bin/x.rb"),
        ("^/usr/local/bin/mutant", "/usr/local/bin/mutant"),
        ("^bin/process-dataset$", "bin/process-dataset"),
    ] {
        let rule = one_rule(&format!("x | {pattern} | heavy | why"));
        assert_eq!(rule.program, program, "{pattern}");
    }
}

// What makes the file invalid (ARCHITECTURE: every command is then heavy, with a warning naming
// the bad line).

#[test]
fn a_rule_without_a_why_is_invalid() {
    for text in [
        "x | ^bin/x | heavy",
        "x | ^bin/x | heavy |   ",
        "x | ^bin/x | heavy |  | cpu=1",
    ] {
        assert_eq!(
            invalid(text),
            Invalid {
                line: 1,
                why: Why::NoWhy,
            },
            "{text}"
        );
    }
}

#[test]
fn a_line_with_too_few_or_too_many_fields_is_invalid() {
    for text in ["x | ^bin/x", "x | ^bin/x | heavy | why | cpu=1 | extra"] {
        assert_eq!(invalid(text).why, Why::Fields, "{text}");
    }
}

#[test]
fn a_rule_without_a_name_is_invalid() {
    assert_eq!(invalid(" | ^bin/x | heavy | why").why, Why::NoName);
}

#[test]
fn two_rules_with_one_name_are_invalid() {
    let text = "x | ^bin/x | heavy | why\nx | ^bin/y | light | why\n";

    assert_eq!(
        invalid(text),
        Invalid {
            line: 2,
            why: Why::DuplicateName("x".into()),
        }
    );
}

#[test]
fn a_pattern_that_doesnt_compile_is_invalid() {
    assert!(matches!(
        invalid("x | ^bin/x( | heavy | why").why,
        Why::BadPattern(_)
    ));
}

#[test]
fn a_pattern_that_doesnt_start_with_its_programs_literal_path_is_invalid() {
    for pattern in [
        "bin/x",
        "^.*x",
        "^(bin/x|bin/y)",
        "^bin/xx*",
        "^bin/x?",
        "^[a-z]+",
        "^",
        // An escaped letter is a class, not a literal.
        "^bin/\\d",
    ] {
        assert_eq!(
            invalid(&format!("x | {pattern} | heavy | why")).why,
            Why::NoProgram,
            "{pattern}"
        );
    }
}

#[test]
fn an_unknown_class_is_invalid() {
    assert_eq!(
        invalid("x | ^bin/x | huge | why").why,
        Why::UnknownClass("huge".into())
    );
}

#[test]
fn an_unknown_option_is_invalid() {
    for (options, key) in [("colour=red", "colour"), ("keep", "keep")] {
        assert_eq!(
            invalid(&format!("x | ^bin/x | heavy | why | {options}")).why,
            Why::UnknownOption(key.into()),
            "{options}"
        );
    }
}

#[test]
fn an_options_bad_value_is_invalid() {
    for (key, value) in [
        ("cpu", "0"),
        ("cpu", "-1"),
        ("cpu", "1.2345"),
        ("cpu", "two"),
        ("cpu", "+1"),
        ("mem", "+3M"),
        ("mem", "0M"),
        ("mem", "3"),
        ("mem", "3T"),
        ("mem", "1.5G"),
        ("leftovers", "maybe"),
        ("checker", ""),
        ("checker", "/usr/bin/cost"),
        ("checker", "../other/cost"),
        ("checker", "bin/../../cost"),
        ("slot", ""),
    ] {
        assert_eq!(
            invalid(&format!("x | ^bin/x | heavy | why | {key}={value}")).why,
            Why::BadValue {
                key: key.into(),
                value: value.into(),
            },
            "{key}={value}"
        );
    }
}

#[test]
fn an_option_given_twice_is_invalid() {
    for options in [
        "cpu=1 cpu=2",
        "slot=test_env slot=test_env",
        "leftovers=keep leftovers=stop",
    ] {
        assert!(
            matches!(
                invalid(&format!("x | ^bin/x | heavy | why | {options}")).why,
                Why::RepeatedOption(_)
            ),
            "{options}"
        );
    }
}

#[test]
fn a_queue_line_after_a_rule_is_not_a_queue_line() {
    let text = "x | ^bin/x | light | why\nqueue: docker compose exec app\n";

    assert_eq!(
        invalid(text),
        Invalid {
            line: 2,
            why: Why::Fields,
        }
    );
}

#[test]
fn an_empty_queue_line_is_invalid() {
    assert_eq!(invalid("queue:   \n").why, Why::EmptyQueue);
}

#[test]
fn leftovers_can_be_said_to_stop_explicitly() {
    let rule = one_rule("x | ^bin/x | heavy | why | leftovers=stop");

    assert_eq!(rule.options.leftovers, Leftovers::Stop);
}

#[test]
fn cpu_takes_up_to_three_decimal_places() {
    let rule = one_rule("x | ^bin/x | heavy | why | cpu=1.125");

    assert_eq!(rule.options.cpu_millicores, Some(1_125));
}
