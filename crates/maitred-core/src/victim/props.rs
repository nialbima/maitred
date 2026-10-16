//! Property tests over random snapshots: the `choose_victim` row of ARCHITECTURE.md's Testing table.

use proptest::prelude::*;

use super::*;
use crate::testgen::snapshot;
use crate::{Resources, Snapshot, Ticket};

fn triggers() -> impl Strategy<Value = Trigger> {
    prop_oneof![
        any::<bool>().prop_map(|critical| Trigger::Memory { critical }),
        any::<bool>().prop_map(|critical| Trigger::Cpu { critical }),
        Just(Trigger::Gpu),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

    #[test]
    fn the_victim_is_a_running_job_of_this_boot_whose_holder_is_alive(snap in snapshot(), trigger in triggers()) {
        if let Some(victim) = choose_victim(&snap, trigger) {
            let t = snap.tickets.iter().find(|t| t.id == victim).expect("the victim is one of the snapshot's tickets");
            prop_assert!(is_candidate(&snap, t));
        }
    }

    /// MT-043: the victim's holder is no more than a stall limit behind the freshest live holder of
    /// a running job of this boot (from its admission before its first pass), unless the machine
    /// woke within a stall limit.
    #[test]
    fn the_victims_holder_hasnt_fallen_behind(snap in snapshot(), trigger in triggers()) {
        if let Some(victim) = choose_victim(&snap, trigger) {
            let stall = snap.config.stall_limit_ms();
            let woke = snap.load.as_ref().and_then(|l| l.woke_at).into_iter().chain(snap.my_wake)
                .any(|w| snap.now.since(w) < stall);
            let passes = snap.load.as_ref().map(|l| l.holder_passes.clone()).unwrap_or_default();
            let pass = |t: &Ticket| passes.get(&t.id).copied().or(t.admitted_at);
            let freshest = snap.tickets.iter()
                .filter(|t| t.state == State::Running && t.holder_alive && t.is_this_boot())
                .filter_map(pass).max().expect("the victim's own holder is live");
            let t = snap.tickets.iter().find(|t| t.id == victim).expect("the victim is one of the snapshot's tickets");
            let mine = pass(t).expect("a running ticket was admitted");
            prop_assert!(woke || freshest.since(mine) <= stall);
        }
    }

    #[test]
    fn nobody_is_chosen_while_a_kill_is_under_way_or_a_ticket_was_just_recreated(snap in snapshot(), trigger in triggers()) {
        if !snap.live_stop_markers.is_empty() || snap.recreated_recently() {
            prop_assert_eq!(choose_victim(&snap, trigger), None);
            prop_assert_eq!(super::trigger(&snap), None);
        }
    }

    #[test]
    fn every_holder_names_the_same_victim_whatever_order_it_reads_the_tickets(snap in snapshot(), trigger in triggers(), seed in any::<u64>()) {
        let mut shuffled = snap.clone();
        let n = shuffled.tickets.len().max(1);
        shuffled.tickets.rotate_left(usize::try_from(seed % n as u64).unwrap_or(0));
        if seed % 2 == 1 {
            shuffled.tickets.reverse();
        }
        prop_assert_eq!(choose_victim(&snap, trigger), choose_victim(&shuffled, trigger));
        prop_assert_eq!(super::trigger(&snap), super::trigger(&shuffled));
    }
}

/// How far `t`'s use in the sample is over its reservation on the trigger's resource, if it is.
fn over_by(snap: &Snapshot, t: &Ticket, trigger: Trigger) -> Option<u64> {
    let of: fn(Resources) -> u64 = match trigger {
        Trigger::Memory { .. } => |r| r.mem_mb,
        Trigger::Cpu { .. } => |r| r.cpu_millicores,
        Trigger::Gpu => return None,
    };
    let load = snap.load.as_ref()?;
    let used = of(*load.jobs.get(&t.id)?);
    let reserved = of(snap.config.reserve(t.class, t.reservation, load.capacity));
    used.checked_sub(reserved).filter(|&by| by > 0)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

    /// Which job, not just whether: the furthest over its reservation if any candidate is over;
    /// otherwise the most recently admitted of the first class present in kill order.
    #[test]
    fn the_victim_is_the_furthest_over_or_else_the_newest_in_kill_order(snap in snapshot(), trigger in triggers()) {
        // A named victim is the next property's.
        if snap.named_victim.is_some() {
            return Ok(());
        }
        let Some(victim) = choose_victim(&snap, trigger) else { return Ok(()) };
        let candidates: Vec<&Ticket> = snap.tickets.iter().filter(|t| is_candidate(&snap, t)).collect();
        let chosen = candidates.iter().find(|t| t.id == victim).expect("the victim is a candidate");
        let furthest = candidates.iter().filter_map(|t| over_by(&snap, t, trigger)).max();
        if let Some(most) = furthest {
            prop_assert_eq!(over_by(&snap, chosen, trigger), Some(most));
            for t in &candidates {
                if over_by(&snap, t, trigger) == Some(most) {
                    prop_assert!(t.admitted_at <= chosen.admitted_at);
                }
            }
        } else {
            let critical = matches!(trigger, Trigger::Memory { critical: true } | Trigger::Cpu { critical: true });
            let order: &[Class] = if critical { &[Class::Heavy, Class::Light, Class::Quiet] } else { &[Class::Heavy, Class::Light] };
            let first = order.iter().find(|c| candidates.iter().any(|t| t.class == **c)).expect("some class is present");
            prop_assert_eq!(chosen.class, *first);
            for t in candidates.iter().filter(|t| t.class == *first) {
                prop_assert!(t.admitted_at <= chosen.admitted_at);
            }
        }
    }

    /// A victim named earlier for the breach stands while it is a candidate this rule may choose and
    /// either it is still over or no candidate is; otherwise it isn't chosen over one that is over.
    #[test]
    fn a_named_victim_stands_while_it_is_eligible(snap in snapshot(), trigger in triggers()) {
        let Some(named) = snap.named_victim.clone() else { return Ok(()) };
        let critical = matches!(trigger, Trigger::Memory { critical: true } | Trigger::Cpu { critical: true });
        let any_over = snap.tickets.iter().any(|t| is_candidate(&snap, t) && over_by(&snap, t, trigger).is_some());
        let named_ticket = snap.tickets.iter().find(|t| t.id == named && is_candidate(&snap, t));
        let named_over = named_ticket.is_some_and(|t| over_by(&snap, t, trigger).is_some());
        let eligible = named_ticket.is_some_and(|t| {
            named_over || (!any_over && (t.class != Class::Quiet || critical))
        });
        let blocked = !snap.live_stop_markers.is_empty() || snap.recreated_recently();
        if blocked {
            return Ok(());
        }
        if eligible {
            prop_assert_eq!(choose_victim(&snap, trigger), Some(named));
        } else if any_over {
            prop_assert_ne!(choose_victim(&snap, trigger), Some(named));
        }
    }
}
