//! How maitred is verified (ARCHITECTURE.md, Test kit): workloads, replayed load, faults, the invariant
//! checker and the scenarios. Owned by the test kit, never shipped, and never dependent on the `maitred`
//! crate.

pub mod checker;
pub mod load_file;
pub mod regressions;
pub mod ticket;
pub mod timing;
