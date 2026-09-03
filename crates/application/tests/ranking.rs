//! Placing competitors (ADR 0010).
//!
//! A ranking is only honest where "finished" has a meaning. Under `CourseComplete` it does
//! -- completing the course, timed at the last station's exit -- so first place is a fact.
//! Under a class-duration rule everybody stops at the same moment with different amounts of
//! work done, and ordering them by time would rank people who did different things.

mod support;

use application::{results, Ordering};
use domain::{Duration, FinishPolicy};
use support::FakeStore;

/// Builds a stored session whose athletes finished at the given offsets. `None` never
/// finished.
fn session_with(policy: FinishPolicy, finishes: &[(&str, Option<i64>)]) -> FakeStore {
    let store = FakeStore::new();
    store.seed_finished_session("s1", policy, finishes);
    store
}

#[tokio::test]
async fn course_completion_ranks_by_who_finished_first() {
    let store = session_with(
        FinishPolicy::CourseComplete,
        &[
            ("A", Some(3_600_000)),
            ("B", Some(3_100_000)),
            ("C", Some(3_400_000)),
        ],
    );

    let r = results(&store, "s1").await.unwrap().expect("results");

    assert_eq!(r.ordering, Ordering::FinishTime);
    let order: Vec<(&str, Option<usize>)> = r
        .rows
        .iter()
        .map(|row| (row.name.as_str(), row.place))
        .collect();
    assert_eq!(order, [("B", Some(1)), ("C", Some(2)), ("A", Some(3))]);
}

/// Somebody still on the course has no place. `None` is not last -- they have not finished.
#[tokio::test]
async fn an_unfinished_competitor_has_no_place_and_sorts_last() {
    let store = session_with(
        FinishPolicy::CourseComplete,
        &[("A", None), ("B", Some(3_100_000)), ("C", None)],
    );

    let r = results(&store, "s1").await.unwrap().expect("results");

    assert_eq!(r.rows[0].name, "B");
    assert_eq!(r.rows[0].place, Some(1));
    assert!(r.rows[1..].iter().all(|row| row.place.is_none()));
}

/// Standard competition ranking: a shared time shares a place, and the next place skips.
#[tokio::test]
async fn a_dead_heat_shares_a_place_and_the_next_one_skips() {
    let store = session_with(
        FinishPolicy::CourseComplete,
        &[
            ("A", Some(3_100_000)),
            ("B", Some(3_100_000)),
            ("C", Some(3_400_000)),
        ],
    );

    let places: Vec<Option<usize>> = results(&store, "s1")
        .await
        .unwrap()
        .unwrap()
        .rows
        .iter()
        .map(|r| r.place)
        .collect();

    assert_eq!(places, [Some(1), Some(1), Some(3)]);
}

/// The training case, and the reason ranking is not simply always on: everyone stops when
/// the clock runs out, having done different amounts of work.
#[tokio::test]
async fn a_class_that_ends_on_the_clock_is_not_ranked() {
    let store = session_with(
        FinishPolicy::ClassDuration {
            limit: Duration(3_600_000),
        },
        &[
            ("A", Some(3_600_000)),
            ("B", Some(3_600_000)),
            ("C", Some(3_600_000)),
        ],
    );

    let r = results(&store, "s1").await.unwrap().expect("results");

    assert_eq!(r.ordering, Ordering::Bib);
    assert!(
        r.rows.iter().all(|row| row.place.is_none()),
        "a timed class has no placings"
    );
}

#[tokio::test]
async fn a_session_with_no_finish_rule_is_not_ranked() {
    let store = session_with(
        FinishPolicy::NotConfigured,
        &[("A", Some(1)), ("B", Some(2))],
    );

    let r = results(&store, "s1").await.unwrap().expect("results");

    assert_eq!(r.ordering, Ordering::Bib);
    assert!(r.rows.iter().all(|row| row.place.is_none()));
}

/// Bib order is still bib order: the roster's own numbering, not the order the store
/// happened to return rows in.
#[tokio::test]
async fn an_unranked_session_comes_back_in_bib_order() {
    let store = session_with(
        FinishPolicy::CoachDecides,
        &[("A", None), ("B", None), ("C", None)],
    );

    let r = results(&store, "s1").await.unwrap().expect("results");
    let names: Vec<&str> = r.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["A", "B", "C"]);
}
