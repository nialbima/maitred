//! maitred's decisions: admission, the kill policy, the ticket state machine and class-rule matching,
//! as pure functions over snapshots. Nothing here touches a file, a clock, a process, the environment
//! or the network; `clippy.toml` in this crate refuses them.
//!
//! Time arrives as plain milliseconds: instants on the boot clock as [`BootTime`], and lengths of time as `u64`
//! fields whose names end in `_ms`. A length measured on the clock that stops while the machine sleeps
//! says so in its name (`waited_ms`).

#![forbid(unsafe_code)]

mod config;
mod decide;
mod resources;
mod snapshot;
#[cfg(test)]
mod testgen;
mod ticket;
mod victim;

pub use config::{ClassMinimums, Config, ConfigError, PoolConfig};
pub use decide::{Admission, Decision, Limit, Reason, decide};
pub use resources::Resources;
pub use snapshot::{Load, Snapshot};
pub use ticket::{BootTime, Class, Liveness, Pool, Position, State, Ticket, TicketId};
pub use victim::{Trigger, choose_victim, trigger};
