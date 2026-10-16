//! Property tests over random snapshots: the `choose_victim` row of ARCHITECTURE.md's Testing table.

use proptest::prelude::*;

use super::*;
use crate::testgen::snapshot;

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
            prop_assert!(is_candidate(t));
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
    fn a_quiet_job_is_chosen_only_when_the_trigger_is_critical_or_it_is_over(snap in snapshot(), trigger in triggers()) {
        if let Some(victim) = choose_victim(&snap, trigger) {
            let t = snap.tickets.iter().find(|t| t.id == victim).expect("the victim is one of the snapshot's tickets");
            let critical = matches!(trigger, Trigger::Memory { critical: true } | Trigger::Cpu { critical: true });
            let measured_over = trigger != Trigger::Gpu && snap.load.as_ref().is_some_and(|l| l.jobs.contains_key(&t.id));
            prop_assert!(t.class != Class::Quiet || critical || measured_over);
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
