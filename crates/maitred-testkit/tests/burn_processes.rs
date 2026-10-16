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
    let ready = report_path("term-ignore-ready");
    let child = burn(&[
        "--run-ms",
        "400",
        "--on-term",
        "ignore",
        "--exit",
        "3",
        "--report",
        path.to_str().unwrap(),
        "--ready",
        ready.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    ready_pids(&ready);
    signal(&child, "TERM");
    let output = child.wait_with_output().expect("burn didn't finish");

    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn with_term_handled_it_exits_cleanly_at_once_and_says_so() {
    let path = report_path("term-exit");
    let ready = report_path("term-exit-ready");
    let started = Instant::now();
    let child = burn(&[
        "--run-ms",
        "5000",
        "--on-term",
        "exit",
        "--report",
        path.to_str().unwrap(),
        "--ready",
        ready.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    ready_pids(&ready);
    signal(&child, "TERM");
    let output = child.wait_with_output().expect("burn didn't finish");

    assert_eq!(output.status.code(), Some(0));
    assert!(started.elapsed() < millis(2_000), "it ran on after TERM");
    assert_eq!(read_report(&path).get("signals"), Some(&"TERM".to_string()));
}

#[test]
fn with_int_handled_it_records_the_signal_the_same_way() {
    let path = report_path("int-exit");
    let ready = report_path("int-exit-ready");
    let child = burn(&[
        "--run-ms",
        "5000",
        "--on-int",
        "exit",
        "--report",
        path.to_str().unwrap(),
        "--ready",
        ready.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    ready_pids(&ready);
    signal(&child, "INT");
    let output = child.wait_with_output().expect("burn didn't finish");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(read_report(&path).get("signals"), Some(&"INT".to_string()));
}

#[test]
fn by_default_term_kills_it() {
    let ready = report_path("default-term-ready");
    let child = burn(&["--run-ms", "5000", "--ready", ready.to_str().unwrap()])
        .spawn()
        .expect("burn didn't start");
    ready_pids(&ready);
    signal(&child, "TERM");
    let output = child.wait_with_output().expect("burn didn't finish");

    assert_eq!(output.status.signal(), Some(15));
}

/// The pids burn listed in its ready file, once it has written it.
fn ready_pids(path: &Path) -> HashMap<String, String> {
    wait_for(path, 2_000);
    read_report(path)
}

#[test]
fn a_ready_file_appears_once_its_signal_handlers_are_in() {
    let ready = report_path("ready");
    let mut child = burn(&[
        "--run-ms",
        "5000",
        "--on-term",
        "exit",
        "--ready",
        ready.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    let pids = ready_pids(&ready);
    signal(&child, "TERM");
    let status = child.wait().expect("burn didn't finish");

    assert_eq!(pids.get("pid"), Some(&child.id().to_string()));
    assert_eq!(status.code(), Some(0));
}

#[test]
fn an_ignored_term_isnt_passed_on_to_the_children_it_starts() {
    let ready = report_path("ignore-inherit");
    let mut parent = burn(&[
        "--run-ms",
        "3000",
        "--on-term",
        "ignore",
        "--orphan-child-run-ms",
        "5000",
        "--ready",
        ready.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    let orphan = ready_pids(&ready)
        .get("orphan")
        .cloned()
        .expect("no orphan in the ready file");

    let killed = Command::new("kill")
        .args(["-TERM", &orphan])
        .status()
        .expect("kill didn't run");
    thread::sleep(millis(300));

    // burn collects a child as soon as it exits, so a child TERM killed is gone, not a zombie.
    assert!(killed.success());
    assert!(!alive(&orphan), "the child kept burn's ignored TERM");
    let _ = parent.kill();
    let _ = parent.wait();
}

#[test]
fn a_child_that_finishes_mid_run_doesnt_linger_as_a_zombie() {
    let ready = report_path("reaped");
    let mut parent = burn(&[
        "--run-ms",
        "1500",
        "--orphan-child-run-ms",
        "100",
        "--ready",
        ready.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    let orphan = ready_pids(&ready)
        .get("orphan")
        .cloned()
        .expect("no orphan in the ready file");

    // The child has been gone for 500 ms while burn still runs. `kill -0` succeeds on a zombie and fails on a
    // process its parent has collected, so finding nothing means burn collected it.
    thread::sleep(millis(600));
    let lingering = alive(&orphan);
    let _ = parent.wait();

    assert!(
        !lingering,
        "the finished child was left a zombie while burn ran on"
    );
}

#[test]
fn printing_every_zero_milliseconds_is_refused() {
    let status = burn(&["--run-ms", "100", "--print-every-ms", "0"])
        .status()
        .expect("burn didn't start");

    assert_eq!(status.code(), Some(64));
}

#[test]
fn an_ignored_int_isnt_passed_on_to_the_children_it_starts() {
    let ready = report_path("ignore-int-inherit");
    let mut parent = burn(&[
        "--run-ms",
        "3000",
        "--on-int",
        "ignore",
        "--orphan-child-run-ms",
        "5000",
        "--ready",
        ready.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    let orphan = ready_pids(&ready)
        .get("orphan")
        .cloned()
        .expect("no orphan in the ready file");

    let killed = Command::new("kill")
        .args(["-INT", &orphan])
        .status()
        .expect("kill didn't run");
    thread::sleep(millis(300));

    assert!(killed.success());
    assert!(!alive(&orphan), "the child kept burn's ignored INT");
    let _ = parent.kill();
    let _ = parent.wait();
}

#[test]
fn the_ready_file_comes_after_the_memory_is_touched_and_names_the_daemon() {
    let ready = report_path("ready-late");
    let daemon_report = report_path("ready-late-daemon");
    let mut parent = burn(&[
        "--run-ms",
        "1000",
        "--mem-mb",
        "64",
        "--daemon",
        "setsid",
        "--daemon-run-ms",
        "200",
        "--daemon-report",
        daemon_report.to_str().unwrap(),
        "--ready",
        ready.to_str().unwrap(),
    ])
    .spawn()
    .expect("burn didn't start");
    let pids = ready_pids(&ready);
    let _ = parent.wait();

    assert!(
        pids.get("daemon").is_some_and(|pid| pid != "none"),
        "no daemon in {pids:?}"
    );
    assert!(
        pids.get("resident_mb")
            .and_then(|mb| mb.parse::<u64>().ok())
            .is_some_and(|mb| mb >= 64),
        "the ready file came before the memory was touched: {pids:?}"
    );
}
