//! The test kit's own writer of `load/now.json` (ARCHITECTURE.md: `load/now.json`), a second writer every one of
//! maitred's readers is tested against.

use maitred_scenario::{Amount, MachineConfig, Publisher, Reading, Running, Sample, ms};
use maitred_testkit::load_file::{Clocks, Platform, Written, now_json, rfc3339};

fn inputs(platform: Platform) -> Written<'static> {
    Written {
        platform,
        clocks: Clocks {
            wall_ms_at_start: 1_700_000_000_000,
            boot_ms_at_start: 50_000,
        },
        boot: 0x0123_4567_89ab_cdef,
        tickets: &[("a", "0192f3a1c2e0-9f3c2a7e5b1d4c88")],
        seq_high: 4_183,
    }
}

fn published(sample: &Sample) -> maitred_scenario::Published {
    let mut publisher = Publisher::new(&MachineConfig::for_tests());
    let a = Amount {
        cpu_millicores: 1_850,
        mem_mb: 760,
    };
    publisher.publish(
        sample,
        &[Running::measured("a", a).holder_passed(sample.at - ms(100))],
        None,
    )
}

#[test]
fn times_are_written_as_rfc_3339_in_utc_with_milliseconds() {
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(rfc3339(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    assert_eq!(rfc3339(951_782_400_000), "2000-02-29T00:00:00.000Z");
}

#[test]
fn a_linux_load_file_has_every_field_the_doc_lists_and_pressure_but_no_macos_ones() {
    let json = now_json(&published(&Sample::at(ms(2_000))), &inputs(Platform::Linux));

    assert_eq!(
        json,
        concat!(
            "{\"v\":\"4.0\",",
            "\"sampled_at\":\"2023-11-14T22:13:22.000Z\",\"sampled_at_ms\":52000,",
            "\"boot\":\"0123456789abcdef\",\"interval_ms\":200,\"fresh\":true,",
            "\"woke_at\":null,\"woke_at_ms\":null,",
            "\"capacity\":{\"cpu_millicores\":8000,\"mem_mb\":16384},",
            "\"cpu_used_millicores\":1850,\"cpu_pressure\":0.231,",
            "\"mem_available_mb\":15624,",
            "\"quiet_streak\":0,\"over_cpu_streak\":0,\"over_gpu_streak\":0,",
            "\"pressure\":{\"cpu_some_avg10\":0.0,\"mem_some_avg10\":0.0,\"level\":\"normal\"},",
            "\"jobs\":{\"0192f3a1c2e0-9f3c2a7e5b1d4c88\":{\"cpu_millicores\":1850,\"mem_mb\":760}},",
            "\"holder_passes\":{\"0192f3a1c2e0-9f3c2a7e5b1d4c88\":51900},",
            "\"seq_high\":4183,\"missing\":[]}"
        )
    );
}

#[test]
fn a_macos_load_file_has_threads_and_sleep_assertions_but_no_pressure() {
    let json = now_json(&published(&Sample::at(ms(2_000))), &inputs(Platform::MacOs));

    assert!(json.contains("\"maitred_running_threads\":2,"), "{json}");
    assert!(
        json.contains("\"sleep_assertions\":{\"count\":0,\"unexpected\":0}"),
        "{json}"
    );
    assert!(!json.contains("\"pressure\":{"), "{json}");
}

#[test]
fn a_reading_the_sampler_couldnt_take_is_null_and_named_in_missing() {
    let json = now_json(
        &published(&Sample::at(ms(2_000)).missing(Reading::Memory)),
        &inputs(Platform::Linux),
    );

    assert!(json.contains("\"mem_available_mb\":null,"), "{json}");
    assert!(
        json.contains("\"missing\":[\"mem_available_mb\"]"),
        "{json}"
    );
}

#[test]
fn a_gpu_reading_is_absent_without_a_gpu_and_written_with_one() {
    let without = now_json(&published(&Sample::at(ms(2_000))), &inputs(Platform::Linux));
    let with = now_json(
        &published(&Sample::at(ms(2_000)).gpu(120)),
        &inputs(Platform::Linux),
    );

    assert!(!without.contains("\"gpu\""), "{without}");
    assert!(
        with.contains("\"gpu\":{\"utilization\":0.12,\"mem_used_mb\":0},"),
        "{with}"
    );
}

#[test]
fn a_wake_is_written_on_both_clocks() {
    let json = now_json(
        &published(&Sample::at(ms(2_000)).woke_at(ms(1_900))),
        &inputs(Platform::Linux),
    );

    assert!(json.contains("\"fresh\":false,"), "{json}");
    assert!(
        json.contains("\"woke_at\":\"2023-11-14T22:13:21.900Z\",\"woke_at_ms\":51900,"),
        "{json}"
    );
}

#[test]
fn a_running_job_whose_ticket_replay_hasnt_found_isnt_listed() {
    let mut no_tickets = inputs(Platform::Linux);
    no_tickets.tickets = &[];

    let json = now_json(&published(&Sample::at(ms(2_000))), &no_tickets);

    assert!(json.contains("\"jobs\":{},"), "{json}");
    assert!(json.contains("\"holder_passes\":{},"), "{json}");
}

#[test]
fn an_unread_pressure_reading_writes_the_pressure_block_as_null_not_normal() {
    let json = now_json(
        &published(&Sample::at(ms(2_000)).missing(Reading::CpuPressure)),
        &inputs(Platform::Linux),
    );

    assert!(json.contains("\"cpu_pressure\":null,"), "{json}");
    assert!(json.contains("\"pressure\":null,"), "{json}");
}
