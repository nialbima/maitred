//! A scenario: a machine, what happens to it over time, and what the run must show.

use std::collections::HashSet;

use crate::expect::{Expect, Failure};
use crate::fault::Fault;
use crate::job::Job;
use crate::load::Sample;
use crate::machine::MachineConfig;
use crate::outcome::Outcome;
use crate::time::Ms;

/// Something that happens at a moment in the scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A worker runs `maitred -- <the job>`.
    Arrive(Job),
    /// Something outside maitred acts on a job.
    Fault(Fault),
    /// The whole machine sleeps until `wakes_at`. Nothing runs while it sleeps, and a job's `runs_for` counts
    /// only time awake. On waking, maitred's processes resume in an order the seed chooses, and the first sample
    /// after waking carries `woke_at`.
    Sleep { wakes_at: Ms },
    /// The sampler stops, so no load file is published, until `until` (or to the end of the run). The last file
    /// published goes stale.
    SamplerDown { until: Option<Ms> },
    /// A swap storm: until `until`, every holder passes no more often than every `holder_every`, while the sampler
    /// keeps its interval (ARCHITECTURE rev 172, MT-043). The real runner makes it with SIGSTOP and CONT duty cycles on
    /// the holders.
    Storm { until: Ms, holder_every: Ms },
}

/// A reason a scenario can't be run as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// maitred would refuse the machine config, by the rule named (ARCHITECTURE.md, Settle and the minimums).
    ConfigRefused(&'static str),
    /// Two jobs share a label.
    DuplicateLabel(String),
    /// A job alternates its use with a period of zero.
    NoPeriod(String),
    /// A fault or an expectation names a job that never arrives.
    UnknownLabel(String),
    /// A fault acts on a job before it arrives.
    BeforeArrival { label: String, at: Ms },
    /// A job asks for a slot from a pool the machine config doesn't declare.
    UndeclaredPool { label: String, pool: String },
    /// An `at_most_together` allows as many jobs as it names, so it can never fail.
    CantFail { allowed: usize, jobs: usize },
    /// Two expectations about one job can't both hold.
    Contradicts(String),
    /// A job expected never to start, or a time named past the last event, needs `run_until`: otherwise the run may
    /// end before the expectation can be judged.
    NeedsAnEnd,
    /// An expectation can never fail, given the others (for example `never_together` with a job never admitted).
    NeverFails(String),
    /// The run stops before a time an expectation names, so the expectation can't be judged.
    StopsBeforeExpectation { until: Ms, named: Ms },
    /// The run stops before its last event.
    StopsBeforeLastEvent { until: Ms, last: Ms },
    /// A sampler outage ends no later than it starts.
    OutageEndsBeforeItStarts { at: Ms, until: Ms },
    /// A storm ends no later than it starts.
    StormEndsBeforeItStarts { at: Ms, until: Ms },
    /// A storm whose holders pass no less often than the poll interval slows nothing.
    StormSlowsNothing { holder_every: Ms },
    /// A sleep wakes no later than it starts.
    SleepEndsBeforeItStarts { at: Ms, wakes_at: Ms },
    /// A strict order is expected between two jobs that arrive less than a poll interval apart, which
    /// real processes can't promise.
    OrderTooClose {
        first: String,
        second: String,
        apart: Ms,
    },
}

/// One scenario. Built with the methods below; both targets take it as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scenario {
    pub name: String,
    /// The review findings this scenario checks, as `FINDINGS.md` names them (`r4-state S1`).
    pub findings: Vec<String>,
    /// Drives every random choice either target makes, such as poll jitter; printed when a run fails.
    pub seed: u64,
    pub machine: MachineConfig,
    /// The background load, sample by sample, in time order. Between two samples the earlier one holds.
    pub trace: Vec<Sample>,
    /// What happens, in the order it happens. Events at one moment happen in the order they were added. Both
    /// targets publish the moment's load sample (the trace's sample due then, if the sampler is up) before any
    /// event at that moment, so a job arriving at 0 meets a load file, not the no-load fallback.
    pub events: Vec<(Ms, Event)>,
    pub expectations: Vec<Expect>,
    /// When the run stops. `None` means once every job that started has ended and no event is left. A scenario
    /// expecting a job never to start needs one, since that job never ends.
    pub until: Option<Ms>,
}

impl Scenario {
    #[must_use]
    pub fn new(name: &str, machine: MachineConfig) -> Self {
        Self {
            name: name.into(),
            findings: Vec::new(),
            seed: 0,
            machine,
            trace: Vec::new(),
            events: Vec::new(),
            expectations: Vec::new(),
            until: None,
        }
    }

    /// Names a review finding this scenario checks.
    #[must_use]
    pub fn checks(mut self, finding: &str) -> Self {
        self.findings.push(finding.into());
        self
    }

    #[must_use]
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Adds a sample of background load to the trace.
    #[must_use]
    pub fn load(mut self, sample: Sample) -> Self {
        let index = self.trace.partition_point(|s| s.at <= sample.at);
        self.trace.insert(index, sample);
        self
    }

    /// A worker submits `job` at `at`.
    #[must_use]
    pub fn arrive(self, at: Ms, job: Job) -> Self {
        self.at(at, Event::Arrive(job))
    }

    /// Stops the run at `until`.
    #[must_use]
    pub fn run_until(mut self, until: Ms) -> Self {
        self.until = Some(until);
        self
    }

    /// The sampler stops at `at`, until `until` or for the rest of the run.
    #[must_use]
    pub fn sampler_down(self, at: Ms, until: Option<Ms>) -> Self {
        self.at(at, Event::SamplerDown { until })
    }

    /// The machine sleeps from `at` until `wakes_at`.
    #[must_use]
    pub fn sleep(self, at: Ms, wakes_at: Ms) -> Self {
        self.at(at, Event::Sleep { wakes_at })
    }

    /// A storm from `at` to `until`, in which every holder passes no more often than every `holder_every`.
    #[must_use]
    pub fn storm(self, at: Ms, until: Ms, holder_every: Ms) -> Self {
        self.at(
            at,
            Event::Storm {
                until,
                holder_every,
            },
        )
    }

    /// `fault` happens at `at`.
    #[must_use]
    pub fn fault(self, at: Ms, fault: Fault) -> Self {
        self.at(at, Event::Fault(fault))
    }

    #[must_use]
    pub fn expect(mut self, expectation: Expect) -> Self {
        self.expectations.push(expectation);
        self
    }

    fn at(mut self, at: Ms, event: Event) -> Self {
        let index = self.events.partition_point(|(t, _)| *t <= at);
        self.events.insert(index, (at, event));
        self
    }

    /// The jobs that arrive, with when each arrives.
    pub fn arrivals(&self) -> impl Iterator<Item = (Ms, &Job)> {
        self.events.iter().filter_map(|(at, event)| match event {
            Event::Arrive(job) => Some((*at, job)),
            Event::Fault(_)
            | Event::Sleep { .. }
            | Event::SamplerDown { .. }
            | Event::Storm { .. } => None,
        })
    }

    fn arrival_of(&self, label: &str) -> Option<Ms> {
        self.arrivals()
            .find(|(_, job)| job.label == label)
            .map(|(at, _)| at)
    }

    /// Every reason the scenario can't be run, in the order the events and expectations give them.
    ///
    /// # Errors
    ///
    /// Returns each problem found; an empty scenario or a sound one returns `Ok`.
    pub fn validate(&self) -> Result<(), Vec<Invalid>> {
        let mut problems = self.machine.refused();
        let mut seen = HashSet::new();
        for (_, job) in self.arrivals() {
            if !seen.insert(job.label.as_str()) {
                problems.push(Invalid::DuplicateLabel(job.label.clone()));
            }
            if job.alternate.is_some_and(|(period, _)| period.0 == 0) {
                problems.push(Invalid::NoPeriod(job.label.clone()));
            }
            for pool in &job.slots {
                if !self.machine.declares_pool(pool) {
                    problems.push(Invalid::UndeclaredPool {
                        label: job.label.clone(),
                        pool: pool.clone(),
                    });
                }
            }
        }
        for (at, event) in &self.events {
            if let Event::SamplerDown { until: Some(until) } = event
                && until <= at
            {
                problems.push(Invalid::OutageEndsBeforeItStarts {
                    at: *at,
                    until: *until,
                });
            }
            if let Event::Sleep { wakes_at } = event
                && wakes_at <= at
            {
                problems.push(Invalid::SleepEndsBeforeItStarts {
                    at: *at,
                    wakes_at: *wakes_at,
                });
            }
            if let Event::Storm {
                until,
                holder_every,
            } = event
            {
                if until <= at {
                    problems.push(Invalid::StormEndsBeforeItStarts {
                        at: *at,
                        until: *until,
                    });
                }
                if *holder_every <= self.machine.poll_interval {
                    problems.push(Invalid::StormSlowsNothing {
                        holder_every: *holder_every,
                    });
                }
            }
            if let Event::Fault(fault) = event {
                match self.arrival_of(fault.label()) {
                    None => problems.push(Invalid::UnknownLabel(fault.label().into())),
                    Some(arrival) if arrival > *at => {
                        problems.push(Invalid::BeforeArrival {
                            label: fault.label().into(),
                            at: *at,
                        });
                    }
                    Some(_) => {}
                }
            }
        }
        for expectation in &self.expectations {
            for label in expectation.labels() {
                if !seen.contains(label) {
                    problems.push(Invalid::UnknownLabel(label.into()));
                }
            }
            if let Expect::AdmittedBefore { first, second } = expectation
                && let (Some(a), Some(b)) = (self.arrival_of(first), self.arrival_of(second))
                && a.apart(b) < self.machine.poll_interval
            {
                problems.push(Invalid::OrderTooClose {
                    first: first.clone(),
                    second: second.clone(),
                    apart: a.apart(b),
                });
            }
        }
        if let (Some(until), Some((last, _))) = (self.until, self.events.last())
            && until < *last
        {
            problems.push(Invalid::StopsBeforeLastEvent { until, last: *last });
        }
        problems.extend(self.repeated_labels());
        problems.extend(self.contradictions());
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    /// Expectations that name one job twice, and an `at_most_together` that allows every job it names.
    fn repeated_labels(&self) -> Vec<Invalid> {
        let mut problems = Vec::new();
        for expectation in &self.expectations {
            match expectation {
                Expect::NeverTogether { first, second } if first == second => {
                    problems.push(Invalid::DuplicateLabel(first.clone()));
                }
                Expect::OneKilledBy { labels, .. } => problems.extend(duplicates(labels).0),
                Expect::AtMostTogether { allowed, labels } => {
                    let (repeats, jobs) = duplicates(labels);
                    problems.extend(repeats);
                    if *allowed >= jobs {
                        problems.push(Invalid::CantFail {
                            allowed: *allowed,
                            jobs,
                        });
                    }
                }
                _ => {}
            }
        }
        problems
    }

    /// Expectations that can't all hold, that can't fail, or that can't be judged within the run.
    fn contradictions(&self) -> Vec<Invalid> {
        let mut problems = Vec::new();
        let named_past_last_event = self
            .expectations
            .iter()
            .filter_map(named_time)
            .any(|named| self.events.last().is_none_or(|(last, _)| named > *last));
        if self.until.is_none()
            && (named_past_last_event
                || self
                    .expectations
                    .iter()
                    .any(|e| matches!(e, Expect::NeverAdmitted(_))))
        {
            problems.push(Invalid::NeedsAnEnd);
        }
        for expectation in &self.expectations {
            match expectation {
                Expect::NeverAdmitted(label) => {
                    if self
                        .expectations
                        .iter()
                        .any(|other| implies_admitted(other, label))
                    {
                        problems.push(Invalid::Contradicts(label.clone()));
                    }
                    let together = self.expectations.iter().any(|other| {
                        matches!(other, Expect::NeverTogether { first, second } if first == label || second == label)
                    });
                    if together {
                        problems.push(Invalid::NeverFails(label.clone()));
                    }
                }
                Expect::AdmittedBefore { first, second } => {
                    // `first` can't be admitted before its earliest allowed time, nor `second` after its latest.
                    let impossible = matches!(
                        (self.earliest(first), self.latest(second)),
                        (Some(earliest), Some(latest)) if latest < earliest
                    );
                    if impossible {
                        problems.push(Invalid::Contradicts(first.clone()));
                    }
                }
                Expect::AtMostTogether { allowed, labels } => {
                    let never: Vec<&String> = labels
                        .iter()
                        .filter(|label| self.expectations.contains(&Expect::never_admitted(label)))
                        .collect();
                    if let Some(first) = never.first()
                        && labels.len() - never.len() <= *allowed
                    {
                        problems.push(Invalid::NeverFails((*first).clone()));
                    }
                }
                Expect::Killed { label, trigger } => {
                    let no_kills = self
                        .expectations
                        .iter()
                        .any(|other| matches!(other, Expect::AtMostKills { trigger: t, allowed: 0 } if t == trigger));
                    if self.expectations.contains(&Expect::not_killed(label)) || no_kills {
                        problems.push(Invalid::Contradicts(label.clone()));
                    }
                }
                Expect::OneKilledBy {
                    labels, trigger, ..
                } => {
                    let no_kills = self
                        .expectations
                        .iter()
                        .any(|other| matches!(other, Expect::AtMostKills { trigger: t, allowed: 0 } if t == trigger));
                    if no_kills && let Some(first) = labels.first() {
                        problems.push(Invalid::Contradicts(first.clone()));
                    }
                }
                Expect::AdmittedBy { label, by }
                    if self.earliest(label).is_some_and(|earliest| *by < earliest)
                        || self.arrival_of(label).is_some_and(|arrival| *by < arrival) =>
                {
                    problems.push(Invalid::Contradicts(label.clone()));
                }
                _ => {}
            }
            if let (Some(until), Some(named)) = (self.until, named_time(expectation))
                && until < named
            {
                problems.push(Invalid::StopsBeforeExpectation { until, named });
            }
        }
        if let Some(label) = self.order_cycle() {
            problems.push(Invalid::Contradicts(label));
        }
        problems
    }

    /// A job on a cycle of `admitted_before` expectations (a before b, b before c, c before a), which no run can
    /// satisfy: the least such label, so the problem is reported once.
    fn order_cycle(&self) -> Option<String> {
        let edges: Vec<(&str, &str)> = self
            .expectations
            .iter()
            .filter_map(|e| match e {
                Expect::AdmittedBefore { first, second } => Some((first.as_str(), second.as_str())),
                _ => None,
            })
            .collect();
        let reaches = |from: &str, to: &str| {
            let mut seen = vec![from];
            let mut frontier = vec![from];
            while let Some(at) = frontier.pop() {
                for (a, b) in &edges {
                    if *a == at && !seen.contains(b) {
                        if *b == to {
                            return true;
                        }
                        seen.push(b);
                        frontier.push(b);
                    }
                }
            }
            false
        };
        edges
            .iter()
            .filter(|(first, second)| reaches(second, first))
            .flat_map(|(first, second)| [*first, *second])
            .min()
            .map(str::to_string)
    }

    /// The latest `not_admitted_before` time on `label`: it can't be admitted earlier.
    fn earliest(&self, label: &str) -> Option<Ms> {
        self.expectations
            .iter()
            .filter_map(|e| match e {
                Expect::NotAdmittedBefore { label: l, before } if l == label => Some(*before),
                _ => None,
            })
            .max()
    }

    /// The earliest `admitted_by` time on `label`: it must be admitted by then.
    fn latest(&self, label: &str) -> Option<Ms> {
        self.expectations
            .iter()
            .filter_map(|e| match e {
                Expect::AdmittedBy { label: l, by } if l == label => Some(*by),
                _ => None,
            })
            .min()
    }

    /// Every expectation `outcome` breaks.
    ///
    /// # Errors
    ///
    /// Returns each broken expectation, in the order the scenario states them.
    pub fn judge(&self, outcome: &Outcome) -> Result<(), Vec<Failure>> {
        let mut failures: Vec<Failure> = self
            .expectations
            .iter()
            .filter_map(|expectation| expectation.judge(outcome))
            .collect();
        failures.splice(0..0, outcome.impossible());
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures)
        }
    }
}

/// Each label named more than once in `labels`, and how many different jobs it names.
fn duplicates(labels: &[String]) -> (Vec<Invalid>, usize) {
    let mut seen = HashSet::new();
    let repeats = labels
        .iter()
        .filter(|label| !seen.insert(label.as_str()))
        .map(|label| Invalid::DuplicateLabel(label.clone()))
        .collect();
    (repeats, seen.len())
}

/// The time an expectation names, if it names one.
fn named_time(expectation: &Expect) -> Option<Ms> {
    match expectation {
        Expect::AdmittedBy { by, .. } | Expect::OneKilledBy { by, .. } => Some(*by),
        Expect::NotAdmittedBefore { before, .. } => Some(*before),
        _ => None,
    }
}

/// Whether `expectation` can only hold if `label` is admitted.
fn implies_admitted(expectation: &Expect, label: &str) -> bool {
    match expectation {
        Expect::Admitted(l)
        | Expect::AdmittedBy { label: l, .. }
        | Expect::Killed { label: l, .. } => l == label,
        Expect::AdmittedBefore { first, .. } => first == label,
        _ => false,
    }
}
