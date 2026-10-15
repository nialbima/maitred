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
}

/// A reason a scenario can't be run as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// Two jobs share a label.
    DuplicateLabel(String),
    /// A fault or an expectation names a job that never arrives.
    UnknownLabel(String),
    /// A fault acts on a job before it arrives.
    BeforeArrival { label: String, at: Ms },
    /// A job asks for a slot from a pool the machine config doesn't declare.
    UndeclaredPool { label: String, pool: String },
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
    /// What happens, in the order it happens.
    pub events: Vec<(Ms, Event)>,
    pub expectations: Vec<Expect>,
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
            Event::Fault(_) => None,
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
        let mut problems = Vec::new();
        let mut seen = HashSet::new();
        for (_, job) in self.arrivals() {
            if !seen.insert(job.label.as_str()) {
                problems.push(Invalid::DuplicateLabel(job.label.clone()));
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
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    /// Every expectation `outcome` breaks.
    ///
    /// # Errors
    ///
    /// Returns each broken expectation, in the order the scenario states them.
    pub fn judge(&self, outcome: &Outcome) -> Result<(), Vec<Failure>> {
        let failures: Vec<Failure> = self
            .expectations
            .iter()
            .filter_map(|expectation| expectation.judge(outcome))
            .collect();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures)
        }
    }
}
