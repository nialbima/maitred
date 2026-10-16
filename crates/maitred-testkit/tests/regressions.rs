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

/// The findings list, one line per review finding: `<report> <id> | <failure> | <disposition>`.
const FINDINGS: &str = include_str!("../FINDINGS.md");

/// Phase 1 findings whose scenario can't be written yet, each with the reason. Remove one when its scenario
/// lands; the test below fails if a finding here is already checked.
const PENDING: &[(&str, &str)] = &[(
    "r3-admission A19",
    "needs a fault that corrupts a ticket, which waits on the ticket header's encoding (MT-018)",
)];

fn finding_lines() -> impl Iterator<Item = (String, &'static str)> {
    FINDINGS
        .lines()
        .filter(|line| line.starts_with('r') && line.contains(" | "))
        .map(|line| {
            let id: Vec<&str> = line.split(' ').take(2).collect();
            let disposition = line.rsplit(" | ").next().unwrap_or("");
            (id.join(" "), disposition)
        })
}

fn checked_in_phase_1() -> Vec<String> {
    let mut checked: Vec<String> = phase1_admission()
        .into_iter()
        .flat_map(|s| s.findings)
        .collect();
    checked.extend(phase1_classification().into_iter().flat_map(|c| c.findings));
    checked
}

#[test]
fn every_finding_a_scenario_checks_is_a_scenario_line_in_the_findings_list() {
    let lines: Vec<_> = finding_lines().collect();
    for finding in checked_in_phase_1() {
        let disposition = lines.iter().find(|(id, _)| *id == finding).map_or_else(
            || panic!("{finding} isn't in FINDINGS.md"),
            |(_, disposition)| *disposition,
        );
        assert!(
            disposition.starts_with("scenario "),
            "{finding} is '{disposition}', not a scenario"
        );
    }
}

#[test]
fn every_phase_1_simulator_finding_is_checked_or_waiting_for_a_named_reason() {
    let checked = checked_in_phase_1();
    for (id, disposition) in finding_lines() {
        let phase_1_sim = disposition.starts_with("scenario ")
            && (disposition.ends_with(" sim phase 1") || disposition.ends_with(" both phase 1"));
        if !phase_1_sim {
            continue;
        }
        let pending = PENDING.iter().any(|(pending, _)| *pending == id);
        let is_checked = checked.contains(&id);
        assert!(
            is_checked || pending,
            "{id} has no phase 1 scenario and isn't listed as pending"
        );
        assert!(
            !(is_checked && pending),
            "{id} is checked now; take it off PENDING"
        );
    }
}
