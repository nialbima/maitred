//! `burn`: the job in every real-process scenario (ARCHITECTURE.md, Test kit: Workloads).
//!
//! It uses exactly the CPU and memory it is told to, for as long as it is told to, then writes a report of
//! what it really did, so a scenario can compare a holder's measurement with the truth.
//!
//! ```text
//! burn [--label L] [--run-ms T] [--exit S] [--cpu-millicores C] [--mem-mb M]
//!      [--print-every-ms P] [--port] [--report PATH]
//! ```
//!
//! Exits 64 on an argument it doesn't know.

use std::fmt::Write as _;
use std::hint::black_box;
use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;
use std::{env, fs};

use maitred_testkit::timing::{burn_duty_period, millis};

const USAGE: u8 = 64;

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
            "--print-every-ms" => args.print_every_ms = Some(number(&value()?)?),
            "--port" => args.port = true,
            "--report" => args.report = Some(value()?),
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    Ok(args)
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

fn report(args: &Args, port: Option<u16>) -> String {
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
    text
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
    let started = Instant::now();
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
                if let Some(wait) = at.checked_sub(started.elapsed()) {
                    thread::sleep(wait);
                }
                tick += 1;
                println!("{} tick {tick}", args.label_line());
            }
            _ => {
                if let Some(wait) = end.checked_sub(started.elapsed()) {
                    thread::sleep(wait);
                }
                break;
            }
        }
    }

    stop.store(true, Ordering::Relaxed);
    for spinner in spinners {
        let _ = spinner.join();
    }
    if let Some(path) = &args.report {
        fs::write(path, report(&args, port)).expect("burn: can't write the report");
    }
    drop(black_box(memory));
    ExitCode::from(args.exit)
}

impl Args {
    fn label_line(&self) -> String {
        format!("burn {}", self.label)
    }
}
