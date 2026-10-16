//! The test kit's own writer of `load/now.json` (ARCHITECTURE.md: `load/now.json`). `replay` writes what the
//! scenario's [`Publisher`](maitred_scenario::Publisher) gives it through this, so every maitred reader of the
//! load file is tested against a writer maitred didn't produce.
//!
//! Hand-written rather than through a JSON library: the format is small, and writing it field by field in the
//! doc's order keeps the output easy to compare with the doc's example.

use std::fmt::Write as _;

use maitred_scenario::{Ms, Published, Reading};

/// Which platform's fields the file carries ("Absent or null" in the doc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    /// Writes `pressure`; no `maitred_running_threads` or `sleep_assertions`.
    Linux,
    /// Writes `maitred_running_threads` and `sleep_assertions`; no `pressure`.
    MacOs,
}

/// Where scenario time 0 falls on each of the two clocks the file uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clocks {
    /// Milliseconds since the Unix epoch, for the RFC 3339 fields.
    pub wall_ms_at_start: u64,
    /// Milliseconds on the boot clock, for the `_ms` fields.
    pub boot_ms_at_start: u64,
}

/// What the file needs beyond the published sample.
#[derive(Clone, Copy, Debug)]
pub struct Written<'a> {
    pub platform: Platform,
    pub clocks: Clocks,
    /// The ticket header's `boot` value; written as 16 lowercase hex digits.
    pub boot: u64,
    /// Each running job's ticket id, by label, as `replay` found them in `tickets/`.
    pub tickets: &'a [(&'a str, &'a str)],
    pub seq_high: u64,
}

/// The reading's field name, as `missing` lists it.
fn field_name(reading: Reading) -> &'static str {
    match reading {
        Reading::Cpu => "cpu_used_millicores",
        Reading::CpuPressure => "cpu_pressure",
        Reading::Memory => "mem_available_mb",
        Reading::Gpu => "gpu",
    }
}

/// Milliseconds since the Unix epoch as RFC 3339 in UTC, with milliseconds and a `Z`.
#[must_use]
pub fn rfc3339(epoch_ms: u64) -> String {
    let days = epoch_ms / 86_400_000;
    let in_day = epoch_ms % 86_400_000;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1_000 % 60,
        in_day % 1_000
    )
}

/// The calendar date `days` after 1970-01-01, by Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

/// Thousandths as a decimal with up to three places, trailing zeros dropped but one kept: 231 is `0.231`, 120 is
/// `0.12`, 0 is `0.0`.
fn thousandths(permille: u32) -> String {
    let text = format!("{}.{:03}", permille / 1_000, permille % 1_000);
    let trimmed = text.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{trimmed}0")
    } else {
        trimmed.to_string()
    }
}

/// A reading, or `null` when the sample names it missing.
fn reading(published: &Published, which: Reading, value: String) -> String {
    if published.missing.contains(&which) {
        "null".into()
    } else {
        value
    }
}

/// The load file for one published sample, in the doc's field order.
#[must_use]
pub fn now_json(published: &Published, written: &Written<'_>) -> String {
    let mut json = String::from("{\"v\":\"4.0\",");
    times(&mut json, published, written);
    readings(&mut json, published, written.platform);
    jobs(&mut json, published, written);
    if written.platform == Platform::MacOs {
        // Sleep assertions aren't modelled in scenarios; written as none.
        json.push_str("\"sleep_assertions\":{\"count\":0,\"unexpected\":0},");
    }
    let missing: Vec<String> = published
        .missing
        .iter()
        .map(|r| format!("\"{}\"", field_name(*r)))
        .collect();
    let _ = write!(json, "\"missing\":[{}]}}", missing.join(","));
    json
}

/// `sampled_at` through `woke_at_ms`: each moment on the wall clock for people and the boot clock for decisions.
fn times(json: &mut String, published: &Published, written: &Written<'_>) {
    let wall = |at: Ms| rfc3339(written.clocks.wall_ms_at_start + at.0);
    let boot_clock = |at: Ms| written.clocks.boot_ms_at_start + at.0;
    let _ = write!(
        json,
        "\"sampled_at\":\"{}\",\"sampled_at_ms\":{},",
        wall(published.sampled_at),
        boot_clock(published.sampled_at)
    );
    let _ = write!(json, "\"boot\":\"{:016x}\",", written.boot);
    let _ = write!(
        json,
        "\"interval_ms\":{},\"fresh\":{},",
        published.interval.0, published.fresh
    );
    match published.woke_at {
        Some(at) => {
            let _ = write!(
                json,
                "\"woke_at\":\"{}\",\"woke_at_ms\":{},",
                wall(at),
                boot_clock(at)
            );
        }
        None => json.push_str("\"woke_at\":null,\"woke_at_ms\":null,"),
    }
}

/// `capacity` through `pressure`: the machine's figures, null where the sample names a reading missing, and absent
/// where the platform has none.
fn readings(json: &mut String, published: &Published, platform: Platform) {
    let _ = write!(
        json,
        "\"capacity\":{{\"cpu_millicores\":{},\"mem_mb\":{}}},",
        published.capacity.cpu_millicores, published.capacity.mem_mb
    );
    let _ = write!(
        json,
        "\"cpu_used_millicores\":{},\"cpu_pressure\":{},",
        reading(
            published,
            Reading::Cpu,
            published.cpu_used_millicores.to_string()
        ),
        reading(
            published,
            Reading::CpuPressure,
            thousandths(published.cpu_pressure_permille)
        )
    );
    if platform == Platform::MacOs {
        let _ = write!(
            json,
            "\"maitred_running_threads\":{},",
            published.maitred_running_threads
        );
    }
    let _ = write!(
        json,
        "\"mem_available_mb\":{},",
        reading(
            published,
            Reading::Memory,
            published.mem_available_mb.to_string()
        )
    );
    if published.missing.contains(&Reading::Gpu) {
        json.push_str("\"gpu\":null,");
    } else if let Some(gpu) = published.gpu_permille {
        // GPU memory isn't modelled in scenarios; the shared pool on Apple Silicon is in the memory figures.
        let _ = write!(
            json,
            "\"gpu\":{{\"utilization\":{},\"mem_used_mb\":0}},",
            thousandths(gpu)
        );
    }
    let _ = write!(
        json,
        "\"quiet_streak\":{},\"over_cpu_streak\":{},\"over_gpu_streak\":{},",
        published.quiet_streak, published.over_cpu_streak, published.over_gpu_streak
    );
    if platform == Platform::Linux {
        // Decisions read cpu_pressure, not these averages, so scenarios don't model them.
        json.push_str(
            "\"pressure\":{\"cpu_some_avg10\":0.0,\"mem_some_avg10\":0.0,\"level\":\"normal\"},",
        );
    }
}

/// `jobs` and `seq_high`: each running job's use under its ticket id, for the jobs `replay` found a ticket for.
fn jobs(json: &mut String, published: &Published, written: &Written<'_>) {
    let entries: Vec<String> = published
        .jobs
        .iter()
        .filter_map(|(label, used)| {
            let id = written.tickets.iter().find(|(l, _)| l == label)?.1;
            Some(format!(
                "\"{id}\":{{\"cpu_millicores\":{},\"mem_mb\":{}}}",
                used.cpu_millicores, used.mem_mb
            ))
        })
        .collect();
    let _ = write!(
        json,
        "\"jobs\":{{{}}},\"seq_high\":{},",
        entries.join(","),
        written.seq_high
    );
}
