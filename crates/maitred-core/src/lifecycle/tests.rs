//! One test per row of ARCHITECTURE.md's Ticket state machine table, in its order.

use super::*;

fn living(state: State) -> Situation {
    Situation {
        state,
        marker: Marker::None,
        liveness: Liveness::Living,
        record_present: false,
        session_alive: false,
        past_orphan_ttl: false,
    }
}

fn orphaned(state: State) -> Situation {
    Situation {
        liveness: Liveness::Orphaned { this_boot: true },
        ..living(state)
    }
}

fn act(by: Actor, under: &[Lock], write: Write) -> Action {
    Action {
        by,
        under: under.to_vec(),
        write,
    }
}

use Actor::{Adopter, Holder, Supervisor, Sweep};
use Lock::{Admit, Ticket};

#[test]
fn a_ticket_is_created_waiting_under_admit_lock_with_its_holder_parked() {
    assert_eq!(
        create(),
        vec![
            act(Supervisor, &[Admit, Ticket], Write::CreateTicket),
            act(Supervisor, &[Ticket], Write::StartHolder),
        ]
    );
}

#[test]
fn an_admitted_ticket_takes_its_slots_under_admit_lock() {
    assert_eq!(
        next(living(State::Waiting), Event::Admit),
        Ok(vec![
            act(Supervisor, &[Admit, Ticket], Write::TakeSlots),
            act(Supervisor, &[Admit, Ticket], Write::State(State::Admitted)),
        ])
    );
}

#[test]
fn a_job_starts_only_after_its_session_is_in_the_ticket() {
    assert_eq!(
        next(living(State::Admitted), Event::Start),
        Ok(vec![
            act(Supervisor, &[Ticket], Write::WriteSession),
            act(Supervisor, &[Ticket], Write::State(State::Running)),
            act(Supervisor, &[Ticket], Write::OpenGate),
        ])
    );
}

#[test]
fn a_named_victims_holder_marks_it_stopping_under_admit_lock() {
    assert_eq!(
        next(living(State::Running), Event::Named),
        Ok(vec![act(
            Holder,
            &[Admit, Lock::Marker],
            Write::CreateMarker
        )])
    );
}

#[test]
fn an_orphans_holder_still_marks_it_stopping() {
    assert_eq!(
        next(orphaned(State::Running), Event::Named),
        Ok(vec![act(
            Holder,
            &[Admit, Lock::Marker],
            Write::CreateMarker
        )])
    );
}

#[test]
fn a_stopped_job_ends_record_first_then_ticket_then_marker() {
    let stopping = Situation {
        marker: Marker::Held,
        ..living(State::Running)
    };

    assert_eq!(
        next(stopping, Event::Gone),
        Ok(vec![
            act(Supervisor, &[Ticket], Write::Record(Outcome::Killed)),
            act(Supervisor, &[Ticket], Write::DeleteTicket),
            act(Holder, &[Lock::Marker], Write::DeleteMarker),
        ])
    );
}

#[test]
fn a_running_job_ends_record_first() {
    assert_eq!(
        next(living(State::Running), Event::End(Outcome::Exited)),
        Ok(vec![
            act(Supervisor, &[Ticket], Write::Record(Outcome::Exited)),
            act(Supervisor, &[Ticket], Write::DeleteTicket),
        ])
    );
}

#[test]
fn a_waiting_or_admitted_ticket_ends_record_first() {
    for (state, outcome) in [
        (State::Waiting, Outcome::Cancelled),
        (State::Waiting, Outcome::TimedOut),
        (State::Admitted, Outcome::Signalled),
        (State::Admitted, Outcome::SpawnFailed),
    ] {
        assert_eq!(
            next(living(state), Event::End(outcome)),
            Ok(vec![
                act(Supervisor, &[Ticket], Write::Record(outcome)),
                act(Supervisor, &[Ticket], Write::DeleteTicket),
            ]),
            "{state:?} {outcome:?}"
        );
    }
}

#[test]
fn an_adopted_waiting_orphan_keeps_waiting_with_a_new_holder() {
    assert_eq!(
        next(orphaned(State::Waiting), Event::Adopt),
        Ok(vec![act(Adopter, &[Ticket], Write::StartHolder)])
    );
}

#[test]
fn an_adopted_admitted_orphan_waits_again_without_its_slots() {
    assert_eq!(
        next(orphaned(State::Admitted), Event::Adopt),
        Ok(vec![
            act(Adopter, &[Ticket], Write::DropSlots),
            act(Adopter, &[Ticket], Write::State(State::Waiting)),
            act(Adopter, &[Ticket], Write::StartHolder),
        ])
    );
}

#[test]
fn a_finished_orphan_is_removed_by_the_sweep() {
    for marker in [Marker::None, Marker::Free] {
        let finished = Situation {
            marker,
            record_present: true,
            ..orphaned(State::Running)
        };
        let mut writes = vec![act(Sweep, &[Ticket], Write::DeleteTicket)];
        if marker == Marker::Free {
            writes.push(act(Sweep, &[Lock::Marker], Write::DeleteMarker));
        }

        assert_eq!(next(finished, Event::Sweep), Ok(writes), "{marker:?}");
    }
}

#[test]
fn an_orphan_whose_session_is_alive_is_never_adopted() {
    for marker in [Marker::None, Marker::Held] {
        let still_running = Situation {
            marker,
            session_alive: true,
            ..orphaned(State::Running)
        };

        assert_eq!(
            next(still_running, Event::Adopt),
            Err(Refused::StillRunning),
            "{marker:?}"
        );
    }
}

#[test]
fn an_interrupted_orphan_is_recorded_and_queued_again() {
    assert_eq!(
        next(orphaned(State::Running), Event::Adopt),
        Ok(vec![
            act(Adopter, &[Ticket], Write::Record(Outcome::Interrupted)),
            act(Adopter, &[Ticket], Write::DeleteTicket),
            act(Adopter, &[Admit, Ticket], Write::CreateTicket),
            act(Adopter, &[Ticket], Write::StartHolder),
        ])
    );
}

#[test]
fn an_interrupted_orphans_free_marker_goes_after_its_ticket() {
    let was_stopping = Situation {
        marker: Marker::Free,
        ..orphaned(State::Running)
    };

    assert_eq!(
        next(was_stopping, Event::Adopt),
        Ok(vec![
            act(Adopter, &[Ticket], Write::Record(Outcome::Interrupted)),
            act(Adopter, &[Ticket], Write::DeleteTicket),
            act(Adopter, &[Lock::Marker], Write::DeleteMarker),
            act(Adopter, &[Admit, Ticket], Write::CreateTicket),
            act(Adopter, &[Ticket], Write::StartHolder),
        ])
    );
}

#[test]
fn an_expired_orphan_is_recorded_then_removed_by_the_sweep() {
    for state in [State::Waiting, State::Admitted, State::Running] {
        let expired = Situation {
            past_orphan_ttl: true,
            ..orphaned(state)
        };

        assert_eq!(
            next(expired, Event::Sweep),
            Ok(vec![
                act(Sweep, &[Ticket], Write::Record(Outcome::Expired)),
                act(Sweep, &[Ticket], Write::DeleteTicket),
            ]),
            "{state:?}"
        );
    }
}

#[test]
fn an_expired_orphan_whose_session_is_alive_is_left_alone() {
    let still_running = Situation {
        past_orphan_ttl: true,
        session_alive: true,
        ..orphaned(State::Running)
    };

    assert_eq!(
        next(still_running, Event::Sweep),
        Err(Refused::StillRunning)
    );
}

// What the table doesn't allow.

#[test]
fn a_living_ticket_is_never_adopted_or_swept() {
    for state in [State::Waiting, State::Admitted, State::Running] {
        for event in [Event::Adopt, Event::Sweep] {
            assert_eq!(
                next(living(state), event),
                Err(Refused::NotFromHere),
                "{state:?} {event:?}"
            );
        }
    }
}

#[test]
fn an_orphan_is_never_admitted_started_or_ended_by_its_dead_supervisor() {
    for state in [State::Waiting, State::Admitted, State::Running] {
        for event in [
            Event::Admit,
            Event::Start,
            Event::End(Outcome::Exited),
            Event::Gone,
        ] {
            assert_eq!(
                next(orphaned(state), event),
                Err(Refused::NotFromHere),
                "{state:?} {event:?}"
            );
        }
    }
}

#[test]
fn only_a_running_job_can_be_named_and_only_once() {
    let stopping = Situation {
        marker: Marker::Held,
        ..living(State::Running)
    };
    for at in [living(State::Waiting), living(State::Admitted), stopping] {
        assert_eq!(next(at, Event::Named), Err(Refused::NotFromHere), "{at:?}");
    }
}

#[test]
fn a_stopping_job_ends_only_through_the_kill() {
    let stopping = Situation {
        marker: Marker::Held,
        ..living(State::Running)
    };

    assert_eq!(
        next(stopping, Event::End(Outcome::Exited)),
        Err(Refused::NotFromHere)
    );
    assert_eq!(
        next(living(State::Running), Event::Gone),
        Err(Refused::NotFromHere)
    );
}

#[test]
fn a_ticket_is_admitted_once_and_started_once() {
    assert_eq!(
        next(living(State::Admitted), Event::Admit),
        Err(Refused::NotFromHere)
    );
    assert_eq!(
        next(living(State::Waiting), Event::Start),
        Err(Refused::NotFromHere)
    );
    assert_eq!(
        next(living(State::Running), Event::Start),
        Err(Refused::NotFromHere)
    );
}

#[test]
fn an_orphan_not_yet_expired_and_unfinished_is_left_by_the_sweep() {
    for state in [State::Waiting, State::Admitted, State::Running] {
        assert_eq!(
            next(orphaned(state), Event::Sweep),
            Err(Refused::NotFromHere),
            "{state:?}"
        );
    }
}

#[test]
fn a_finished_orphan_is_removed_by_wait_too() {
    let finished = Situation {
        record_present: true,
        ..orphaned(State::Running)
    };

    assert_eq!(
        next(finished, Event::Adopt),
        Ok(vec![act(Adopter, &[Ticket], Write::DeleteTicket)])
    );
}

// The three invariants (ARCHITECTURE, Ticket state machine), over every situation and every event.

fn every_situation() -> Vec<Situation> {
    let mut all = Vec::new();
    for state in [State::Waiting, State::Admitted, State::Running] {
        for marker in [Marker::None, Marker::Held, Marker::Free] {
            for liveness in [
                Liveness::Living,
                Liveness::Orphaned { this_boot: true },
                Liveness::Orphaned { this_boot: false },
            ] {
                for bits in 0..8u8 {
                    all.push(Situation {
                        state,
                        marker,
                        liveness,
                        record_present: bits & 1 != 0,
                        session_alive: bits & 2 != 0,
                        past_orphan_ttl: bits & 4 != 0,
                    });
                }
            }
        }
    }
    all
}

fn every_event() -> Vec<Event> {
    let mut all = vec![
        Event::Admit,
        Event::Start,
        Event::Named,
        Event::Gone,
        Event::Adopt,
        Event::Sweep,
    ];
    all.extend(
        [
            Outcome::Exited,
            Outcome::Cancelled,
            Outcome::TimedOut,
            Outcome::Signalled,
            Outcome::SpawnFailed,
            Outcome::Killed,
            Outcome::Interrupted,
            Outcome::Expired,
        ]
        .map(Event::End),
    );
    all
}

fn every_step() -> Vec<(Situation, Event, Vec<Action>)> {
    let mut steps = vec![];
    for at in every_situation() {
        for event in every_event() {
            if let Ok(writes) = next(at, event) {
                steps.push((at, event, writes));
            }
        }
    }
    steps.push((living(State::Waiting), Event::Admit, create()));
    steps
}

fn is_marker_write(write: Write) -> bool {
    matches!(write, Write::CreateMarker | Write::DeleteMarker)
}

#[test]
fn a_tickets_file_changes_only_under_its_lock() {
    for (at, event, writes) in every_step() {
        for a in &writes {
            if is_marker_write(a.write) {
                assert!(a.under.contains(&Lock::Marker), "{at:?} {event:?} {a:?}");
            } else {
                assert!(a.under.contains(&Lock::Ticket), "{at:?} {event:?} {a:?}");
            }
        }
    }
}

#[test]
fn no_job_runs_before_its_session_is_in_its_ticket() {
    for (at, event, writes) in every_step() {
        let gate = writes.iter().position(|a| a.write == Write::OpenGate);
        let session = writes.iter().position(|a| a.write == Write::WriteSession);
        if let Some(gate) = gate {
            assert!(session.is_some_and(|s| s < gate), "{at:?} {event:?}");
        }
    }
}

#[test]
fn no_ticket_is_deleted_before_its_record_exists() {
    for (at, event, writes) in every_step() {
        let deleted = writes.iter().position(|a| a.write == Write::DeleteTicket);
        let recorded = writes
            .iter()
            .position(|a| matches!(a.write, Write::Record(_)));
        if let Some(deleted) = deleted {
            assert!(
                at.record_present || recorded.is_some_and(|r| r < deleted),
                "{at:?} {event:?}"
            );
        }
    }
}

#[test]
fn a_marker_goes_only_after_its_ticket() {
    for (at, event, writes) in every_step() {
        if let Some(marker) = writes.iter().position(|a| a.write == Write::DeleteMarker) {
            let ticket = writes.iter().position(|a| a.write == Write::DeleteTicket);
            assert!(ticket.is_some_and(|t| t < marker), "{at:?} {event:?}");
        }
    }
}

#[test]
fn only_a_holder_deletes_a_marker_whose_lock_is_held() {
    for (at, event, writes) in every_step() {
        for a in &writes {
            if a.write == Write::DeleteMarker && at.marker == Marker::Held {
                assert_eq!(a.by, Actor::Holder, "{at:?} {event:?}");
            }
        }
    }
}

#[test]
fn only_its_supervisor_and_holder_act_on_a_living_ticket() {
    for (at, event, writes) in every_step() {
        if at.liveness == Liveness::Living {
            for a in &writes {
                assert!(
                    matches!(a.by, Actor::Supervisor | Actor::Holder),
                    "{at:?} {event:?} {a:?}"
                );
            }
        }
    }
}

#[test]
fn a_job_whose_session_is_alive_is_never_ended_by_anyone_else() {
    for (at, event, writes) in every_step() {
        if at.session_alive && !at.record_present && at.state == State::Running {
            for a in &writes {
                assert!(
                    !matches!(a.by, Actor::Adopter | Actor::Sweep),
                    "{at:?} {event:?} {a:?}"
                );
            }
        }
    }
}

#[test]
fn only_a_job_that_ran_can_exit() {
    for state in [State::Waiting, State::Admitted] {
        assert_eq!(
            next(living(state), Event::End(Outcome::Exited)),
            Err(Refused::NotFromHere),
            "{state:?}"
        );
    }
}

#[test]
fn only_an_admitted_ticket_can_fail_to_spawn() {
    for state in [State::Waiting, State::Running] {
        assert_eq!(
            next(living(state), Event::End(Outcome::SpawnFailed)),
            Err(Refused::NotFromHere),
            "{state:?}"
        );
    }
}

#[test]
fn a_supervisor_never_ends_its_own_ticket_as_killed_interrupted_or_expired() {
    // Those outcomes come only from the kill, an adoption and the sweep.
    for state in [State::Waiting, State::Admitted, State::Running] {
        for outcome in [Outcome::Killed, Outcome::Interrupted, Outcome::Expired] {
            assert_eq!(
                next(living(state), Event::End(outcome)),
                Err(Refused::NotFromHere),
                "{state:?} {outcome:?}"
            );
        }
    }
}
