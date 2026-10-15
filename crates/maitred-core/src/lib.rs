//! maitred's decisions: admission, the kill policy, the ticket state machine and class-rule matching,
//! as pure functions over snapshots. Nothing here touches a file, a clock, a process, the environment
//! or the network; `clippy.toml` in this crate refuses them.

#![forbid(unsafe_code)]
