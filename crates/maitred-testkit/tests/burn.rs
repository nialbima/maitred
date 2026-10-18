//! `burn` is every job in every real-process scenario, so it has to do exactly what it is told and say what
//! it did.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::Instant;
use std::{env, fs, thread};

use maitred_testkit::timing::{OUTLIVE_MS, millis, wait_ceiling};

fn burn(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_burn"));
    command.args(args);
    command
}

fn report_path(name: &str) -> PathBuf {
    env::temp_dir().join(format!("burn-test-{}-{name}.report", std::process::id()))
}

fn read_report(path: &PathBuf) -> HashMap<String, String> {
    fs::read_to_string(path)
        .expect("burn wrote no report")
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn run(command: &mut Command) -> Output {
    command.output().expect("burn didn't start")
}

#[test]
fn it_exits_with_the_status_it_was_given() {
    let output = run(&mut burn(&["--run-ms", "50", "--exit", "7"]));

    assert_eq!(output.status.code(), Some(7));
}

#[test]
fn it_prints_one_line_per_tick_while_it_runs() {
    let output = run(&mut burn(&[
        "--run-ms",
        "250",
        "--print-every-ms",
        "100",
        "--label",
        "a",
    ]));

    let stdout = String::from_utf8(output.stdout).expect("stdout isn't text");
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        vec!["burn a tick 1", "burn a tick 2"]
    );
}

#[test]
fn it_reports_its_own_process_ids() {
    let path = report_path("ids");
    let child = burn(&["--run-ms", "50", "--report", path.to_str().unwrap()])
        .spawn()
        .expect("burn didn't start");
    let pid = child.id().to_string();
    let output = child.wait_with_output().expect("burn didn't finish");

    let report = read_report(&path);
    assert!(output.status.success());
    assert_eq!(report.get("pid"), Some(&pid));
    assert!(report.contains_key("pgid"), "no pgid in {report:?}");
    assert!(report.contains_key("sid"), "no sid in {report:?}");
}

#[test]
fn it_reports_the_slot_number_maitred_gave_it() {
    let path = report_path("slot");
    run(burn(&["--run-ms", "50", "--report", path.to_str().unwrap()]).env("TEST_ENV_NUMBER", "3"));

    assert_eq!(read_report(&path).get("slot"), Some(&"3".to_string()));
}

#[test]
fn it_reports_no_slot_when_it_was_given_none() {
    let path = report_path("noslot");
    run(
        burn(&["--run-ms", "50", "--report", path.to_str().unwrap()]).env_remove("TEST_ENV_NUMBER"),
    );

    assert_eq!(read_report(&path).get("slot"), Some(&"none".to_string()));
}

#[test]
fn it_holds_a_port_and_reports_which() {
    let path = report_path("port");
    run(&mut burn(&[
        "--run-ms",
        "50",
        "--port",
        "--report",
        path.to_str().unwrap(),
    ]));

    let port: u16 = read_report(&path)
        .get("port")
        .expect("no port in the report")
        .parse()
        .expect("port isn't a number");
    assert!(port > 0);
}

#[test]
fn it_allocates_and_touches_the_memory_it_was_asked_for() {
    let path = report_path("mem");
    run(&mut burn(&[
        "--run-ms",
        "50",
        "--mem-mb",
        "64",
        "--report",
        path.to_str().unwrap(),
    ]));

    let resident_mb: u64 = read_report(&path)
        .get("peak_resident_mb")
        .expect("no peak_resident_mb in the report")
        .parse()
        .expect("peak_resident_mb isn't a number");
    assert!(
        resident_mb >= 64,
        "peak resident {resident_mb} MB, asked for 64"
    );
}

fn cpu_ms_for(name: &str, millicores: &str) -> f64 {
    let path = report_path(name);
    run(&mut burn(&[
        "--run-ms",
        "500",
        "--cpu-millicores",
        millicores,
        "--report",
        path.to_str().unwrap(),
    ]));
    let cpu_ms: u32 = read_report(&path)
        .get("cpu_ms")
        .expect("no cpu_ms in the report")
        .parse()
        .expect("cpu_ms isn't a number");
    f64::from(cpu_ms)
}

// How much CPU burn gets depends on what else the machine is running, so the accuracy checks need a quiet
// machine and are left out of an ordinary run. Run them with:
//   heavy cargo nextest run -p maitred-testkit --run-ignored only
#[test]
fn it_spends_cpu_when_asked_for_a_core() {
    // burn ends once it has used 200 ms of CPU, however long a loaded machine takes to give it that; one that didn't
    // spin would run on to the ceiling (MT-054).
    let path = report_path("cpu-spins");
    let mut child = burn(&[
        "--run-ms",
        &OUTLIVE_MS.to_string(),
        "--cpu-millicores",
        "1000",
        "--until-cpu-ms",
        "200",
        "--report",
        path.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    let started = Instant::now();
    while child.try_wait().expect("can't wait for burn").is_none() {
        if started.elapsed() >= wait_ceiling() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("1 core used under 200 ms of CPU in {:?}", wait_ceiling());
        }
        thread::sleep(millis(10));
    }

    let cpu_ms: u64 = read_report(&path)["cpu_ms"]
        .parse()
        .expect("cpu_ms isn't a number");
    assert!(cpu_ms >= 200, "it ended having used {cpu_ms} ms");
}

#[test]
#[ignore = "needs a quiet machine"]
fn two_cores_use_about_twice_the_cpu_of_one() {
    let one = cpu_ms_for("cpu-one", "1000");
    let two = cpu_ms_for("cpu-two", "2000");

    let ratio = two / one;
    assert!(
        (1.5..=2.5).contains(&ratio),
        "2 cores used {two} ms, 1 core {one} ms"
    );
}

#[test]
#[ignore = "needs a quiet machine"]
fn half_a_core_uses_about_half_the_cpu_of_one() {
    let one = cpu_ms_for("cpu-whole", "1000");
    let half = cpu_ms_for("cpu-half", "500");

    let ratio = half / one;
    assert!(
        (0.3..=0.7).contains(&ratio),
        "half a core used {half} ms, 1 core {one} ms"
    );
}

#[test]
fn no_cores_uses_next_to_no_cpu() {
    let none = cpu_ms_for("cpu-none", "0");

    assert!(none < 50.0, "asked for no CPU, used {none} ms");
}

#[test]
fn a_hold_without_its_file_or_one_the_fill_never_reaches_is_refused() {
    for args in [
        &["--mem-mb", "8", "--hold-fill-at-mb", "4"][..],
        &["--mem-mb", "8", "--held", "/never"],
        &[
            "--mem-mb",
            "8",
            "--hold-fill-at-mb",
            "8",
            "--held",
            "/never",
        ],
    ] {
        assert_eq!(run(&mut burn(args)).status.code(), Some(64), "{args:?}");
    }
}

#[test]
fn it_refuses_an_argument_it_doesnt_know() {
    let output = run(&mut burn(&["--no-such-flag"]));

    assert_eq!(output.status.code(), Some(64));
}
