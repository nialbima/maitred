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
    /// The job's session, or its holder, is still alive.
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
    /// Its supervisor ends it: the job ended, or it was cancelled, timed out, abandoned, stopped by
    /// signal or lost its holder, or the spawn failed.
    End(Outcome),
    /// `maitred wait` adopts it, confirmed at a terminal.
    Adopt,
    /// A later supervisor's sweep, of the same uid and project.
    Sweep,
}

/// How a run ended, as its record says (ARCHITECTURE, Project files, `record.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The job's main process exited.
    Completed,
    Cancelled,
    TimedOut,
    /// TERM, HUP, INT or QUIT to the supervisor before the job ran.
    Abandoned,
    /// TERM or HUP to the supervisor while the job ran.
    StoppedBySignal,
    /// The job couldn't be started.
    SpawnFailed,
    /// Stopped by the kill policy.
    Killed,
    /// Orphaned while running, and its session gone with no record: queued again on adoption.
    Interrupted,
    /// The holder died while the job ran: the supervisor stopped the session.
    HolderLost,
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
    /// A kill is under way, and it wins: the supervisor lets it finish, and the record says killed,
    /// noting the cancel or signal that also came (Nick, 2026-10-16).
    KillUnderWay,
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
        // A running job is named by the kill policy and its holder acts: living, or an orphan
        // whose holder still runs it.
        Event::Named if running && at.marker == Marker::None && (living || at.session_alive) => Ok(
            vec![act(Holder, &[Admit, Lock::Marker], Write::CreateMarker)],
        ),
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
            (State::Running, Marker::Held, Event::Gone) => {
                let mut writes =
                    ended(Supervisor, Outcome::Killed, at.record_present, Marker::None);
                writes.push(act(Holder, &[Lock::Marker], Write::DeleteMarker));
                Ok(writes)
            }
            // The holder died mid-kill: the supervisor stops the session itself (Kill policy).
            (State::Running, Marker::Free, Event::End(Outcome::HolderLost)) => Ok(ended(
                Supervisor,
                Outcome::HolderLost,
                at.record_present,
                Marker::Free,
            )),
            (
                State::Running,
                Marker::Held,
                Event::End(Outcome::Cancelled | Outcome::StoppedBySignal),
            ) => Err(Refused::KillUnderWay),
            (state, Marker::None, Event::End(outcome)) if ends(state, outcome) => {
                Ok(ended(Supervisor, outcome, at.record_present, Marker::None))
            }
            _ => Err(Refused::NotFromHere),
        },
        // An orphan's kill: with no supervisor, its holder records it, and the job isn't queued
        // again (Nick, 2026-10-16).
        Event::Gone if running && at.marker == Marker::Held => Ok(ended(
            Holder,
            Outcome::Killed,
            at.record_present,
            Marker::Free,
        )),
        // An orphan whose record exists: its supervisor died between the record and the deletion,
        // in any state (The sweep).
        Event::Adopt if at.record_present => Ok(removed(Adopter, at.marker)),
        Event::Sweep if at.record_present => Ok(removed(Sweep, at.marker)),
        Event::Adopt => match at.state {
            State::Waiting => Ok(vec![act(Adopter, &[Ticket], Write::StartHolder)]),
            State::Admitted => Ok(vec![
                act(Adopter, &[Ticket], Write::DropSlots),
                act(Adopter, &[Ticket], Write::State(State::Waiting)),
                act(Adopter, &[Ticket], Write::StartHolder),
            ]),
            // Running or stopping: never while its session or a kill lives, or the job would run
            // twice, or a job being shed would be queued again.
            State::Running if at.session_alive || at.marker == Marker::Held => {
                Err(Refused::StillRunning)
            }
            // A marker nobody holds: a kill was started, so it ended killed, not interrupted.
            State::Running if at.marker == Marker::Free => {
                Ok(ended(Adopter, Outcome::Killed, false, Marker::Free))
            }
            State::Running => {
                let mut writes = ended(Adopter, Outcome::Interrupted, false, at.marker);
                writes.extend(created_by(Adopter));
                Ok(writes)
            }
        },
        Event::Sweep if running && (at.session_alive || at.marker == Marker::Held) => {
            Err(Refused::StillRunning)
        }
        Event::Sweep if running && at.marker == Marker::Free => {
            Ok(ended(Sweep, Outcome::Killed, false, Marker::Free))
        }
        Event::Sweep if at.past_orphan_ttl => Ok(ended(Sweep, Outcome::Expired, false, at.marker)),
        _ => Err(Refused::NotFromHere),
    }
}

/// Whether a supervisor may end its own ticket from `state` with `outcome`, per ARCHITECTURE's
/// Signals table: a signal before the job runs abandons it and one after stops it; only a run can
/// complete or lose its holder; only an admitted ticket can fail to spawn, and only a waiting one
/// can time out. Killed, interrupted and
/// expired come only from the kill, an adoption and the sweep.
fn ends(state: State, outcome: Outcome) -> bool {
    let ran = state == State::Running;
    match outcome {
        Outcome::Cancelled => true,
        // --timeout passes only before admission (exit 75).
        Outcome::TimedOut => state == State::Waiting,
        Outcome::Completed | Outcome::StoppedBySignal | Outcome::HolderLost => ran,
        Outcome::Abandoned => !ran,
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

/// Record first, unless one already exists (records are created exclusively), then the ticket,
/// then a dead holder's marker.
fn ended(by: Actor, outcome: Outcome, record_present: bool, marker: Marker) -> Vec<Action> {
    let mut writes = Vec::new();
    if !record_present {
        writes.push(act(by, &[Lock::Ticket], Write::Record(outcome)));
    }
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
