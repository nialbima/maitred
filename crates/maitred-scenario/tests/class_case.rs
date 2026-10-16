//! Classification cases: a class file and a command, and what the hook or argv matching should make of it.

use maitred_scenario::{Class, ClassCase, Hook, InvalidCase, Matched};

const CLASSES: &str = "\
# name    | pattern            | class | why
mutation  | ^bin/mutant( |$)   | heavy | every core for hours
specs     | ^bin/rspec( |$)    | light | its databases come from a slot | slot=test_env
";

#[test]
fn a_hook_case_passes_when_the_hook_gives_the_expected_verdict() {
    let case = ClassCase::hook(
        "a heavy program after if",
        CLASSES,
        "if bin/mutant run; then echo ok; fi",
    )
    .expect_hook(Hook::refuse("bin/mutant"));

    assert_eq!(case.judge_hook(&Hook::refuse("bin/mutant")), Ok(()));
}

#[test]
fn a_hook_case_fails_with_both_verdicts_when_they_differ() {
    let case = ClassCase::hook(
        "a heavy program after if",
        CLASSES,
        "if bin/mutant run; then echo ok; fi",
    )
    .expect_hook(Hook::refuse("bin/mutant"));

    assert_eq!(
        case.judge_hook(&Hook::Allow),
        Err((Hook::refuse("bin/mutant"), Hook::Allow))
    );
}

#[test]
fn a_match_case_passes_when_the_class_rule_and_slots_agree() {
    let expected = Matched::queued(Class::Light).rule("specs").slot("test_env");
    let case = ClassCase::argv(
        "specs behind bundle exec",
        CLASSES,
        &["bundle", "exec", "bin/rspec"],
    )
    .pool("test_env")
    .expect_match(expected.clone());

    assert_eq!(case.judge_match(&expected), Ok(()));
}

#[test]
fn a_match_case_fails_when_the_slots_differ() {
    let case = ClassCase::argv(
        "specs behind bundle exec",
        CLASSES,
        &["bundle", "exec", "bin/rspec"],
    )
    .pool("test_env")
    .expect_match(Matched::queued(Class::Light).rule("specs").slot("test_env"));
    let actual = Matched::queued(Class::Light).rule("specs");

    assert_eq!(
        case.judge_match(&actual),
        Err((
            Matched::queued(Class::Light).rule("specs").slot("test_env"),
            actual
        ))
    );
}

#[test]
fn a_hook_case_must_expect_a_hook_verdict() {
    let case = ClassCase::hook("wrong kind of answer", CLASSES, "bin/mutant run")
        .expect_match(Matched::queued(Class::Heavy));

    assert_eq!(case.validate(), Err(InvalidCase::WrongExpectation));
}

#[test]
fn a_match_case_must_expect_a_match() {
    let case = ClassCase::argv("wrong kind of answer", CLASSES, &["bin/mutant", "run"])
        .expect_hook(Hook::Allow);

    assert_eq!(case.validate(), Err(InvalidCase::WrongExpectation));
}

#[test]
fn a_case_with_no_expectation_is_refused() {
    let case = ClassCase::argv("nothing expected", CLASSES, &["bin/mutant", "run"]);

    assert_eq!(case.validate(), Err(InvalidCase::NoExpectation));
}

#[test]
fn a_match_case_whose_expected_slot_comes_from_an_undeclared_pool_is_refused() {
    let case = ClassCase::argv("undeclared pool", CLASSES, &["bin/rspec"])
        .expect_match(Matched::queued(Class::Light).rule("specs").slot("test_env"));

    assert_eq!(
        case.validate(),
        Err(InvalidCase::UndeclaredPool("test_env".into()))
    );
}

#[test]
fn a_case_that_names_no_rule_accepts_any_rule() {
    let case = ClassCase::argv(
        "two programs in one string",
        CLASSES,
        &["sh", "-c", "bin/rspec && bin/mutant run"],
    )
    .pool("test_env")
    .expect_match(Matched::queued(Class::Heavy).slot("test_env"));

    assert_eq!(
        case.judge_match(
            &Matched::queued(Class::Heavy)
                .rule("mutation")
                .slot("test_env")
        ),
        Ok(())
    );
}

#[test]
fn a_case_that_names_a_rule_rejects_a_different_one() {
    let case = ClassCase::argv("specs", CLASSES, &["bin/rspec"])
        .pool("test_env")
        .expect_match(Matched::queued(Class::Light).rule("specs").slot("test_env"));
    let actual = Matched::queued(Class::Light)
        .rule("mutation")
        .slot("test_env");

    assert_eq!(
        case.judge_match(&actual),
        Err((
            Matched::queued(Class::Light).rule("specs").slot("test_env"),
            actual
        ))
    );
}

#[test]
fn flags_beyond_the_ones_a_case_lists_are_allowed() {
    let case = ClassCase::argv(
        "an invalid class file",
        "no why here | ^x | heavy",
        &["bin/rspec"],
    )
    .expect_match(Matched::queued(Class::Heavy));

    assert_eq!(
        case.judge_match(&Matched::queued(Class::Heavy).flag("invalid-classes")),
        Ok(())
    );
}

#[test]
fn a_flag_a_case_lists_must_be_there() {
    let case = ClassCase::argv("no class file", "", &["bin/rspec"])
        .expect_match(Matched::queued(Class::Heavy).flag("no-class-file"));
    let actual = Matched::queued(Class::Heavy);

    assert_eq!(
        case.judge_match(&actual),
        Err((Matched::queued(Class::Heavy).flag("no-class-file"), actual))
    );
}
