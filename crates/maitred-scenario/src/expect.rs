//! What a scenario expects of a run, and how each expectation is judged against an outcome.

use crate::outcome::{End, Outcome, Trigger};
use crate::time::Ms;

/// One expectation. Order is only ever stated between two jobs: real processes poll with jitter, so a whole
/// admission sequence is more than a run can promise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    /// `first` is admitted, and `second` is not admitted before it.
    AdmittedBefore { first: String, second: String },
    /// The job is admitted at some point in the run.
    Admitted(String),
    /// The job is admitted at or before `by`.
    AdmittedBy { label: String, by: Ms },
    /// The job isn't admitted before `before`; later, or never, is fine.
    NotAdmittedBefore { label: String, before: Ms },
    /// The job is never admitted.
    NeverAdmitted(String),
    /// The two jobs are never running at the same moment.
    NeverTogether { first: String, second: String },
    /// At no moment are more than `allowed` of these jobs running.
    AtMostTogether { allowed: usize, labels: Vec<String> },
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
    /// The outcome reports one job admitted twice.
    AdmittedTwice(String),
    /// The outcome reports one job ended twice.
    EndedTwice(String),
    /// The outcome reports a job ending before it was admitted, or without being admitted.
    EndedBeforeAdmitted(String),
    NotBefore {
        first: String,
        second: String,
    },
    NotAdmitted(String),
    Admitted(String),
    NotAdmittedBy {
        label: String,
        by: Ms,
        at: Option<Ms>,
    },
    AdmittedEarly {
        label: String,
        before: Ms,
        at: Ms,
    },
    TooManyTogether {
        allowed: usize,
        most: usize,
    },
    RanTogether {
        first: String,
        second: String,
    },
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
    pub fn admitted(label: &str) -> Self {
        Self::Admitted(label.into())
    }

    #[must_use]
    pub fn never_together(first: &str, second: &str) -> Self {
        Self::NeverTogether {
            first: first.into(),
            second: second.into(),
        }
    }

    #[must_use]
    pub fn at_most_together(allowed: usize, labels: &[&str]) -> Self {
        Self::AtMostTogether {
            allowed,
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
        }
    }

    #[must_use]
    pub fn admitted_by(label: &str, by: Ms) -> Self {
        Self::AdmittedBy {
            label: label.into(),
            by,
        }
    }

    #[must_use]
    pub fn not_admitted_before(label: &str, before: Ms) -> Self {
        Self::NotAdmittedBefore {
            label: label.into(),
            before,
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
            Self::AdmittedBefore { first, second } | Self::NeverTogether { first, second } => {
                vec![first, second]
            }
            Self::Admitted(label)
            | Self::AdmittedBy { label, .. }
            | Self::NotAdmittedBefore { label, .. }
            | Self::NeverAdmitted(label)
            | Self::NotKilled(label)
            | Self::Killed { label, .. }
            | Self::Ends { label, .. } => vec![label],
            Self::AtMostTogether { labels, .. } => labels.iter().map(String::as_str).collect(),
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
            Self::Admitted(label) => outcome
                .admitted_at(label)
                .is_none()
                .then(|| Failure::NotAdmitted(label.clone())),
            Self::NeverTogether { first, second } => {
                // A run lasts from its admission to its end, or to the end of the run if it never ended.
                let overlap = match (outcome.run_of(first), outcome.run_of(second)) {
                    (Some((a_start, a_end)), Some((b_start, b_end))) => {
                        a_end.is_none_or(|end| b_start < end)
                            && b_end.is_none_or(|end| a_start < end)
                    }
                    _ => false,
                };
                overlap.then(|| Failure::RanTogether {
                    first: first.clone(),
                    second: second.clone(),
                })
            }
            Self::AtMostTogether { allowed, labels } => {
                let most = most_running_at_once(outcome, labels);
                (most > *allowed).then_some(Failure::TooManyTogether {
                    allowed: *allowed,
                    most,
                })
            }
            Self::AdmittedBy { label, by } => {
                let at = outcome.admitted_at(label);
                at.is_none_or(|at| at > *by)
                    .then(|| Failure::NotAdmittedBy {
                        label: label.clone(),
                        by: *by,
                        at,
                    })
            }
            Self::NotAdmittedBefore { label, before } => outcome
                .admitted_at(label)
                .filter(|at| at < before)
                .map(|at| Failure::AdmittedEarly {
                    label: label.clone(),
                    before: *before,
                    at,
                }),
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

/// The most of `labels` running at any one moment. A run starts at its admission and ends at its end, or never
/// within the run if it has none; a run that ends at the moment another starts doesn't overlap it.
fn most_running_at_once(outcome: &Outcome, labels: &[String]) -> usize {
    let runs: Vec<_> = labels
        .iter()
        .filter_map(|label| outcome.run_of(label))
        .collect();
    runs.iter()
        .map(|(start, _)| {
            runs.iter()
                .filter(|(other_start, other_end)| {
                    other_start <= start && other_end.is_none_or(|end| *start < end)
                })
                .count()
        })
        .max()
        .unwrap_or(0)
}
