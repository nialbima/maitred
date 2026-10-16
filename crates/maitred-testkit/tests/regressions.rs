//! Every regression scenario can be run as written. Running them on the simulator comes once `maitred-sim`
//! exists.

use maitred_testkit::regressions::{phase1_admission, phase1_classification};

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

#[test]
fn every_phase_1_classification_case_is_valid_and_names_its_finding() {
    for case in phase1_classification() {
        assert_eq!(case.validate(), Ok(()), "{}", case.name);
        assert!(!case.findings.is_empty(), "{} names no finding", case.name);
    }
}
