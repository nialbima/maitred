//! `burn`: the job in every real-process scenario (ARCHITECTURE.md, Test kit: Workloads).
//!
//! It uses exactly the CPU and memory it is told to, for as long as it is told to, then writes a report of
//! what it really did, so a scenario can compare a holder's measurement with the truth.
//!
//! ```text
//! burn [--label L] [--run-ms T] [--exit S] [--cpu-millicores C] [--mem-mb M]
//!      [--print-every-ms P] [--port] [--report PATH]
//!      [--children N --child-run-ms T] [--orphan-child-run-ms T]
//!      [--daemon setsid|setpgid --daemon-run-ms T [--daemon-report PATH]]
//!      [--on-term ignore|exit] [--on-int ignore|exit] [--ready PATH]
//! ```
//!
//! Children, orphans and daemons are `burn` again, started with no load of their own and with TERM and INT at
//! their default action, whatever this `burn` does with them. A signal it is told to `exit` on ends the run
//! early with status 0 and is named in the report; one it is told to `ignore` changes nothing; any other keeps
//! its default action. With `--exit` on a signal, it still waits for the children it started (not the orphan or
//! the daemon) before it exits.
//!
//! `--ready PATH` is written once the signal handlers are in and every child has started. It lists `pid`,
//! `orphan` and `children` in the report's format, so a test or the runner signals `burn` only once it is ready.
//!
//! Exits 64 on an argument it doesn't know or `--print-every-ms 0`, and 71 if `setsid` or `setpgid` fails.

use std::fmt::Write as _;
use std::hint::black_box;
use std::net::TcpListener;
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, ExitCode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::Instant;
use std::{env, fs};

use maitred_testkit::timing::{burn_duty_period, millis};

const USAGE: u8 = 64;
const OS_ERROR: u8 = 71;

/// Set by a signal handler when a signal burn was told to exit on arrives.
static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);
/// The signals that arrived, as bits: 1 for TERM, 2 for INT.
static RECEIVED: AtomicU8 = AtomicU8::new(0);
const GOT_TERM: u8 = 1;
const GOT_INT: u8 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OnSignal {
    #[default]
    Default,
    Ignore,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Detach {
    Session,
    Group,
}

#[derive(Debug, Default)]
struct Args {
    label: String,
    run_ms: u64,
    exit: u8,
    cpu_millicores: u64,
    mem_mb: u64,
    print_every_ms: Option<u64>,
    port: bool,
    report: Option<String>,
    children: u32,
    child_run_ms: u64,
    orphan_child_run_ms: Option<u64>,
    daemon: Option<Detach>,
    daemon_run_ms: u64,
    daemon_report: Option<String>,
    on_term: OnSignal,
    on_int: OnSignal,
    /// Set on a daemon started by another burn: what it leaves before doing anything else.
    detach: Option<Detach>,
    ready: Option<String>,
}

fn parse(mut raw: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        label: "burn".into(),
        ..Args::default()
    };
    while let Some(flag) = raw.next() {
        let mut value = || raw.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--label" => args.label = value()?,
            "--run-ms" => args.run_ms = number(&value()?)?,
            "--exit" => args.exit = number(&value()?)?,
            "--cpu-millicores" => args.cpu_millicores = number(&value()?)?,
            "--mem-mb" => args.mem_mb = number(&value()?)?,
            "--print-every-ms" => {
                let every: u64 = number(&value()?)?;
                if every == 0 {
                    return Err("--print-every-ms must be more than 0".into());
                }
                args.print_every_ms = Some(every);
            }
            "--ready" => args.ready = Some(value()?),
            "--port" => args.port = true,
            "--report" => args.report = Some(value()?),
            "--children" => args.children = number(&value()?)?,
            "--child-run-ms" => args.child_run_ms = number(&value()?)?,
            "--orphan-child-run-ms" => args.orphan_child_run_ms = Some(number(&value()?)?),
            "--daemon" => args.daemon = Some(detach(&value()?)?),
            "--daemon-run-ms" => args.daemon_run_ms = number(&value()?)?,
            "--daemon-report" => args.daemon_report = Some(value()?),
            "--on-term" => args.on_term = on_signal(&value()?)?,
            "--on-int" => args.on_int = on_signal(&value()?)?,
            "--detach" => args.detach = Some(detach(&value()?)?),
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    Ok(args)
}

fn detach(text: &str) -> Result<Detach, String> {
    match text {
        "setsid" => Ok(Detach::Session),
        "setpgid" => Ok(Detach::Group),
        _ => Err(format!("{text} isn't setsid or setpgid")),
    }
}

fn on_signal(text: &str) -> Result<OnSignal, String> {
    match text {
        "ignore" => Ok(OnSignal::Ignore),
        "exit" => Ok(OnSignal::Exit),
        _ => Err(format!("{text} isn't ignore or exit")),
    }
}

extern "C" fn record_and_exit(signal: libc::c_int) {
    let bit = if signal == libc::SIGTERM {
        GOT_TERM
    } else {
        GOT_INT
    };
    RECEIVED.fetch_or(bit, Ordering::SeqCst);
    EXIT_REQUESTED.store(true, Ordering::SeqCst);
}

/// Sets how burn treats `signal`. Only atomics are touched in the handler, which is async-signal-safe.
fn handle(signal: libc::c_int, how: OnSignal) {
    let action = match how {
        OnSignal::Default => return,
        OnSignal::Ignore => libc::SIG_IGN,
        OnSignal::Exit => record_and_exit as extern "C" fn(libc::c_int) as libc::sighandler_t,
    };
    // SAFETY: the handler only stores to static atomics, and SIG_IGN needs no handler at all.
    unsafe {
        libc::signal(signal, action);
    }
}

/// Leaves the session or the process group, as a daemon does. Returns false if the call failed.
fn leave(how: Detach) -> bool {
    // SAFETY: setsid and setpgid act only on the calling process; burn has started no threads yet.
    let result = unsafe {
        match how {
            Detach::Session => libc::setsid(),
            Detach::Group => libc::setpgid(0, 0),
        }
    };
    result != -1
}

/// Starts burn again with no load of its own, running for `run_ms`, plus `extra` arguments.
fn start_burn(run_ms: u64, extra: &[&str]) -> Child {
    let exe = env::current_exe().expect("burn: can't find its own binary");
    let mut command = Command::new(exe);
    command.args(["--run-ms", &run_ms.to_string()]).args(extra);
    // SAFETY: between fork and exec only `signal` runs, which is async-signal-safe. It puts TERM and INT back to
    // their default action, which an ignored disposition would otherwise carry through exec into the child.
    unsafe {
        command.pre_exec(|| {
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            Ok(())
        });
    }
    command.spawn().expect("burn: can't start a child")
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("{text} isn't a number"))
}

/// Keeps `millicores` of CPU busy until `stop` is set: one thread per whole core, and one more that spins
/// for its share of each duty period for the remainder.
fn spin(millicores: u64, stop: &Arc<AtomicBool>) -> Vec<thread::JoinHandle<()>> {
    let mut shares = vec![1_000; usize::try_from(millicores / 1_000).unwrap_or(usize::MAX)];
    if !millicores.is_multiple_of(1_000) {
        shares.push(millicores % 1_000);
    }
    shares
        .into_iter()
        .map(|share| {
            let stop = Arc::clone(stop);
            thread::spawn(move || {
                let period = burn_duty_period();
                let busy = period * u32::try_from(share).unwrap_or(1_000) / 1_000;
                let mut counter = 0_u64;
                while !stop.load(Ordering::Relaxed) {
                    let started = Instant::now();
                    while started.elapsed() < busy {
                        counter = black_box(counter.wrapping_add(1));
                    }
                    if let Some(rest) = period.checked_sub(started.elapsed()) {
                        thread::sleep(rest);
                    }
                }
            })
        })
        .collect()
}

/// Allocates `mb` megabytes and writes to every page, so all of it is resident.
fn touch(mb: u64) -> Vec<u8> {
    let mut memory = vec![0_u8; usize::try_from(mb * 1024 * 1024).unwrap_or(usize::MAX)];
    for page in memory.chunks_mut(4096) {
        page[0] = 1;
    }
    black_box(memory)
}

/// This process's CPU time so far, and its peak resident size, from `getrusage`.
fn usage() -> (u64, u64) {
    // SAFETY: getrusage only writes into the struct it is given, which is zeroed and lives on this stack.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &raw mut usage);
        usage
    };
    let ms = |t: libc::timeval| {
        u64::try_from(t.tv_sec).unwrap_or(0) * 1_000 + u64::try_from(t.tv_usec).unwrap_or(0) / 1_000
    };
    let max_rss = u64::try_from(usage.ru_maxrss).unwrap_or(0);
    // macOS reports ru_maxrss in bytes, Linux in kilobytes.
    let resident_mb = if cfg!(target_os = "macos") {
        max_rss / (1024 * 1024)
    } else {
        max_rss / 1024
    };
    (ms(usage.ru_utime) + ms(usage.ru_stime), resident_mb)
}

fn process_ids() -> (u32, i32, i32) {
    // SAFETY: getpgid and getsid with 0 only read the calling process's own ids.
    let (pgid, sid) = unsafe { (libc::getpgid(0), libc::getsid(0)) };
    (std::process::id(), pgid, sid)
}

fn signal_names() -> String {
    let received = RECEIVED.load(Ordering::SeqCst);
    let names: Vec<&str> = [(GOT_TERM, "TERM"), (GOT_INT, "INT")]
        .into_iter()
        .filter(|(bit, _)| received & bit != 0)
        .map(|(_, name)| name)
        .collect();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(",")
    }
}

fn report(args: &Args, port: Option<u16>, children: &[u32], orphan: Option<u32>) -> String {
    let (process, group, session) = process_ids();
    let (cpu_ms, resident_mb) = usage();
    let slot = env::var("TEST_ENV_NUMBER").unwrap_or_else(|_| "none".into());
    let mut text = String::new();
    let _ = writeln!(text, "label {}", args.label);
    let _ = writeln!(text, "pid {process}");
    let _ = writeln!(text, "pgid {group}");
    let _ = writeln!(text, "sid {session}");
    let _ = writeln!(text, "slot {slot}");
    let _ = writeln!(
        text,
        "port {}",
        port.map_or_else(|| "none".into(), |p| p.to_string())
    );
    let _ = writeln!(text, "cpu_ms {cpu_ms}");
    let _ = writeln!(text, "peak_resident_mb {resident_mb}");
    let ids: Vec<String> = children.iter().map(ToString::to_string).collect();
    let _ = writeln!(
        text,
        "children {}",
        if ids.is_empty() {
            "none".into()
        } else {
            ids.join(",")
        }
    );
    let _ = writeln!(
        text,
        "orphan {}",
        orphan.map_or_else(|| "none".into(), |pid| pid.to_string())
    );
    let _ = writeln!(text, "signals {}", signal_names());
    text
}

/// Starts the children, the orphan and the daemon `args` ask for.
fn start_children(args: &Args) -> (Vec<Child>, Option<Child>, Option<Child>) {
    let children = (0..args.children)
        .map(|_| start_burn(args.child_run_ms, &[]))
        .collect();
    let orphan = args
        .orphan_child_run_ms
        .map(|run_ms| start_burn(run_ms, &[]));
    let daemon = args.daemon.map(|how| {
        let how = if how == Detach::Session {
            "setsid"
        } else {
            "setpgid"
        };
        let mut extra = vec!["--detach", how];
        if let Some(path) = &args.daemon_report {
            extra.extend(["--report", path]);
        }
        start_burn(args.daemon_run_ms, &extra)
    });
    (children, orphan, daemon)
}

/// Writes the ready file: burn's pid and its children's, in the report's format.
fn write_ready(path: &str, children: &[Child], orphan: Option<&Child>) {
    let (process, _, _) = process_ids();
    let ids: Vec<String> = children.iter().map(|c| c.id().to_string()).collect();
    let text = format!(
        "pid {process}\norphan {}\nchildren {}\n",
        orphan.map_or_else(|| "none".into(), |o| o.id().to_string()),
        if ids.is_empty() {
            "none".into()
        } else {
            ids.join(",")
        }
    );
    fs::write(path, text).expect("burn: can't write the ready file");
}

#[expect(
    clippy::print_stderr,
    reason = "a command-line program reports a usage error on stderr"
)]
fn main() -> ExitCode {
    let args = match parse(env::args().skip(1)) {
        Ok(args) => args,
        Err(problem) => {
            eprintln!("burn: {problem}");
            return ExitCode::from(USAGE);
        }
    };
    if let Some(how) = args.detach
        && !leave(how)
    {
        eprintln!(
            "burn: can't leave the {}",
            if how == Detach::Session {
                "session"
            } else {
                "process group"
            }
        );
        return ExitCode::from(OS_ERROR);
    }
    handle(libc::SIGTERM, args.on_term);
    handle(libc::SIGINT, args.on_int);
    let started = Instant::now();
    let (children, mut orphan, mut daemon) = start_children(&args);
    if let Some(path) = &args.ready {
        write_ready(path, &children, orphan.as_ref());
    }
    let mut reap = || {
        // The orphan and the daemon are meant to outlive burn, so burn never waits for them, but it collects
        // either one as soon as it has exited, so no zombie of burn's lingers in the session.
        for left_behind in [orphan.as_mut(), daemon.as_mut()].into_iter().flatten() {
            let _ = left_behind.try_wait();
        }
    };
    let listener = args
        .port
        .then(|| TcpListener::bind("127.0.0.1:0").expect("burn: can't bind a port"));
    let port = listener
        .as_ref()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port());
    let memory = touch(args.mem_mb);
    let stop = Arc::new(AtomicBool::new(false));
    let spinners = spin(args.cpu_millicores, &stop);

    let end = millis(args.run_ms);
    let mut tick = 0_u64;
    loop {
        let next = args.print_every_ms.map(|every| millis(every * (tick + 1)));
        match next {
            Some(at) if at < end => {
                if !sleep_until(started, at, &mut reap) {
                    break;
                }
                tick += 1;
                println!("{} tick {tick}", args.label_line());
            }
            _ => {
                sleep_until(started, end, &mut reap);
                break;
            }
        }
    }

    stop.store(true, Ordering::Relaxed);
    for spinner in spinners {
        let _ = spinner.join();
    }
    let child_ids: Vec<u32> = children.iter().map(Child::id).collect();
    for mut child in children {
        let _ = child.wait();
    }
    reap();
    let orphan = orphan.as_ref().map(Child::id);
    if let Some(path) = &args.report {
        fs::write(path, report(&args, port, &child_ids, orphan))
            .expect("burn: can't write the report");
    }
    drop(black_box(memory));
    if EXIT_REQUESTED.load(Ordering::SeqCst) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(args.exit)
    }
}

/// Sleeps until `at` after `started`, calling `reap` every duty period and waking early if a signal asked burn
/// to exit. Returns false if it did.
fn sleep_until(started: Instant, at: std::time::Duration, reap: &mut impl FnMut()) -> bool {
    while let Some(left) = at.checked_sub(started.elapsed()) {
        if EXIT_REQUESTED.load(Ordering::SeqCst) {
            return false;
        }
        reap();
        thread::sleep(left.min(burn_duty_period()));
    }
    !EXIT_REQUESTED.load(Ordering::SeqCst)
}

impl Args {
    fn label_line(&self) -> String {
        format!("burn {}", self.label)
    }
}
