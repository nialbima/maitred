//! What a scenario expects of a run, and how each expectation is judged against an outcome.

use crate::outcome::{End, Outcome, Trigger};

/// One expectation. Order is only ever stated between two jobs: real processes poll with jitter, so a whole
/// admission sequence is more than a run can promise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    /// `first` is admitted, and `second` is not admitted before it.
    AdmittedBefore { first: String, second: String },
    /// The job is never admitted.
    NeverAdmitted(String),
    /// The job is chosen as a victim for `trigger`.
    Killed { label: String, trigger: Trigger },
    /// The job is never chosen as a victim.
    NotKilled(String),
    /// No more than `allowed` victims are chosen for `trigger` in the whole run.
    AtMostKills { trigger: Trigger, allowed: usize },
    /// The job's record says it ended this way.
    Ends { label: String, end: End },
}

/// One expectation that a run broke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    NotBefore {
        first: String,
        second: String,
    },
    Admitted(String),
    NotKilled {
        label: String,
        trigger: Trigger,
    },
    Killed(String),
    TooManyKills {
        trigger: Trigger,
        allowed: usize,
        killed: usize,
    },
    EndedDifferently {
        label: String,
        expected: End,
        actual: Option<End>,
    },
}

impl Expect {
    #[must_use]
    pub fn admitted_before(first: &str, second: &str) -> Self {
        Self::AdmittedBefore {
            first: first.into(),
            second: second.into(),
        }
    }

    #[must_use]
    pub fn never_admitted(label: &str) -> Self {
        Self::NeverAdmitted(label.into())
    }

    #[must_use]
    pub fn killed(label: &str, trigger: Trigger) -> Self {
        Self::Killed {
            label: label.into(),
            trigger,
        }
    }

    #[must_use]
    pub fn not_killed(label: &str) -> Self {
        Self::NotKilled(label.into())
    }

    #[must_use]
    pub fn at_most_kills(trigger: Trigger, allowed: usize) -> Self {
        Self::AtMostKills { trigger, allowed }
    }

    #[must_use]
    pub fn ends(label: &str, end: End) -> Self {
        Self::Ends {
            label: label.into(),
            end,
        }
    }

    /// The jobs this expectation names.
    pub(crate) fn labels(&self) -> Vec<&str> {
        match self {
            Self::AdmittedBefore { first, second } => vec![first, second],
            Self::NeverAdmitted(label)
            | Self::NotKilled(label)
            | Self::Killed { label, .. }
            | Self::Ends { label, .. } => vec![label],
            Self::AtMostKills { .. } => vec![],
        }
    }

    /// The failure this expectation finds in `outcome`, if any.
    pub(crate) fn judge(&self, outcome: &Outcome) -> Option<Failure> {
        match self {
            Self::AdmittedBefore { first, second } => {
                let held = match (outcome.admitted_at(first), outcome.admitted_at(second)) {
                    (Some(a), Some(b)) => a < b,
                    (Some(_), None) => true,
                    (None, _) => false,
                };
                (!held).then(|| Failure::NotBefore {
                    first: first.clone(),
                    second: second.clone(),
                })
            }
            Self::NeverAdmitted(label) => outcome
                .admitted_at(label)
                .map(|_| Failure::Admitted(label.clone())),
            Self::Killed { label, trigger } => {
                let held = outcome
                    .kills
                    .iter()
                    .any(|(l, t, _)| l == label && t == trigger);
                (!held).then(|| Failure::NotKilled {
                    label: label.clone(),
                    trigger: *trigger,
                })
            }
            Self::NotKilled(label) => outcome
                .kills
                .iter()
                .any(|(l, _, _)| l == label)
                .then(|| Failure::Killed(label.clone())),
            Self::AtMostKills { trigger, allowed } => {
                let killed = outcome
                    .kills
                    .iter()
                    .filter(|(_, t, _)| t == trigger)
                    .count();
                (killed > *allowed).then_some(Failure::TooManyKills {
                    trigger: *trigger,
                    allowed: *allowed,
                    killed,
                })
            }
            Self::Ends { label, end } => {
                let actual = outcome.end_of(label);
                (actual != Some(end)).then(|| Failure::EndedDifferently {
                    label: label.clone(),
                    expected: end.clone(),
                    actual: actual.cloned(),
                })
            }
        }
    }
}
