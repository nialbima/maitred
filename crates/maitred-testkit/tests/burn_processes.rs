//! `burn`'s process behaviour: the children, daemons and signal handling that supervision scenarios need.

use std::collections::HashMap;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Instant;
use std::{env, fs, thread};

use maitred_testkit::timing::{OUTLIVE_MS, millis, wait_ceiling};

// Every wait here is for an event, never for a time: a file waited for is waited for while the process that writes it
// runs, a death is polled for, and anything that must not end by itself runs for `OUTLIVE_MS`. Only a failing test
// reaches `wait_ceiling` (MT-054).

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

/// Waits until `path` exists. Only `writer` writes it, so the wait lasts while `running` says it runs: it fails at
/// once if the writer ends without writing the file, and at the ceiling if it runs on and never does.
fn wait_for(path: &Path, writer: &str, mut running: impl FnMut() -> bool) {
    let started = Instant::now();
    while !path.exists() {
        if !running() {
            assert!(
                path.exists(),
                "{writer} ended without writing {}",
                path.display()
            );
            return;
        }
        assert!(
            started.elapsed() < wait_ceiling(),
            "{writer} ran for {:?} without writing {}",
            wait_ceiling(),
            path.display()
        );
        thread::sleep(millis(10));
    }
}

/// Waits until `pid` is gone, failing with `why` at the ceiling.
fn until_dead(pid: &str, why: &str) {
    let started = Instant::now();
    while alive(pid) {
        assert!(started.elapsed() < wait_ceiling(), "{why} (pid {pid})");
        thread::sleep(millis(10));
    }
}

/// The report `path` once `pid`, which writes it as it ends, has written it.
fn report_of(path: &Path, pid: &str) -> HashMap<String, String> {
    wait_for(path, &format!("process {pid}"), || alive(pid));
    read_report(path)
}

fn outlive() -> String {
    OUTLIVE_MS.to_string()
}

/// A burn this test started. Dropping it kills and collects it, so a test that fails part way leaves nothing
/// running for `OUTLIVE_MS`.
struct Burn(Child);

impl Burn {
    fn start(args: &[&str]) -> Self {
        Self(burn(args).spawn().expect("burn didn't start"))
    }

    fn pid(&self) -> String {
        self.0.id().to_string()
    }

    fn running(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }

    fn signal(&self, name: &str) {
        let status = Command::new("kill")
            .args([&format!("-{name}"), &self.pid()])
            .status();
        assert!(status.is_ok_and(|s| s.success()), "couldn't send {name}");
    }

    /// The pids burn listed in its ready file, once it has written it.
    fn ready(&mut self, path: &Path) -> HashMap<String, String> {
        let writer = format!("burn {}", self.pid());
        wait_for(path, &writer, || self.running());
        read_report(path)
    }

    /// Waits for burn to end by itself, failing at the ceiling.
    fn finish(&mut self) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.0.try_wait().expect("can't wait for burn") {
                return status;
            }
            assert!(
                started.elapsed() < wait_ceiling(),
                "burn {} ran on for {:?}",
                self.pid(),
                wait_ceiling()
            );
            thread::sleep(millis(10));
        }
    }
}

impl Drop for Burn {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A process this test knows only by pid (an orphan or a daemon), killed when the test ends if it still runs.
struct Stray(String);

impl Drop for Stray {
    fn drop(&mut self) {
        if alive(&self.0) {
            let _ = Command::new("kill").args(["-KILL", &self.0]).status();
        }
    }
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
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
        &outlive(),
        "--report",
        path.to_str().unwrap(),
    ])
    .status()
    .expect("burn didn't start");

    let orphan = Stray(
        read_report(&path)
            .get("orphan")
            .cloned()
            .expect("no orphan in the report"),
    );
    assert!(alive(&orphan.0), "the orphan died with its parent");
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

    let parent = read_report(&path);
    let daemon = report_of(&daemon_path, &parent["daemon"]);
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

    let parent = read_report(&path);
    let daemon = report_of(&daemon_path, &parent["daemon"]);
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
    let mut child = Burn::start(&[
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
    ]);
    child.ready(&ready);
    child.signal("TERM");

    assert_eq!(child.finish().code(), Some(3));
}

#[test]
fn with_term_handled_it_exits_cleanly_at_once_and_says_so() {
    let path = report_path("term-exit");
    let ready = report_path("term-exit-ready");
    let mut child = Burn::start(&[
        "--run-ms",
        &outlive(),
        "--on-term",
        "exit",
        "--report",
        path.to_str().unwrap(),
        "--ready",
        ready.to_str().unwrap(),
    ]);
    child.ready(&ready);
    child.signal("TERM");

    assert_eq!(child.finish().code(), Some(0));
    assert_eq!(read_report(&path).get("signals"), Some(&"TERM".to_string()));
}

#[test]
fn with_int_handled_it_records_the_signal_the_same_way() {
    let path = report_path("int-exit");
    let ready = report_path("int-exit-ready");
    let mut child = Burn::start(&[
        "--run-ms",
        &outlive(),
        "--on-int",
        "exit",
        "--report",
        path.to_str().unwrap(),
        "--ready",
        ready.to_str().unwrap(),
    ]);
    child.ready(&ready);
    child.signal("INT");

    assert_eq!(child.finish().code(), Some(0));
    assert_eq!(read_report(&path).get("signals"), Some(&"INT".to_string()));
}

#[test]
fn by_default_term_kills_it() {
    let ready = report_path("default-term-ready");
    let mut child = Burn::start(&["--run-ms", &outlive(), "--ready", ready.to_str().unwrap()]);
    child.ready(&ready);
    child.signal("TERM");

    assert_eq!(child.finish().signal(), Some(15));
}

#[test]
fn a_ready_file_appears_once_its_signal_handlers_are_in() {
    let ready = report_path("ready");
    let mut child = Burn::start(&[
        "--run-ms",
        &outlive(),
        "--on-term",
        "exit",
        "--ready",
        ready.to_str().unwrap(),
    ]);
    let pids = child.ready(&ready);
    child.signal("TERM");

    assert_eq!(pids.get("pid"), Some(&child.pid()));
    assert_eq!(child.finish().code(), Some(0));
}

#[test]
fn an_ignored_term_isnt_passed_on_to_the_children_it_starts() {
    let ready = report_path("ignore-inherit");
    let mut parent = Burn::start(&[
        "--run-ms",
        &outlive(),
        "--on-term",
        "ignore",
        "--orphan-child-run-ms",
        &outlive(),
        "--ready",
        ready.to_str().unwrap(),
    ]);
    let orphan = Stray(
        parent
            .ready(&ready)
            .get("orphan")
            .cloned()
            .expect("no orphan in the ready file"),
    );

    let killed = Command::new("kill")
        .args(["-TERM", &orphan.0])
        .status()
        .expect("kill didn't run");

    // burn collects a child as soon as it exits, so a child TERM killed is gone, not a zombie.
    assert!(killed.success());
    until_dead(&orphan.0, "the child kept burn's ignored TERM");
}

#[test]
fn a_child_that_finishes_mid_run_doesnt_linger_as_a_zombie() {
    let ready = report_path("reaped");
    let mut parent = Burn::start(&[
        "--run-ms",
        &outlive(),
        "--orphan-child-run-ms",
        "100",
        "--ready",
        ready.to_str().unwrap(),
    ]);
    let orphan = parent
        .ready(&ready)
        .get("orphan")
        .cloned()
        .expect("no orphan in the ready file");

    // `kill -0` succeeds on a zombie and fails on a process its parent has collected, so the child disappearing
    // while burn still runs means burn collected it.
    until_dead(
        &orphan,
        "the finished child was left a zombie while burn ran on",
    );
    assert!(
        parent.running(),
        "burn ended, so init may have collected it"
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
    let mut parent = Burn::start(&[
        "--run-ms",
        &outlive(),
        "--on-int",
        "ignore",
        "--orphan-child-run-ms",
        &outlive(),
        "--ready",
        ready.to_str().unwrap(),
    ]);
    let orphan = Stray(
        parent
            .ready(&ready)
            .get("orphan")
            .cloned()
            .expect("no orphan in the ready file"),
    );

    let killed = Command::new("kill")
        .args(["-INT", &orphan.0])
        .status()
        .expect("kill didn't run");

    assert!(killed.success());
    until_dead(&orphan.0, "the child kept burn's ignored INT");
}

#[test]
fn the_ready_file_comes_after_the_memory_is_touched_and_names_the_daemon() {
    let ready = report_path("ready-late");
    let daemon_report = report_path("ready-late-daemon");
    let mut parent = Burn::start(&[
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
    ]);
    let pids = parent.ready(&ready);
    parent.finish();

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

#[test]
fn a_handled_term_ends_the_memory_fill_where_it_is() {
    let held = report_path("held");
    let path = report_path("held-report");
    let mut child = Burn::start(&[
        "--run-ms",
        &outlive(),
        "--mem-mb",
        "256",
        "--hold-fill-at-mb",
        "64",
        "--held",
        held.to_str().unwrap(),
        "--on-term",
        "exit",
        "--report",
        path.to_str().unwrap(),
    ]);
    let writer = format!("burn {}", child.pid());
    wait_for(&held, &writer, || child.running());
    child.signal("TERM");

    assert_eq!(child.finish().code(), Some(0));
    assert_eq!(
        read_report(&path).get("touched_mb").map(String::as_str),
        Some("64"),
        "the fill went on after TERM"
    );
}

#[test]
fn a_reader_that_goes_away_kills_it_by_sigpipe_as_it_would_any_program() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_burn"))
        .args(["--run-ms", "3000", "--print-every-ms", "20"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("burn didn't start");
    drop(child.stdout.take());
    let status = child.wait().expect("burn didn't finish");

    assert_eq!(status.signal(), Some(13), "{status:?}");
}

#[test]
fn children_can_keep_its_ignored_term_as_a_nohup_child_would() {
    let ready = report_path("keep-signals");
    let orphan_report = report_path("keep-signals-orphan");
    let mut parent = Burn::start(&[
        "--run-ms",
        &outlive(),
        "--on-term",
        "ignore",
        "--children-keep-signals",
        "--orphan-child-run-ms",
        "0",
        "--orphan-report",
        orphan_report.to_str().unwrap(),
        "--ready",
        ready.to_str().unwrap(),
    ]);
    let orphan = parent
        .ready(&ready)
        .get("orphan")
        .cloned()
        .expect("no orphan in the ready file");

    // The child reports the signals it ignores, read back from sigaction, rather than surviving a TERM for a while.
    assert_eq!(
        report_of(&orphan_report, &orphan)
            .get("ignored")
            .map(String::as_str),
        Some("TERM"),
        "the child didn't keep the ignored TERM"
    );
}

#[test]
fn a_daemon_is_not_burns_child() {
    let path = report_path("daemon-parent");
    let daemon_path = report_path("daemon-parent-daemon");
    burn(&[
        "--run-ms",
        "20",
        "--daemon",
        "setsid",
        "--daemon-run-ms",
        "300",
        "--daemon-report",
        daemon_path.to_str().unwrap(),
        "--report",
        path.to_str().unwrap(),
    ])
    .status()
    .expect("burn didn't start");

    let parent = read_report(&path);
    let daemon = report_of(&daemon_path, &parent["daemon"]);
    let daemon_parent = daemon.get("ppid").expect("no ppid in the daemon's report");
    assert_ne!(
        Some(daemon_parent),
        parent.get("pid"),
        "the daemon is burn's own child"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn on_macos_the_report_gives_a_physical_footprint_that_covers_the_memory() {
    let path = report_path("footprint");
    burn(&[
        "--run-ms",
        "50",
        "--mem-mb",
        "64",
        "--report",
        path.to_str().unwrap(),
    ])
    .status()
    .expect("burn didn't start");

    let footprint: u64 = read_report(&path)
        .get("peak_footprint_mb")
        .expect("no peak_footprint_mb in the report")
        .parse()
        .expect("not a number");
    assert!(footprint >= 64, "footprint {footprint} MB, asked for 64");
}
