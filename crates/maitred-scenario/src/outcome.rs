//! What happened in one run, as either target reports it.

use crate::time::Ms;

/// Which kill threshold a victim was chosen for (ARCHITECTURE.md: Kill policy).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trigger {
    Memory,
    Cpu,
    Gpu,
}

/// How a run ended, as its record says (ARCHITECTURE.md: Project files, `record.json`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum End {
    Completed { status: u8 },
    Cancelled,
    TimedOut,
    Killed(Trigger),
    StoppedBySignal,
    Abandoned,
    Interrupted,
    HolderLost,
    Expired,
    Lost,
}

/// One run's events, by job label.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub admissions: Vec<(String, Ms)>,
    pub kills: Vec<(String, Trigger, Ms)>,
    /// How each run ended, and when, if the target knows.
    pub ends: Vec<(String, End, Option<Ms>)>,
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

    /// Records how `label`'s run ended.
    #[must_use]
    pub fn ended(mut self, label: &str, end: End) -> Self {
        self.ends.push((label.into(), end, None));
        self
    }

    /// Records how `label`'s run ended, and when.
    #[must_use]
    pub fn ended_at(mut self, label: &str, end: End, at: Ms) -> Self {
        self.ends.push((label.into(), end, Some(at)));
        self
    }

    pub(crate) fn admitted_at(&self, label: &str) -> Option<Ms> {
        self.admissions
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, at)| *at)
    }

    pub(crate) fn end_of(&self, label: &str) -> Option<&End> {
        self.ends
            .iter()
            .find(|(l, _, _)| l == label)
            .map(|(_, end, _)| end)
    }

    /// When `label` was running: from its admission to its end, or to the end of the run if it never ended.
    /// `None` if it was never admitted.
    pub(crate) fn run_of(&self, label: &str) -> Option<(Ms, Option<Ms>)> {
        let start = self.admitted_at(label)?;
        let end = self
            .ends
            .iter()
            .find(|(l, _, _)| l == label)
            .and_then(|(_, _, at)| *at);
        Some((start, end))
    }
}
