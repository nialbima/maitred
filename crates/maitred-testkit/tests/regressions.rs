//! Every regression scenario can be run as written. Running them on the simulator comes once `maitred-sim`
//! exists.

use maitred_testkit::regressions::phase1_admission;

#[test]
fn every_phase_1_scenario_is_valid() {
    for scenario in phase1_admission() {
        assert_eq!(scenario.validate(), Ok(()), "{}", scenario.name);
    }
}

#[test]
fn every_phase_1_scenario_names_the_finding_it_checks() {
    for scenario in phase1_admission() {
        assert!(
            !scenario.findings.is_empty(),
            "{} names no finding",
            scenario.name
        );
    }
}
