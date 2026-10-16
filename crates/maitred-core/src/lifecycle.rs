//! The ticket state machine (ARCHITECTURE, Ticket state machine): given where a ticket stands and
//! what happened, the writes that move it on, in order, each with who makes it and under which
//! locks. Pure: the supervisor, holder and sweep carry the writes out.
//!
//! The order is what the three invariants hang on: a ticket's file changes only under its lock, no
//! job runs before its session is in its ticket, and no ticket is deleted before its record exists.

use crate::{Liveness, State};

/// Where a ticket stands, as a reader sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Situation {
    /// The state its supervisor last wrote.
    pub state: State,
    /// Whether the job's holder has marked it stopping, and whether that marker's lock is held.
    pub marker: Marker,
    /// Computed from the ticket's lock: orphaned is never written.
    pub liveness: Liveness,
    /// Observations only an orphan's outcome needs, made by whoever acts on it.
    pub record_present: bool,
    pub session_alive: bool,
    pub past_orphan_ttl: bool,
}

/// The stop marker beside a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    None,
    /// Its holder holds its lock: a kill is under way.
    Held,
    /// Its lock is free: its holder is gone.
    Free,
}

/// What happened to a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// `decide` admitted it.
    Admit,
    /// Its supervisor starts the job.
    Start,
    /// `choose_victim` named it.
    Named,
    /// A victim's processes are gone (or the bounded wait has passed), and the sampler has
    /// published a sample whose interval began after that.
    Gone,
    /// Its job exited, or it was cancelled, timed out or signalled, or the spawn failed.
    End(Outcome),
    /// `maitred wait` adopts it, confirmed at a terminal.
    Adopt,
    /// A later supervisor's sweep, of the same uid and project.
    Sweep,
}

/// How a run ended, as its record says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Exited,
    Cancelled,
    TimedOut,
    Signalled,
    SpawnFailed,
    /// Stopped by the kill policy.
    Killed,
    /// Orphaned while running, and its session gone with no record: queued again on adoption.
    Interrupted,
    /// Orphaned past `orphan_ttl`.
    Expired,
}

/// Who makes a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Supervisor,
    Holder,
    /// The supervisor that adopts an orphan, and holds its lock from then on.
    Adopter,
    /// A later supervisor's sweep (or `wait`), of the same uid and project.
    Sweep,
}

/// A lock held while a write is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    Admit,
    /// The ticket's own lock.
    Ticket,
    /// The stop marker's lock.
    Marker,
}

/// One write to the files, or one step a process takes that the order depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    /// Create the ticket with the next `seq`, locked, and renamed into place.
    CreateTicket,
    /// Start the holder, parked at its gate.
    StartHolder,
    TakeSlots,
    /// A waiting ticket holds no slots.
    DropSlots,
    State(State),
    /// The holder's pid, session and start time, into the ticket.
    WriteSession,
    /// Open the holder's gate: it forks the job and execs the command.
    OpenGate,
    CreateMarker,
    /// Created exclusively, so two writers can never both write one.
    Record(Outcome),
    DeleteTicket,
    DeleteMarker,
}

/// A write, who makes it, and under which locks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub by: Actor,
    pub under: Vec<Lock>,
    pub write: Write,
}

/// Why an event can't move a ticket from where it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The job is still running: adopting it would run it twice. Its holder still measures it and
    /// carries out a kill.
    StillRunning,
    /// Nothing in the table moves a ticket from here on this event.
    NotFromHere,
}

/// The writes that create a ticket: (none) to waiting.
#[must_use]
pub fn create() -> Vec<Action> {
    created_by(Actor::Supervisor)
}

/// The writes that move a ticket on from `at` when `event` happens, in order.
///
/// # Errors
///
/// `Refused` when the table has no such transition.
pub fn next(at: Situation, event: Event) -> Result<Vec<Action>, Refused> {
    use Actor::{Adopter, Holder, Supervisor, Sweep};
    use Lock::{Admit, Ticket};

    let living = at.liveness == Liveness::Living;
    let running = at.state == State::Running;
    match event {
        // A running job, living or orphaned, is named by the kill policy; its holder acts.
        Event::Named if running && at.marker == Marker::None => Ok(vec![act(
            Holder,
            &[Admit, Lock::Marker],
            Write::CreateMarker,
        )]),
        // Everything else a living ticket does is its supervisor's.
        _ if living => match (at.state, at.marker, event) {
            (State::Waiting, Marker::None, Event::Admit) => Ok(vec![
                act(Supervisor, &[Admit, Ticket], Write::TakeSlots),
                act(Supervisor, &[Admit, Ticket], Write::State(State::Admitted)),
            ]),
            (State::Admitted, Marker::None, Event::Start) => Ok(vec![
                act(Supervisor, &[Ticket], Write::WriteSession),
                act(Supervisor, &[Ticket], Write::State(State::Running)),
                act(Supervisor, &[Ticket], Write::OpenGate),
            ]),
            (State::Running, Marker::Held, Event::Gone) => Ok(vec![
                act(Supervisor, &[Ticket], Write::Record(Outcome::Killed)),
                act(Supervisor, &[Ticket], Write::DeleteTicket),
                act(Holder, &[Lock::Marker], Write::DeleteMarker),
            ]),
            (state, Marker::None, Event::End(outcome)) if ends(state, outcome) => {
                Ok(ended(Supervisor, outcome, Marker::None))
            }
            _ => Err(Refused::NotFromHere),
        },
        // An orphan: adopted, or swept.
        Event::Adopt => match at.state {
            State::Waiting => Ok(vec![act(Adopter, &[Ticket], Write::StartHolder)]),
            State::Admitted => Ok(vec![
                act(Adopter, &[Ticket], Write::DropSlots),
                act(Adopter, &[Ticket], Write::State(State::Waiting)),
                act(Adopter, &[Ticket], Write::StartHolder),
            ]),
            // Running or stopping: never while its session lives, or the job would run twice.
            State::Running if at.session_alive => Err(Refused::StillRunning),
            State::Running if at.record_present => Ok(removed(Adopter, at.marker)),
            State::Running => {
                let mut writes = ended(Adopter, Outcome::Interrupted, at.marker);
                writes.extend(created_by(Adopter));
                Ok(writes)
            }
        },
        Event::Sweep if running && at.record_present => Ok(removed(Sweep, at.marker)),
        Event::Sweep if running && at.session_alive => Err(Refused::StillRunning),
        Event::Sweep if at.past_orphan_ttl => Ok(ended(Sweep, Outcome::Expired, at.marker)),
        _ => Err(Refused::NotFromHere),
    }
}

/// Whether a supervisor may end its own ticket from `state` with `outcome`. Only a job that ran can
/// exit, only an admitted one can fail to spawn, and killed, interrupted and expired come only from
/// the kill, an adoption and the sweep.
fn ends(state: State, outcome: Outcome) -> bool {
    match outcome {
        Outcome::Cancelled | Outcome::TimedOut | Outcome::Signalled => true,
        Outcome::Exited => state == State::Running,
        Outcome::SpawnFailed => state == State::Admitted,
        Outcome::Killed | Outcome::Interrupted | Outcome::Expired => false,
    }
}

/// A new waiting ticket and its parked holder.
fn created_by(by: Actor) -> Vec<Action> {
    vec![
        act(by, &[Lock::Admit, Lock::Ticket], Write::CreateTicket),
        act(by, &[Lock::Ticket], Write::StartHolder),
    ]
}

/// Record first, then the ticket, then a dead holder's marker.
fn ended(by: Actor, outcome: Outcome, marker: Marker) -> Vec<Action> {
    let mut writes = vec![act(by, &[Lock::Ticket], Write::Record(outcome))];
    writes.extend(removed(by, marker));
    writes
}

/// The record already exists: the ticket, then a dead holder's marker.
fn removed(by: Actor, marker: Marker) -> Vec<Action> {
    let mut writes = vec![act(by, &[Lock::Ticket], Write::DeleteTicket)];
    if marker == Marker::Free {
        writes.push(act(by, &[Lock::Marker], Write::DeleteMarker));
    }
    writes
}

fn act(by: Actor, under: &[Lock], write: Write) -> Action {
    Action {
        by,
        under: under.to_vec(),
        write,
    }
}

#[cfg(test)]
mod tests;
