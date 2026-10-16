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
use crate::machine::Amount;

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
    /// Queued in `class`, under `rule` (`None` when no rule matched), with these slots, record flags and reservation.
    Queued {
        class: Class,
        rule: Option<String>,
        slots: Vec<String>,
        flags: Vec<String>,
        reservation: Option<Amount>,
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
            reservation: None,
        }
    }

    /// Sets the reservation matching gave the job.
    ///
    /// # Panics
    ///
    /// On a refusal, which has no reservation.
    #[must_use]
    pub fn reservation(mut self, cpu_millicores: u32, mem_mb: u32) -> Self {
        let Self::Queued { reservation, .. } = &mut self else {
            panic!("a refusal has no reservation")
        };
        *reservation = Some(Amount {
            cpu_millicores,
            mem_mb,
        });
        self
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

/// Which rule a case expects to match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleExpect {
    /// Any rule, or none: for strings of several programs, where the doc doesn't fix which rule is reported.
    Any,
    /// No rule matched.
    None,
    /// This rule.
    Named(String),
}

/// What a case expects argv matching to answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchExpect {
    /// Queued in `class`, under `rule`, with exactly these slots (in any order), at least the `flags`, none of the
    /// `without_flags`, and the `reservation` if one is named.
    Queued {
        class: Class,
        rule: RuleExpect,
        slots: Vec<String>,
        flags: Vec<String>,
        without_flags: Vec<String>,
        reservation: Option<Amount>,
    },
    Refused,
}

impl MatchExpect {
    /// Queued in `class`, under any rule, with no slots.
    #[must_use]
    pub fn queued(class: Class) -> Self {
        Self::Queued {
            class,
            rule: RuleExpect::Any,
            slots: Vec::new(),
            flags: Vec::new(),
            without_flags: Vec::new(),
            reservation: None,
        }
    }

    /// Expects this reservation: the strictest class's, or the largest of the programs a shell string runs.
    ///
    /// # Panics
    ///
    /// On a refusal.
    #[must_use]
    pub fn reserves(mut self, cpu_millicores: u32, mem_mb: u32) -> Self {
        let Self::Queued { reservation, .. } = &mut self else {
            panic!("a refusal has no reservation")
        };
        *reservation = Some(Amount {
            cpu_millicores,
            mem_mb,
        });
        self
    }

    #[must_use]
    pub fn refused() -> Self {
        Self::Refused
    }

    fn queued_mut(
        &mut self,
    ) -> (
        &mut RuleExpect,
        &mut Vec<String>,
        &mut Vec<String>,
        &mut Vec<String>,
    ) {
        let Self::Queued {
            rule,
            slots,
            flags,
            without_flags,
            ..
        } = self
        else {
            panic!("a refusal has no rule, slots or flags")
        };
        (rule, slots, flags, without_flags)
    }

    /// Expects this rule.
    ///
    /// # Panics
    ///
    /// On a refusal.
    #[must_use]
    pub fn rule(mut self, name: &str) -> Self {
        *self.queued_mut().0 = RuleExpect::Named(name.into());
        self
    }

    /// Expects that no rule matched.
    ///
    /// # Panics
    ///
    /// On a refusal.
    #[must_use]
    pub fn no_rule(mut self) -> Self {
        *self.queued_mut().0 = RuleExpect::None;
        self
    }

    /// Expects a slot from `pool`.
    ///
    /// # Panics
    ///
    /// On a refusal.
    #[must_use]
    pub fn slot(mut self, pool: &str) -> Self {
        self.queued_mut().1.push(pool.into());
        self
    }

    /// Expects this record flag.
    ///
    /// # Panics
    ///
    /// On a refusal.
    #[must_use]
    pub fn flag(mut self, flag: &str) -> Self {
        self.queued_mut().2.push(flag.into());
        self
    }

    /// Expects this record flag to be absent.
    ///
    /// # Panics
    ///
    /// On a refusal.
    #[must_use]
    pub fn without_flag(mut self, flag: &str) -> Self {
        self.queued_mut().3.push(flag.into());
        self
    }

    /// Whether `actual` is an answer this expectation accepts.
    fn allows(&self, actual: &Matched) -> bool {
        match (self, actual) {
            (Self::Refused, Matched::Refused) => true,
            (
                Self::Queued {
                    class,
                    rule,
                    slots,
                    flags,
                    without_flags,
                    reservation,
                },
                Matched::Queued {
                    class: actual_class,
                    rule: actual_rule,
                    slots: actual_slots,
                    flags: actual_flags,
                    reservation: actual_reservation,
                },
            ) => {
                let rule_ok = match rule {
                    RuleExpect::Any => true,
                    RuleExpect::None => actual_rule.is_none(),
                    RuleExpect::Named(name) => actual_rule.as_ref() == Some(name),
                };
                let mut expected_slots = slots.clone();
                let mut got_slots = actual_slots.clone();
                expected_slots.sort();
                got_slots.sort();
                class == actual_class
                    && rule_ok
                    && expected_slots == got_slots
                    && flags.iter().all(|flag| actual_flags.contains(flag))
                    && without_flags
                        .iter()
                        .all(|flag| !actual_flags.contains(flag))
                    && reservation.is_none_or(|amount| *actual_reservation == Some(amount))
            }
            _ => false,
        }
    }
}

/// The answer a case expects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expected {
    Hook(Hook),
    Match(MatchExpect),
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
    /// The triage entry tracking why this case fails against the doc as it stands. A runner reports such a case's
    /// failure as known and its passing as news: the entry may be ready to close.
    pub known_gap: Option<String>,
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
            known_gap: None,
        }
    }

    /// Names a review finding this case checks.
    #[must_use]
    pub fn checks(mut self, finding: &str) -> Self {
        self.findings.push(finding.into());
        self
    }

    /// Marks the case as failing until the triage entry `entry` is fixed.
    #[must_use]
    pub fn fails_until(mut self, entry: &str) -> Self {
        self.known_gap = Some(entry.into());
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
    pub fn expect_match(mut self, matched: MatchExpect) -> Self {
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
            | (Input::Argv(_), Some(Expected::Match(MatchExpect::Refused))) => Ok(()),
            (Input::Argv(_), Some(Expected::Match(MatchExpect::Queued { slots, .. }))) => {
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

    /// Compares argv matching's answer with the expected one (see [`MatchExpect`]). Call [`ClassCase::validate`]
    /// first: a case of the wrong kind fails here with a refusal standing in for the expectation.
    ///
    /// # Errors
    ///
    /// Returns the expected and actual answers when they differ, or when the case expects a hook verdict.
    #[expect(
        clippy::result_large_err,
        reason = "the error is the expected and actual answers a failing case reports, built once per case"
    )]
    pub fn judge_match(&self, actual: &Matched) -> Result<(), (MatchExpect, Matched)> {
        match &self.expected {
            Some(Expected::Match(expected)) if expected.allows(actual) => Ok(()),
            Some(Expected::Match(expected)) => Err((expected.clone(), actual.clone())),
            _ => Err((MatchExpect::Refused, actual.clone())),
        }
    }
}
