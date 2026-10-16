//! The invariant checker, against hand-made machine roots that each break one invariant (ARCHITECTURE.md, Test
//! kit: "The checker is tested too").

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::{env, process};

use maitred_testkit::checker::{Checker, Violation};
use maitred_testkit::ticket::{Header, encode_slot};

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A fresh machine root and project, each its own empty directory under the temp dir.
fn roots() -> (PathBuf, PathBuf) {
    let base = env::temp_dir().join(format!(
        "checker-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let root = base.join("root");
    let project = base.join("project");
    fs::create_dir_all(root.join("tickets")).unwrap();
    fs::create_dir_all(project.join(".maitred/records")).unwrap();
    (root, project)
}

fn header(generation: u64) -> Header {
    Header {
        generation,
        major: 4,
        minor: 0,
        class: 0,
        state: 0,
        cpu_millicores: 1_000,
        mem_mb: 512,
        sid: 0,
        seq: 1,
        last_pass: 0,
        admitted_at: 0,
        recreated_at: 0,
        holder_start: 0,
        pgid: 0,
        holder_pid: 0,
        boot: 1,
    }
}

/// Writes a ticket of `generation`, in its first slot, with the other slot empty.
fn write_ticket(root: &Path, id: &str, generation: u64) -> PathBuf {
    let path = root.join("tickets").join(format!("{id}.tk"));
    let mut bytes = encode_slot(&header(generation), "{}").to_vec();
    bytes.extend_from_slice(&[0_u8; 4096]);
    fs::write(&path, bytes).unwrap();
    path
}

fn write_record(project: &Path, id: &str) {
    let dir = project.join(".maitred/records").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("record.json"), "{}").unwrap();
}

fn locked(path: &Path) -> File {
    let file = File::open(path).unwrap();
    file.lock().unwrap();
    file
}

#[test]
fn a_ticket_deleted_after_its_record_is_written_is_fine() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    let ticket = write_ticket(&root, "t1", 1);
    checker.observe(|_| false);

    write_record(&project, "t1");
    fs::remove_file(ticket).unwrap();

    assert_eq!(checker.observe(|_| false), vec![]);
}

#[test]
fn a_ticket_gone_with_no_record_is_a_violation() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    let ticket = write_ticket(&root, "t1", 1);
    checker.observe(|_| false);

    fs::remove_file(ticket).unwrap();

    assert_eq!(
        checker.observe(|_| false),
        vec![Violation::GoneWithoutRecord("t1".into())]
    );
}

#[test]
fn a_ticket_whose_generation_goes_back_is_a_violation() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    write_ticket(&root, "t1", 5);
    checker.observe(|_| false);

    write_ticket(&root, "t1", 4);

    assert_eq!(
        checker.observe(|_| false),
        vec![Violation::GenerationWentBack {
            id: "t1".into(),
            from: 5,
            to: 4
        }]
    );
}

#[test]
fn a_ticket_written_while_its_owner_holds_the_lock_is_fine() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    let ticket = write_ticket(&root, "t1", 1);
    let _owner = locked(&ticket);

    assert_eq!(checker.modified("t1", |_| false), vec![]);
}

#[test]
fn a_ticket_written_with_its_lock_free_is_a_violation() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    write_ticket(&root, "t1", 1);

    assert_eq!(
        checker.modified("t1", |_| false),
        vec![Violation::WrittenUnlocked("t1".into())]
    );
}

#[test]
fn a_free_lock_is_excused_when_the_runner_says_the_owner_died_after_the_write() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    write_ticket(&root, "t1", 1);

    assert_eq!(checker.modified("t1", |id| id == "t1"), vec![]);
}

#[test]
fn two_live_stop_markers_are_a_violation() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    let first = root.join("tickets/t1.stop");
    let second = root.join("tickets/t2.stop");
    fs::write(&first, "").unwrap();
    fs::write(&second, "").unwrap();
    let _a = locked(&first);
    let _b = locked(&second);

    assert_eq!(
        checker.observe(|_| false),
        vec![Violation::TwoLiveStopMarkers(vec![
            "t1".into(),
            "t2".into()
        ])]
    );
}

#[test]
fn a_stop_marker_whose_lock_is_free_doesnt_count_as_live() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    let first = root.join("tickets/t1.stop");
    fs::write(&first, "").unwrap();
    fs::write(root.join("tickets/t2.stop"), "").unwrap();
    let _a = locked(&first);

    assert_eq!(checker.observe(|_| false), vec![]);
}

#[test]
fn at_the_end_every_ticket_seen_has_its_record() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    write_ticket(&root, "t1", 1);
    write_ticket(&root, "t2", 1);
    checker.observe(|_| false);
    write_record(&project, "t1");

    assert_eq!(
        checker.finish(),
        vec![Violation::NoRecordAtEnd("t2".into())]
    );
}

#[test]
fn a_write_with_the_lock_free_is_fine_once_the_record_exists() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    write_ticket(&root, "t1", 1);
    write_record(&project, "t1");

    assert_eq!(checker.modified("t1", |_| false), vec![]);
}

#[test]
fn a_ticket_a_fault_deleted_is_excused() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    let ticket = write_ticket(&root, "t1", 1);
    checker.observe(|_| false);

    fs::remove_file(ticket).unwrap();

    assert_eq!(checker.observe(|id| id == "t1"), vec![]);
}

#[test]
fn a_recreated_ticket_whose_generation_goes_back_is_a_violation() {
    let (root, project) = roots();
    let mut checker = Checker::new(&root, &[&project]);
    let ticket = write_ticket(&root, "t1", 5);
    checker.observe(|_| false);
    fs::remove_file(ticket).unwrap();
    checker.observe(|id| id == "t1");

    // A recreated ticket's generation carries on from the last its supervisor wrote (ARCHITECTURE rev 145).
    write_ticket(&root, "t1", 3);

    assert_eq!(
        checker.observe(|_| false),
        vec![Violation::GenerationWentBack {
            id: "t1".into(),
            from: 5,
            to: 3
        }]
    );
}
