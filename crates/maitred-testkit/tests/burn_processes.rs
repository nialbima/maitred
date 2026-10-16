//! `burn`'s process behaviour: the children, daemons and signal handling that supervision scenarios need.

use std::collections::HashMap;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;
use std::{env, fs, thread};

use maitred_testkit::timing::millis;

fn burn(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_burn"));
    command
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn report_path(name: &str) -> PathBuf {
    env::temp_dir().join(format!("burn-proc-{}-{name}.report", std::process::id()))
}

fn read_report(path: &Path) -> HashMap<String, String> {
    fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("no report at {}", path.display()))
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// Waits until `path` exists, for up to `limit_ms`.
fn wait_for(path: &Path, limit_ms: u64) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < millis(limit_ms),
            "{} never appeared",
            path.display()
        );
        thread::sleep(millis(10));
    }
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn signal(child: &Child, name: &str) {
    let status = Command::new("kill")
        .args([&format!("-{name}"), &child.id().to_string()])
        .status();
    assert!(status.is_ok_and(|s| s.success()), "couldn't send {name}");
}

#[test]
fn it_waits_for_its_short_lived_children_and_reports_them() {
    let path = report_path("children");
    let status = burn(&[
        "--run-ms",
        "100",
        "--children",
        "3",
        "--child-run-ms",
        "30",
        "--report",
        path.to_str().unwrap(),
    ])
    .status()
    .expect("burn didn't start");

    let report = read_report(&path);
    let children: Vec<&str> = report
        .get("children")
        .expect("no children in the report")
        .split(',')
        .collect();
    assert!(status.success());
    assert_eq!(children.len(), 3);
    assert!(
        children.iter().all(|pid| !alive(pid)),
        "a child outlived burn: {children:?}"
    );
}

#[test]
fn a_child_whose_parent_exits_first_keeps_running_after_it() {
    let path = report_path("orphan");
    burn(&[
        "--run-ms",
        "20",
        "--orphan-child-run-ms",
        "500",
        "--report",
        path.to_str().unwrap(),
    ])
    .status()
    .expect("burn didn't start");

    let orphan = read_report(&path)
        .get("orphan")
        .cloned()
        .expect("no orphan in the report");
    assert!(alive(&orphan), "the orphan died with its parent");
}

#[test]
fn a_daemon_that_calls_setsid_leads_a_session_of_its_own() {
    let path = report_path("setsid");
    let daemon_path = report_path("setsid-daemon");
    burn(&[
        "--run-ms",
        "20",
        "--daemon",
        "setsid",
        "--daemon-run-ms",
        "50",
        "--daemon-report",
        daemon_path.to_str().unwrap(),
        "--report",
        path.to_str().unwrap(),
    ])
    .status()
    .expect("burn didn't start");
    wait_for(&daemon_path, 2_000);

    let parent = read_report(&path);
    let daemon = read_report(&daemon_path);
    assert_eq!(
        daemon.get("sid"),
        daemon.get("pid"),
        "the daemon doesn't lead its session"
    );
    assert_ne!(daemon.get("sid"), parent.get("sid"));
}

#[test]
fn a_daemon_that_calls_setpgid_leaves_the_group_but_not_the_session() {
    let path = report_path("setpgid");
    let daemon_path = report_path("setpgid-daemon");
    burn(&[
        "--run-ms",
        "20",
        "--daemon",
        "setpgid",
        "--daemon-run-ms",
        "50",
        "--daemon-report",
        daemon_path.to_str().unwrap(),
        "--report",
        path.to_str().unwrap(),
    ])
    .status()
    .expect("burn didn't start");
    wait_for(&daemon_path, 2_000);

    let parent = read_report(&path);
    let daemon = read_report(&daemon_path);
    assert_eq!(
        daemon.get("pgid"),
        daemon.get("pid"),
        "the daemon doesn't lead its group"
    );
    assert_eq!(
        daemon.get("sid"),
        parent.get("sid"),
        "the daemon left the session too"
    );
}

#[test]
fn with_term_ignored_it_runs_on_to_its_own_end() {
    let path = report_path("term-ignore");
    let child = burn(&[
        "--run-ms",
        "400",
        "--on-term",
        "ignore",
        "--exit",
        "3",
        "--report",
        path.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    thread::sleep(millis(100));
    signal(&child, "TERM");
    let output = child.wait_with_output().expect("burn didn't finish");

    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn with_term_handled_it_exits_cleanly_at_once_and_says_so() {
    let path = report_path("term-exit");
    let started = Instant::now();
    let child = burn(&[
        "--run-ms",
        "5000",
        "--on-term",
        "exit",
        "--report",
        path.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    thread::sleep(millis(100));
    signal(&child, "TERM");
    let output = child.wait_with_output().expect("burn didn't finish");

    assert_eq!(output.status.code(), Some(0));
    assert!(started.elapsed() < millis(2_000), "it ran on after TERM");
    assert_eq!(read_report(&path).get("signals"), Some(&"TERM".to_string()));
}

#[test]
fn with_int_handled_it_records_the_signal_the_same_way() {
    let path = report_path("int-exit");
    let child = burn(&[
        "--run-ms",
        "5000",
        "--on-int",
        "exit",
        "--report",
        path.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    thread::sleep(millis(100));
    signal(&child, "INT");
    let output = child.wait_with_output().expect("burn didn't finish");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(read_report(&path).get("signals"), Some(&"INT".to_string()));
}

#[test]
fn by_default_term_kills_it() {
    let child = burn(&["--run-ms", "5000"])
        .spawn()
        .expect("burn didn't start");
    thread::sleep(millis(100));
    signal(&child, "TERM");
    let output = child.wait_with_output().expect("burn didn't finish");

    assert_eq!(output.status.signal(), Some(15));
}
