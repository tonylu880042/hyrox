//! Course definition and session configuration (CLAUDE.md 9.1, 9.2, 12).
//!
//! A course is a plan, not a constraint: training records whatever actually happens
//! (CLAUDE.md 9.2), so nothing here may reject an out-of-order station.

use domain::*;

// --- course shape (CLAUDE.md 9.2) -----------------------------------------------------

#[test]
fn a_course_is_an_ordered_list_of_stations() {
    let course = Course::new(
        "Thursday Class",
        vec![
            CourseStep::new("RUN"),
            CourseStep::new("SKIERG"),
            CourseStep::new("WALL BALLS"),
        ],
    );
    assert_eq!(course.name, "Thursday Class");
    assert_eq!(course.len(), 3);
    assert_eq!(course.stations().collect::<Vec<_>>(), ["RUN", "SKIERG", "WALL BALLS"]);
    assert_eq!(course.step(1).map(|s| s.station.as_str()), Some("SKIERG"));
    assert!(course.step(3).is_none(), "out-of-range must be None, not a panic");
}

#[test]
fn a_station_may_repeat_and_each_occurrence_is_its_own_step() {
    // The HYROX pattern is run/station/run/station; the runs are distinct legs with
    // distinct targets, so they cannot be collapsed into one entry.
    let course = Course::new(
        "Intervals",
        vec![
            CourseStep::new("RUN").with_target(StationTarget::Distance { meters: 1_000 }),
            CourseStep::new("SKIERG").with_target(StationTarget::Distance { meters: 500 }),
            CourseStep::new("RUN").with_target(StationTarget::Distance { meters: 400 }),
        ],
    );
    assert_eq!(course.len(), 3);
    assert_eq!(course.occurrences("RUN"), 2);
    assert_eq!(
        course.step(0).unwrap().target,
        Some(StationTarget::Distance { meters: 1_000 })
    );
    assert_eq!(
        course.step(2).unwrap().target,
        Some(StationTarget::Distance { meters: 400 }),
        "the second RUN keeps its own target"
    );
}

#[test]
fn targets_cover_distance_repetitions_and_duration_and_stay_optional() {
    let plank = StationTarget::Duration { duration: Duration(90_000) };
    let course = Course::new(
        "Mixed",
        vec![
            CourseStep::new("ROWING").with_target(StationTarget::Distance { meters: 1_000 }),
            CourseStep::new("WALL BALLS").with_target(StationTarget::Repetitions { count: 100 }),
            CourseStep::new("PLANK").with_target(plank),
            CourseStep::new("STRETCH"),
        ],
    );
    assert_eq!(course.step(1).unwrap().target, Some(StationTarget::Repetitions { count: 100 }));
    assert_eq!(course.step(2).unwrap().target, Some(plank));
    assert_eq!(course.step(3).unwrap().target, None, "targets are optional (CLAUDE.md 9.2)");
}

#[test]
fn an_empty_course_is_legal() {
    // A drop-in training session may have no plan at all; events are still recorded.
    let course = Course::new("Open Gym", vec![]);
    assert!(course.is_empty());
    assert_eq!(course.occurrences("RUN"), 0);
}

#[test]
fn a_course_does_not_constrain_the_order_events_arrive_in() {
    // Guards CLAUDE.md 9.2 against regression: the course exposes no accept/reject verb,
    // and a station absent from the plan is simply not part of the plan.
    let course = Course::new("Plan", vec![CourseStep::new("RUN"), CourseStep::new("SKIERG")]);
    assert_eq!(course.occurrences("BURPEES"), 0);
    assert_eq!(course.len(), 2, "an unplanned station changes nothing about the plan");
}

// --- session configuration and the finish rule (CLAUDE.md 12, 28) ---------------------

#[test]
fn the_finish_policy_defaults_to_not_configured() {
    // CLAUDE.md 12: the finish rule is undecided. The default must say so out loud.
    assert_eq!(FinishPolicy::default(), FinishPolicy::NotConfigured);
    assert_eq!(SessionConfig::new("s1").finish_policy, FinishPolicy::NotConfigured);
}

#[test]
fn an_unconfigured_finish_policy_never_decides_an_athlete_is_finished() {
    let mut athlete = AthleteState::ready("a1", "Chen");
    let mut session = Session::new_draft("s1", "Test", SessionMode::Training);
    session.arm().unwrap();

    // Drive a full station, which is the shape most plausible finish rules would trigger on.
    let entry = ReaderBinding { station: "WALL BALLS".into(), mode: ReaderMode::Entry };
    let exit = ReaderBinding { station: "WALL BALLS".into(), mode: ReaderMode::Exit };
    interpret(&mut athlete, &entry, Instant(0), &session);
    interpret(&mut athlete, &exit, Instant(60_000), &session);

    assert_eq!(
        FinishPolicy::NotConfigured.evaluate(&athlete, Duration(0)),
        FinishDecision::Undetermined,
        "an undecided rule must return Undetermined, never NotFinished by accident"
    );
    assert_eq!(athlete.status, AthleteStatus::Active, "nothing may finish the athlete yet");
}

#[test]
fn session_configuration_carries_the_course_and_the_policy_together() {
    let config = SessionConfig::new("s1")
        .with_course(Course::new("Thursday Class", vec![CourseStep::new("RUN")]));

    assert_eq!(config.session_id, "s1");
    assert_eq!(config.course.as_ref().map(|c| c.name.as_str()), Some("Thursday Class"));
    assert_eq!(config.finish_policy, FinishPolicy::NotConfigured);
}

#[test]
fn a_session_may_run_without_any_course() {
    let config = SessionConfig::new("s1");
    assert!(config.course.is_none(), "a course is a plan, not a precondition");
}

// --- group-class finish rule (CLAUDE.md 12, settled with the user 2026-08-27) ----------

/// An athlete who has scanned in and is working, part-way through the course.
fn athlete_mid_class() -> AthleteState {
    let mut s = Session::new_draft("s1", "Class", SessionMode::Training);
    s.arm().unwrap();
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &ReaderBinding { station: "SKIERG".into(), mode: ReaderMode::Entry },
              Instant(0), &s);
    a
}

#[test]
fn a_class_ends_when_its_time_is_up_even_mid_course() {
    // A one-hour class: most athletes will not have finished all eight stations, and that
    // is the normal outcome rather than an error.
    let policy = FinishPolicy::ClassDuration { limit: Duration(3_600_000) };
    let a = athlete_mid_class();
    assert_eq!(a.runs.len(), 1, "still on the first station");

    assert_eq!(policy.evaluate(&a, Duration(3_599_999)), FinishDecision::NotFinished);
    assert_eq!(policy.evaluate(&a, Duration(3_600_000)), FinishDecision::Finished);
}

#[test]
fn an_athlete_who_never_scanned_in_did_not_take_part() {
    let policy = FinishPolicy::ClassDuration { limit: Duration(3_600_000) };
    let never_started = AthleteState::ready("a2", "Lin");
    assert_eq!(
        policy.evaluate(&never_started, Duration(7_200_000)),
        FinishDecision::NotFinished,
        "the class ended, but this athlete never started one"
    );
}

#[test]
fn coach_decides_never_finishes_anyone_on_its_own() {
    let policy = FinishPolicy::CoachDecides;
    let a = athlete_mid_class();
    assert_eq!(policy.evaluate(&a, Duration(86_400_000)), FinishDecision::NotFinished);
}

#[test]
fn finishing_keeps_an_open_station_open() {
    // No reader said the athlete left, so inventing an exit time would fabricate a split
    // that nothing observed (CLAUDE.md 19).
    let mut a = athlete_mid_class();
    assert_eq!(a.station_state, StationState::Inside);

    domain::finish(&mut a, Instant(3_600_000));

    assert_eq!(a.status, AthleteStatus::Finished);
    assert_eq!(a.runs[0].exited_at, None, "the unfinished station stays unfinished");
    assert_eq!(a.current_station.as_deref(), Some("SKIERG"), "we still know where they were");
}

#[test]
fn a_finished_athlete_stays_finished_under_any_policy() {
    let mut a = athlete_mid_class();
    domain::finish(&mut a, Instant(3_600_000));
    for policy in [
        FinishPolicy::NotConfigured,
        FinishPolicy::CoachDecides,
        FinishPolicy::ClassDuration { limit: Duration(3_600_000) },
    ] {
        assert_eq!(policy.evaluate(&a, Duration(0)), FinishDecision::Finished);
    }
}

#[test]
fn a_member_carries_the_profile_fields_the_gym_app_supplies() {
    // Confirmed with the user: gender, age and photo, with height/weight optional.
    let mut m = MemberRef::new("M-0417", "CHEN YU-TING", MembershipStatus::Expired);
    m.gender = Some(Gender::Female);
    m.age = Some(34);
    m.photo_url = Some("https://example.invalid/m/0417.jpg".into());

    assert_eq!(m.height_cm, None, "height is optional and may simply be absent");
    // Membership validity is informational: an expired member is still timed.
    assert_eq!(m.status, MembershipStatus::Expired);
}

#[test]
fn a_finished_athletes_clocks_stop() {
    // The live screen showed FINISHED cards whose totals kept climbing. A finished
    // athlete's time is a result, not a running clock.
    let mut a = athlete_mid_class();
    domain::finish(&mut a, Instant(3_600_000));

    let long_after = Instant(9_999_999);
    assert_eq!(a.elapsed(long_after), Some(Duration(3_600_000)), "total freezes at the finish");
    assert_eq!(a.current_leg(long_after), Some(Duration(3_600_000)), "so does the station leg");

    // And it stays frozen however long the screen stays up.
    assert_eq!(a.elapsed(Instant(99_999_999)), a.elapsed(long_after));
}

#[test]
fn an_unfinished_athletes_clock_still_runs() {
    let a = athlete_mid_class();
    assert_eq!(a.elapsed(Instant(60_000)), Some(Duration(60_000)));
    assert_eq!(a.elapsed(Instant(120_000)), Some(Duration(120_000)));
}
