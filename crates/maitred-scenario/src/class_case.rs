//! Classification cases (ARCHITECTURE.md: Class files). No time and no load: a class file, a command, and
//! what one of two engines should make of it.
//!
//! - The hook reads a raw command string and lets it through or refuses it, naming the command to run instead.
//! - Argv matching takes a command as the operating system hands it over and gives the class, the rule and the
//!   slots, or a refusal.
//!
//! The simulator runs each case through maitred's core and reports the engine's answer, which
//! [`ClassCase::judge_hook`] or [`ClassCase::judge_match`] compares with the expected one.

use crate::job::Class;

/// What a case feeds the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// A command string, as an agent's harness hands it to the hook.
    Shell(String),
    /// A command's argv, as the operating system hands it to maitred.
    Argv(Vec<String>),
}

/// The hook's verdict on a command string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hook {
    /// The string runs as it is.
    Allow,
    /// The hook refuses it (exit 2) because it found `program`, a program a rule names, normalised as the
    /// class file writes it (`bin/mutant`, not `./bin/mutant`). The exact refusal message isn't checked: the
    /// doc doesn't yet fix its text for strings with wrappers, assignments or pipes.
    Refuse { program: String },
}

impl Hook {
    #[must_use]
    pub fn refuse(program: &str) -> Self {
        Self::Refuse {
            program: program.into(),
        }
    }
}

/// What argv matching makes of a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Matched {
    /// Queued in `class`, under `rule`, with these slots and record flags. As an expectation, `rule: None`
    /// accepts any rule; see [`ClassCase::judge_match`].
    Queued {
        class: Class,
        rule: Option<String>,
        slots: Vec<String>,
        flags: Vec<String>,
    },
    /// Refused before any ticket exists (exit 77): a `none` rule, or a keep-awake wrapper.
    Refused,
}

impl Matched {
    #[must_use]
    pub fn queued(class: Class) -> Self {
        Self::Queued {
            class,
            rule: None,
            slots: Vec::new(),
            flags: Vec::new(),
        }
    }

    /// Names the rule that matched.
    ///
    /// # Panics
    ///
    /// On a refusal, which has no rule.
    #[must_use]
    pub fn rule(mut self, name: &str) -> Self {
        let Self::Queued { rule, .. } = &mut self else {
            panic!("a refusal has no rule")
        };
        *rule = Some(name.into());
        self
    }

    /// Adds a slot from `pool`.
    ///
    /// # Panics
    ///
    /// On a refusal, which takes no slots.
    #[must_use]
    pub fn slot(mut self, pool: &str) -> Self {
        let Self::Queued { slots, .. } = &mut self else {
            panic!("a refusal takes no slots")
        };
        slots.push(pool.into());
        self
    }

    /// Adds a record flag, such as `no-class-file` or `classes-unverified`.
    ///
    /// # Panics
    ///
    /// On a refusal, which writes no record.
    #[must_use]
    pub fn flag(mut self, flag: &str) -> Self {
        let Self::Queued { flags, .. } = &mut self else {
            panic!("a refusal writes no record")
        };
        flags.push(flag.into());
        self
    }
}

/// The answer a case expects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expected {
    Hook(Hook),
    Match(Matched),
}

/// A reason a case can't be run as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvalidCase {
    /// The case states no expected answer.
    NoExpectation,
    /// A shell string needs a hook verdict and an argv needs a match.
    WrongExpectation,
    /// The expected match takes a slot from a pool the case doesn't declare.
    UndeclaredPool(String),
}

/// One classification case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassCase {
    pub name: String,
    /// The review findings this case checks, as FINDINGS.md names them.
    pub findings: Vec<String>,
    /// The project's `.maitred/classes`, verbatim.
    pub class_file: String,
    /// The pools the machine config declares.
    pub pools: Vec<String>,
    pub input: Input,
    pub expected: Option<Expected>,
}

impl ClassCase {
    /// A case for the hook, given a raw command string.
    #[must_use]
    pub fn hook(name: &str, class_file: &str, command: &str) -> Self {
        Self::new(name, class_file, Input::Shell(command.into()))
    }

    /// A case for argv matching.
    #[must_use]
    pub fn argv(name: &str, class_file: &str, argv: &[&str]) -> Self {
        Self::new(
            name,
            class_file,
            Input::Argv(argv.iter().map(|word| (*word).to_string()).collect()),
        )
    }

    fn new(name: &str, class_file: &str, input: Input) -> Self {
        Self {
            name: name.into(),
            findings: Vec::new(),
            class_file: class_file.into(),
            pools: Vec::new(),
            input,
            expected: None,
        }
    }

    /// Names a review finding this case checks.
    #[must_use]
    pub fn checks(mut self, finding: &str) -> Self {
        self.findings.push(finding.into());
        self
    }

    /// Declares a pool on the machine.
    #[must_use]
    pub fn pool(mut self, name: &str) -> Self {
        self.pools.push(name.into());
        self
    }

    #[must_use]
    pub fn expect_hook(mut self, verdict: Hook) -> Self {
        self.expected = Some(Expected::Hook(verdict));
        self
    }

    #[must_use]
    pub fn expect_match(mut self, matched: Matched) -> Self {
        self.expected = Some(Expected::Match(matched));
        self
    }

    /// Whether the case can be run as written.
    ///
    /// # Errors
    ///
    /// Returns the first reason it can't.
    pub fn validate(&self) -> Result<(), InvalidCase> {
        match (&self.input, &self.expected) {
            (_, None) => Err(InvalidCase::NoExpectation),
            (Input::Shell(_), Some(Expected::Hook(_)))
            | (Input::Argv(_), Some(Expected::Match(Matched::Refused))) => Ok(()),
            (Input::Argv(_), Some(Expected::Match(Matched::Queued { slots, .. }))) => {
                match slots.iter().find(|pool| !self.pools.contains(pool)) {
                    Some(pool) => Err(InvalidCase::UndeclaredPool(pool.clone())),
                    None => Ok(()),
                }
            }
            _ => Err(InvalidCase::WrongExpectation),
        }
    }

    /// Compares the hook's verdict with the expected one.
    ///
    /// # Errors
    ///
    /// Returns the expected and actual verdicts when they differ, or when the case expects a match instead.
    pub fn judge_hook(&self, actual: &Hook) -> Result<(), (Hook, Hook)> {
        match &self.expected {
            Some(Expected::Hook(expected)) if expected == actual => Ok(()),
            Some(Expected::Hook(expected)) => Err((expected.clone(), actual.clone())),
            _ => Err((Hook::Allow, actual.clone())),
        }
    }

    /// Compares argv matching's answer with the expected one: the same class and slots, the named rule if the
    /// case names one, and at least the flags it lists.
    ///
    /// # Errors
    ///
    /// Returns the expected and actual answers when they differ, or when the case expects a hook verdict.
    #[expect(
        clippy::result_large_err,
        reason = "the error is the expected and actual answers a failing case reports, built once per case"
    )]
    pub fn judge_match(&self, actual: &Matched) -> Result<(), (Matched, Matched)> {
        match &self.expected {
            Some(Expected::Match(expected)) if expected.allows(actual) => Ok(()),
            Some(Expected::Match(expected)) => Err((expected.clone(), actual.clone())),
            _ => Err((Matched::Refused, actual.clone())),
        }
    }
}

impl Matched {
    /// Whether `actual` is the answer this one expects. The class and the slots must be the same. The rule is
    /// compared only when this names one, and `actual` may carry flags beyond the ones this lists: the doc
    /// doesn't fix which rule a string of several programs reports, or every flag's name.
    fn allows(&self, actual: &Matched) -> bool {
        match (self, actual) {
            (Matched::Refused, Matched::Refused) => true,
            (
                Matched::Queued {
                    class,
                    rule,
                    slots,
                    flags,
                },
                Matched::Queued {
                    class: actual_class,
                    rule: actual_rule,
                    slots: actual_slots,
                    flags: actual_flags,
                },
            ) => {
                class == actual_class
                    && rule
                        .as_ref()
                        .is_none_or(|name| actual_rule.as_ref() == Some(name))
                    && slots == actual_slots
                    && flags.iter().all(|flag| actual_flags.contains(flag))
            }
            _ => false,
        }
    }
}
