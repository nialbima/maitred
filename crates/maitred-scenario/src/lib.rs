//! The test kit's scenario type and its DSL (ARCHITECTURE.md, Test kit). Owned by the test kit, not by
//! maitred's builder; it depends on nothing of maitred's.
//!
//! A [`Scenario`] says what a run is given: a machine config, jobs that arrive, and faults. The simulator
//! (`maitred-sim`) and the real-process runner (`maitred-testkit`) each run it and report an [`Outcome`],
//! and [`Scenario::judge`] checks either one against the same expectations.

mod class_case;
mod expect;
mod fault;
mod job;
mod load;
mod machine;
mod outcome;
mod pause;
mod scenario;
mod time;

pub use class_case::{
    ClassCase, Expected, Hook, Input, InvalidCase, MatchExpect, Matched, RuleExpect,
};
pub use expect::{Expect, Failure};
pub use fault::{Fault, Process, Signal};
pub use job::{Class, Job};
pub use load::{Published, Publisher, Reading, Running, Sample};
pub use machine::{Amount, ClassConfig, MachineConfig, Platform, Pool};
pub use outcome::{End, Outcome, Trigger};
pub use pause::PausePoint;
pub use scenario::{Event, Invalid, Scenario};
pub use time::{Ms, ms, secs};
