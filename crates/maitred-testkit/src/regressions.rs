//! The review findings as scenarios, one function per Build order phase (FINDINGS.md names each finding).
//!
//! Every scenario here is written from ARCHITECTURE.md and DESIGN.md, before the code it covers. The numbers
//! in each are worked from the doc's rules, and the comment beside them says how.

mod phase1;

pub use phase1::admission as phase1_admission;
