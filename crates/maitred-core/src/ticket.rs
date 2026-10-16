use std::collections::{BTreeMap, BTreeSet};

use crate::Resources;

/// An instant on the boot clock, in milliseconds: the clock that keeps counting through sleep and
/// never steps (`CLOCK_BOOTTIME`, `mach_continuous_time`), shared by every process of one boot.
/// Times from another boot mean nothing here; the store compares `boot` before building a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct BootTime(pub u64);

impl BootTime {
    /// Milliseconds from `earlier` to `self`; zero if `earlier` is later.
    #[must_use]
    pub fn since(self, earlier: BootTime) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// Where a ticket sorts in the queue: `(seq, 0)` for every ticket nobody moved. A move puts a ticket
/// at another's `seq` with a smaller or larger `sub`, so it sorts just before or after that ticket
/// without anyone's `seq` changing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub seq: u64,
    pub sub: i64,
}

/// A ticket's id. Ties in `(seq, id)` order and in recency are broken by it, so every reader orders
/// tickets the same way.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TicketId(pub String);

/// The name of a slot pool the machine config declares, such as `test_env`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pool(pub String);

/// A job's class. `none` is a rule's class, not a ticket's: a `none` program never gets a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Light,
    Heavy,
    Quiet,
}

/// The states a ticket's supervisor writes. Stopping is a marker beside the ticket and the end is
/// the ticket's deletion, so neither is a state here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Waiting,
    Admitted,
    Running,
}

/// Whether a ticket's owner is alive, as the snapshot's shared try-lock found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Its supervisor holds its lock.
    Living,
    /// Its lock is free: the supervisor is gone.
    Orphaned {
        /// It was written in this boot and pid namespace, so its pids mean something here.
        this_boot: bool,
    },
}

/// One ticket as a snapshot sees it: its header and payload, and its liveness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub id: TicketId,
    pub seq: u64,
    pub class: Class,
    pub state: State,
    pub liveness: Liveness,
    /// Its holder's pid and start time still match and it isn't a zombie (only a running or
    /// stopping ticket has a holder doing anything). A holder can die while its supervisor lives.
    pub holder_alive: bool,
    pub reservation: Resources,
    /// The pools its rule asks a slot of.
    pub pools: BTreeSet<Pool>,
    /// The slot numbers it holds, once admitted.
    pub slots: BTreeMap<Pool, u32>,
    /// How long it has waited, on the clock that stops while the machine sleeps.
    pub waited_ms: u64,
    /// When its owner last started a pass.
    pub last_pass: BootTime,
    pub admitted_at: Option<BootTime>,
    pub recreated_at: Option<BootTime>,
}

impl Ticket {
    pub(crate) fn is_living(&self) -> bool {
        self.liveness == Liveness::Living
    }
}
