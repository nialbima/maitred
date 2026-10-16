//! The review findings as scenarios, one function per Build order phase (FINDINGS.md names each finding).
//!
//! Every scenario here is written from ARCHITECTURE.md and DESIGN.md, before the code it covers. The numbers
//! in each are worked from the doc's rules, and the comment beside them says how.
//!
//! A runner judges each case or scenario with `judge`. One marked `fails_until("MT-0NN")` is expected to fail: its
//! failure is reported as known, and its passing is reported as a failure, so a closed gap can't stay marked.

mod phase1;

pub use phase1::admission as phase1_admission;
pub use phase1::classification as phase1_classification;
