//! The test machine keeps ARCHITECTURE's ratios between its durations when a scenario changes one.

use maitred_scenario::{MachineConfig, ms};

#[test]
fn a_longer_poll_interval_keeps_the_stall_limit_at_five_polls() {
    let machine = MachineConfig::for_tests().poll_interval(ms(400));

    assert_eq!(machine.stall_limit, ms(2_000));
}
