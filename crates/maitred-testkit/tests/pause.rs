//! The pause controller, against fake paused processes that speak the protocol as ARCHITECTURE.md gives it (Test
//! kit: "The pause protocol"). maitred's own side of it comes with the store.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use maitred_scenario::PausePoint;
use maitred_testkit::pause::Controller;
use maitred_testkit::timing::{millis, wait_ceiling};

// What must arrive is waited for up to the ceiling, which only a failing test reaches (MT-054). The 50 ms checks that
// nothing arrives stay short: under load they can only pass wrongly, never fail wrongly.

/// A paused process: connects, writes `line`, and sends back what it read (empty at end of file).
fn pause_at(socket: &Path, line: &str) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.write_all(line.as_bytes()).unwrap();
    thread::spawn(move || {
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        tx.send(reply).unwrap();
    });
    rx
}

#[test]
fn every_point_the_doc_lists_is_held_until_go() {
    let controller = Controller::listen().unwrap();
    for point in PausePoint::ALL {
        let reply = pause_at(
            controller.path(),
            &format!("at {} pid 42 ticket t1\n", point.name()),
        );

        let paused = controller.next(wait_ceiling()).unwrap();

        assert_eq!(
            (paused.point, paused.pid, paused.ticket.as_deref()),
            (point, 42, Some("t1"))
        );
        assert!(
            reply.recv_timeout(millis(50)).is_err(),
            "{} wasn't held",
            point.name()
        );
        paused.go().unwrap();
        assert_eq!(reply.recv_timeout(wait_ceiling()).unwrap(), "go\n");
    }
}

#[test]
fn a_process_paused_before_its_ticket_exists_names_none() {
    let controller = Controller::listen().unwrap();
    let _reply = pause_at(controller.path(), "at admit.locked pid 7 ticket -\n");

    assert_eq!(controller.next(wait_ceiling()).unwrap().ticket, None);
}

#[test]
fn several_processes_wait_at_once_and_each_goes_only_when_told() {
    let controller = Controller::listen().unwrap();
    let first = pause_at(controller.path(), "at ticket.written pid 1 ticket a\n");
    let first_paused = controller.next(wait_ceiling()).unwrap();
    let second = pause_at(controller.path(), "at ticket.written pid 2 ticket b\n");
    let second_paused = controller.next(wait_ceiling()).unwrap();

    second_paused.go().unwrap();

    assert_eq!(second.recv_timeout(wait_ceiling()).unwrap(), "go\n");
    assert!(first.recv_timeout(millis(50)).is_err());
    first_paused.go().unwrap();
    assert_eq!(first.recv_timeout(wait_ceiling()).unwrap(), "go\n");
}

#[test]
fn a_paused_process_dropped_without_go_reads_end_of_file_and_so_continues() {
    let controller = Controller::listen().unwrap();
    let reply = pause_at(controller.path(), "at stop.marked pid 3 ticket c\n");

    drop(controller.next(wait_ceiling()).unwrap());

    assert_eq!(reply.recv_timeout(wait_ceiling()).unwrap(), "");
}

#[test]
fn a_point_the_doc_doesnt_list_is_refused() {
    let controller = Controller::listen().unwrap();
    let _reply = pause_at(controller.path(), "at ticket.before_sync pid 4 ticket d\n");

    let error = controller.next(wait_ceiling()).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[test]
fn a_line_not_in_the_protocols_shape_is_refused() {
    let controller = Controller::listen().unwrap();
    let _reply = pause_at(controller.path(), "at admit.locked pid four\n");

    let error = controller.next(wait_ceiling()).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[test]
fn waiting_for_a_process_that_never_comes_times_out() {
    let controller = Controller::listen().unwrap();

    let error = controller.next(millis(50)).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::TimedOut);
}

#[test]
fn the_environment_names_the_socket_and_the_points_in_the_order_given() {
    let controller = Controller::listen().unwrap();

    let env = controller.env(&[PausePoint::StopMarked, PausePoint::AdmitLocked]);

    assert_eq!(
        env,
        [
            (
                "MAITRED_TEST_PAUSE".to_string(),
                controller.path().to_str().unwrap().to_string()
            ),
            (
                "MAITRED_TEST_PAUSE_AT".to_string(),
                "stop.marked,admit.locked".to_string()
            ),
        ]
    );
}

#[test]
fn the_socket_path_fits_the_smallest_limit_any_platform_has() {
    let controller = Controller::listen().unwrap();

    // macOS's sun_path is 104 bytes, its terminating NUL included.
    assert!(controller.path().as_os_str().len() <= 103);
}

#[test]
fn a_directory_too_long_for_a_socket_is_refused_before_anything_is_made() {
    let long = std::env::temp_dir().join("d".repeat(120));

    let error = Controller::listen_in(&long).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(!long.exists());
}

#[test]
fn the_socket_and_its_directory_are_removed_when_the_controller_goes() {
    let controller = Controller::listen().unwrap();
    let dir = controller.path().parent().unwrap().to_path_buf();

    drop(controller);

    assert!(!dir.exists());
}
