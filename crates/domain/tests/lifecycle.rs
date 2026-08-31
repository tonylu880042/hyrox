//! Session lifecycle: the six states of the class session, and the clock that pauses
//! with it (CLAUDE.md 19 of the workout brief; ADR 0008).

use domain::{ClassClock, Duration, Instant, Session, SessionError, SessionMode, SessionStatus};

fn draft() -> Session {
    Session::new_draft("s1", "Friday Engine", SessionMode::Training)
}

fn running() -> Session {
    let mut s = draft();
    s.mark_ready().unwrap();
    s.start().unwrap();
    s
}

#[test]
fn a_new_session_is_a_draft() {
    assert_eq!(draft().status, SessionStatus::Draft);
}

#[test]
fn the_happy_path_is_draft_ready_running_completed() {
    let mut s = draft();
    s.mark_ready().unwrap();
    assert_eq!(s.status, SessionStatus::Ready);
    s.start().unwrap();
    assert_eq!(s.status, SessionStatus::Running);
    s.complete().unwrap();
    assert_eq!(s.status, SessionStatus::Completed);
}

#[test]
fn a_draft_cannot_be_started_without_being_made_ready() {
    let mut s = draft();
    assert_eq!(
        s.start(),
        Err(SessionError::IllegalTransition {
            from: SessionStatus::Draft,
            to: SessionStatus::Running
        })
    );
    assert_eq!(s.status, SessionStatus::Draft);
}

#[test]
fn running_pauses_and_resumes() {
    let mut s = running();
    s.pause(Instant(1_000)).unwrap();
    assert_eq!(s.status, SessionStatus::Paused);
    s.resume(Instant(4_000)).unwrap();
    assert_eq!(s.status, SessionStatus::Running);
}

#[test]
fn a_paused_session_completes_without_being_resumed_first() {
    let mut s = running();
    s.pause(Instant(1_000)).unwrap();
    s.complete().unwrap();
    assert_eq!(s.status, SessionStatus::Completed);
}

#[test]
fn every_live_state_can_be_cancelled() {
    for mut s in [draft(), {
        let mut r = draft();
        r.mark_ready().unwrap();
        r
    }, running(), {
        let mut p = running();
        p.pause(Instant(1)).unwrap();
        p
    }] {
        s.cancel().unwrap();
        assert_eq!(s.status, SessionStatus::Cancelled);
    }
}

#[test]
fn a_completed_session_cannot_be_restarted() {
    let mut s = running();
    s.complete().unwrap();
    assert_eq!(
        s.start(),
        Err(SessionError::IllegalTransition {
            from: SessionStatus::Completed,
            to: SessionStatus::Running
        })
    );
}

#[test]
fn a_cancelled_session_cannot_be_restarted() {
    let mut s = running();
    s.cancel().unwrap();
    assert_eq!(
        s.start(),
        Err(SessionError::IllegalTransition {
            from: SessionStatus::Cancelled,
            to: SessionStatus::Running
        })
    );
    assert_eq!(s.reopen(), Err(SessionError::IllegalTransition {
        from: SessionStatus::Cancelled,
        to: SessionStatus::Running
    }));
}

/// ADR 0001 D2 kept: a mis-tap on a busy floor must not force a new session. It is a
/// correction, not the ordinary transition, which is why `start` refuses it and `reopen`
/// -- which the application layer will not run without a stated reason -- does not.
#[test]
fn a_completed_session_can_be_reopened_as_a_correction() {
    let mut s = running();
    s.complete().unwrap();
    s.reopen().unwrap();
    assert_eq!(s.status, SessionStatus::Running);
}

#[test]
fn only_a_running_session_accepts_events() {
    let mut s = draft();
    assert!(!s.accepts_events());
    s.mark_ready().unwrap();
    assert!(!s.accepts_events());
    s.start().unwrap();
    assert!(s.accepts_events());
    s.pause(Instant(1)).unwrap();
    assert!(!s.accepts_events(), "a paused class is not timing anybody");
    s.resume(Instant(2)).unwrap();
    s.complete().unwrap();
    assert!(!s.accepts_events());
}

#[test]
fn configuration_is_editable_before_the_class_starts_and_not_after() {
    let mut s = draft();
    assert!(s.accepts_config_edits());
    s.mark_ready().unwrap();
    assert!(s.accepts_config_edits(), "session-specific tweaks happen in READY");
    s.start().unwrap();
    assert!(!s.accepts_config_edits());
}

#[test]
fn a_ready_session_goes_back_to_draft_freely() {
    let mut s = draft();
    s.mark_ready().unwrap();
    s.back_to_draft().unwrap();
    assert_eq!(s.status, SessionStatus::Draft);
}

#[test]
fn a_running_session_goes_back_to_draft_only_before_anything_is_interpreted() {
    let mut s = running();
    s.back_to_draft().unwrap();
    assert_eq!(s.status, SessionStatus::Draft);

    let mut s = running();
    s.interpreted_event_count = 1;
    assert_eq!(s.back_to_draft(), Err(SessionError::HasInterpretedEvents));
}

// --- the class clock -------------------------------------------------------------------

#[test]
fn an_unpaused_clock_is_wall_time_since_the_start() {
    let clock = ClassClock::started_at(Instant(1_000));
    assert_eq!(clock.elapsed(Instant(4_000)), Duration(3_000));
}

#[test]
fn a_paused_clock_is_frozen_at_the_moment_it_paused() {
    let mut clock = ClassClock::started_at(Instant(1_000));
    clock.pause(Instant(3_000));
    assert_eq!(clock.elapsed(Instant(9_000)), Duration(2_000));
}

#[test]
fn paused_time_is_excluded_after_resuming() {
    let mut clock = ClassClock::started_at(Instant(0));
    clock.pause(Instant(10_000));
    clock.resume(Instant(25_000));
    // 10s ran, 15s paused, then 5s more.
    assert_eq!(clock.elapsed(Instant(30_000)), Duration(15_000));
}

#[test]
fn pauses_accumulate() {
    let mut clock = ClassClock::started_at(Instant(0));
    clock.pause(Instant(1_000));
    clock.resume(Instant(2_000));
    clock.pause(Instant(3_000));
    clock.resume(Instant(9_000));
    assert_eq!(clock.elapsed(Instant(10_000)), Duration(3_000));
}

/// What a duration-based finish rule needs: the wall-clock moment at which the class had
/// been running for `limit`. Without the pause offset a paused class would stop everyone's
/// clock early (CLAUDE.md 12, 17).
#[test]
fn the_wall_clock_instant_of_an_elapsed_target_includes_paused_time() {
    let mut clock = ClassClock::started_at(Instant(0));
    clock.pause(Instant(1_000));
    clock.resume(Instant(6_000));
    assert_eq!(clock.instant_at_elapsed(Duration(3_000)), Instant(8_000));
}

#[test]
fn a_clock_that_never_paused_maps_an_elapsed_target_straight_onto_the_wall_clock() {
    let clock = ClassClock::started_at(Instant(500));
    assert_eq!(clock.instant_at_elapsed(Duration(3_000)), Instant(3_500));
}

/// Pausing twice without resuming must not throw the first pause away, and resuming a
/// clock that is not paused must not invent negative time.
#[test]
fn redundant_pause_and_resume_are_harmless() {
    let mut clock = ClassClock::started_at(Instant(0));
    clock.resume(Instant(1_000));
    assert_eq!(clock.elapsed(Instant(2_000)), Duration(2_000));
    clock.pause(Instant(3_000));
    clock.pause(Instant(4_000));
    clock.resume(Instant(5_000));
    assert_eq!(clock.elapsed(Instant(6_000)), Duration(4_000));
}
