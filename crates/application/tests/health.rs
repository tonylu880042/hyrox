//! Whether the hub may be stopped right now (ADR 0009 §6).
//!
//! The appliance has a nightly maintenance window that updates and powers the machine off.
//! It asks this first. Getting it wrong means a class ending mid-station because a timer
//! fired at 23:30.

mod support;

use application::{health, Blocker, LiveSession};
use domain::{DeviceId, Instant, Session, SessionConfig, SessionMode, SessionStatus};

const NOW: Instant = Instant(2_000_000);

fn session(status: SessionStatus) -> LiveSession {
    let mut s = Session::new_draft("s1", "THU 19:00", SessionMode::Training);
    // Drive the real transitions rather than assigning the field: a state that the domain
    // cannot reach is not a state worth asserting about.
    match status {
        SessionStatus::Draft => {}
        SessionStatus::Ready => s.mark_ready().unwrap(),
        SessionStatus::Running => {
            s.mark_ready().unwrap();
            s.start().unwrap();
        }
        SessionStatus::Paused => {
            s.mark_ready().unwrap();
            s.start().unwrap();
            s.pause(Instant(1)).unwrap();
        }
        SessionStatus::Completed => {
            s.mark_ready().unwrap();
            s.start().unwrap();
            s.complete().unwrap();
        }
        SessionStatus::Cancelled => s.cancel().unwrap(),
    }
    LiveSession::new(s, SessionConfig::new("s1"), Instant(1_000_000))
}

#[test]
fn a_finished_class_is_safe_to_stop() {
    for status in [SessionStatus::Draft, SessionStatus::Completed, SessionStatus::Cancelled] {
        let view = health(&session(status), NOW);
        assert!(view.safe_to_stop, "{status:?} should be safe to stop");
        assert!(view.blocked_by.is_empty());
    }
}

/// READY and PAUSED count as live. A paused class is a coffee break, not an ending, and a
/// READY one is a coach about to press start.
#[test]
fn a_live_class_blocks_the_maintenance_window() {
    for status in [SessionStatus::Ready, SessionStatus::Running, SessionStatus::Paused] {
        let view = health(&session(status), NOW);
        assert!(!view.safe_to_stop, "{status:?} must block");
        assert_eq!(view.blocked_by, vec![Blocker::ClassRunning]);
        assert!(view.class_live);
    }
}

#[test]
fn the_view_reports_the_session_status_by_name() {
    assert_eq!(health(&session(SessionStatus::Running), NOW).session_status, "RUNNING");
    assert_eq!(health(&session(SessionStatus::Paused), NOW).session_status, "PAUSED");
}

/// A device still holding unacknowledged events in its journal (CLAUDE.md 18). Stopping is
/// survivable -- the journal exists precisely for this -- but there is no reason to do it
/// deliberately when tomorrow will do.
#[test]
fn a_device_with_a_backlog_blocks_the_maintenance_window() {
    let mut state = session(SessionStatus::Completed);
    let device = DeviceId::parse("esp32-a4cf128b3d91").expect("a device id");
    support::note_backlog(&mut state, &device, 42, NOW);

    let view = health(&state, NOW);

    assert!(!view.safe_to_stop);
    assert_eq!(view.blocked_by, vec![Blocker::DeviceBacklog]);
    assert_eq!(view.devices_with_backlog, 1);
}

#[test]
fn a_device_that_has_caught_up_does_not_block() {
    let mut state = session(SessionStatus::Completed);
    let device = DeviceId::parse("esp32-a4cf128b3d91").expect("a device id");
    support::note_backlog(&mut state, &device, 0, NOW);

    let view = health(&state, NOW);

    assert!(view.safe_to_stop);
    assert_eq!(view.devices_with_backlog, 0);
}

#[test]
fn both_blockers_are_reported_together_so_one_fix_does_not_hide_the_other() {
    let mut state = session(SessionStatus::Running);
    let device = DeviceId::parse("esp32-a4cf128b3d91").expect("a device id");
    support::note_backlog(&mut state, &device, 7, NOW);

    let view = health(&state, NOW);

    assert!(!view.safe_to_stop);
    assert_eq!(view.blocked_by, vec![Blocker::ClassRunning, Blocker::DeviceBacklog]);
}
