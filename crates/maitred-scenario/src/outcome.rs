//! What happened in one run, as either target reports it.

use crate::time::Ms;

/// Which kill threshold a victim was chosen for (ARCHITECTURE.md: Kill policy).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trigger {
    Memory,
    Cpu,
    Gpu,
}

/// How a run ended, as its record says (ARCHITECTURE.md: Project files, `record.json`), or `Lost`, which is `wait`'s
/// answer when there is neither ticket nor record and is never written.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum End {
    Completed {
        status: u8,
    },
    Cancelled,
    TimedOut,
    Killed(Trigger),
    StoppedBySignal,
    Abandoned,
    /// Admitted, but the job couldn't be started.
    SpawnFailed,
    Interrupted,
    HolderLost,
    Expired,
    Lost,
}

/// One run's events, by job label. Entries are judged by their times, not the order they were reported in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub admissions: Vec<(String, Ms)>,
    pub kills: Vec<(String, Trigger, Ms)>,
    /// How each run ended, and when.
    pub ends: Vec<(String, End, Ms)>,
    /// Each record's flags, such as `tainted`.
    pub flags: Vec<(String, String)>,
}

impl Outcome {
    /// A run in which nothing has happened yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that `label` was admitted at `at`.
    #[must_use]
    pub fn admitted(mut self, label: &str, at: Ms) -> Self {
        self.admissions.push((label.into(), at));
        self
    }

    /// Records that `label` was chosen as a victim for `trigger` at `at`.
    #[must_use]
    pub fn killed(mut self, label: &str, trigger: Trigger, at: Ms) -> Self {
        self.kills.push((label.into(), trigger, at));
        self
    }

    /// Records how `label`'s run ended, and when.
    #[must_use]
    pub fn ended_at(mut self, label: &str, end: End, at: Ms) -> Self {
        self.ends.push((label.into(), end, at));
        self
    }

    /// Records that `label`'s record carries `flag`.
    #[must_use]
    pub fn flagged(mut self, label: &str, flag: &str) -> Self {
        self.flags.push((label.into(), flag.into()));
        self
    }

    pub(crate) fn has_flag(&self, label: &str, flag: &str) -> bool {
        self.flags.iter().any(|(l, f)| l == label && f == flag)
    }

    /// When `label` was first admitted.
    pub(crate) fn admitted_at(&self, label: &str) -> Option<Ms> {
        self.admissions
            .iter()
            .filter(|(l, _)| l == label)
            .map(|(_, at)| *at)
            .min()
    }

    /// How `label`'s run ended, by its earliest end.
    pub(crate) fn end_of(&self, label: &str) -> Option<&End> {
        self.ends
            .iter()
            .filter(|(l, _, _)| l == label)
            .min_by_key(|(_, _, at)| *at)
            .map(|(_, end, _)| end)
    }

    /// When `label` was running: from its admission to its end, or to the end of the run if it never ended.
    /// `None` if it was never admitted.
    pub(crate) fn run_of(&self, label: &str) -> Option<(Ms, Option<Ms>)> {
        let start = self.admitted_at(label)?;
        let end = self
            .ends
            .iter()
            .filter(|(l, _, _)| l == label)
            .map(|(_, _, at)| *at)
            .min();
        Some((start, end))
    }

    /// Entries no run can have, whatever the scenario expects: a job admitted, ended or killed twice, or ended
    /// before it was admitted. ARCHITECTURE has one admission, one record and at most one kill per ticket.
    pub(crate) fn impossible(&self) -> Vec<crate::expect::Failure> {
        use crate::expect::Failure;
        let mut labels: Vec<&str> = self.admissions.iter().map(|(l, _)| l.as_str()).collect();
        labels.extend(self.ends.iter().map(|(l, _, _)| l.as_str()));
        labels.extend(self.kills.iter().map(|(l, _, _)| l.as_str()));
        labels.sort_unstable();
        labels.dedup();
        let mut failures = Vec::new();
        for label in labels {
            if self.admissions.iter().filter(|(l, _)| l == label).count() > 1 {
                failures.push(Failure::AdmittedTwice(label.into()));
            }
            if self.ends.iter().filter(|(l, _, _)| l == label).count() > 1 {
                failures.push(Failure::EndedTwice(label.into()));
            }
            if self.kills.iter().filter(|(l, _, _)| l == label).count() > 1 {
                failures.push(Failure::KilledTwice(label.into()));
            }
            if let Some(ended) = self
                .ends
                .iter()
                .filter(|(l, _, _)| l == label)
                .map(|(_, _, at)| *at)
                .min()
                && self
                    .admitted_at(label)
                    .is_none_or(|admitted| ended < admitted)
            {
                failures.push(Failure::EndedBeforeAdmitted(label.into()));
            }
        }
        failures
    }
}
