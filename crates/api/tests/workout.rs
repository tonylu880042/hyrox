//! The workout library over HTTP (workout brief §17, §27; ADR 0008).

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{call, del, draft, get, post, put, running};

const DESK: &str = "COACH TABLET";

fn engine_800() -> serde_json::Value {
    json!({
        "id": "t1",
        "name": "HYROX Engine 800",
        "category": "ENGINE",
        "blocks": [{
            "name": "Main",
            "block_type": "SEQUENTIAL",
            "rounds": null,
            "duration": null,
            "rest": null,
            "exercises": [
                { "exercise_code": "RUN", "target": { "target_type": "DISTANCE", "value": 800, "unit": "METER" }, "weight": null, "time_limit": null, "notes": null },
                { "exercise_code": "WALL_BALL", "target": { "target_type": "REPS", "value": 50, "unit": "REPS" }, "weight": null, "time_limit": null, "notes": null }
            ]
        }]
    })
}

// --- reads ------------------------------------------------------------------------------

#[tokio::test]
async fn the_exercise_library_is_published_with_its_units() {
    let (router, _) = draft();

    let (status, body) = call(&router, get("/api/exercises")).await;

    assert_eq!(status, StatusCode::OK);
    let exercises = body["exercises"].as_array().expect("a list");
    assert_eq!(exercises.len(), 9);
    let wall_ball = exercises.iter().find(|e| e["code"] == "WALL_BALL").expect("wall ball");
    assert_eq!(wall_ball["station_key"], "WALL BALLS");
    assert_eq!(wall_ball["supported_target_types"], json!(["REPS", "TIME"]));
    // Every read carries its freshness (ADR 0001 D5).
    assert!(body["freshness"]["now"].is_i64());
}

#[tokio::test]
async fn an_empty_library_lists_no_templates() {
    let (router, _) = draft();
    let (status, body) = call(&router, get("/api/workout-templates")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["templates"].as_array().expect("a list").is_empty());
}

#[tokio::test]
async fn a_template_that_does_not_exist_is_a_404() {
    let (router, _) = draft();
    let (status, body) = call(&router, get("/api/workout-templates/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "UNKNOWN_TEMPLATE");
}

// --- writes -----------------------------------------------------------------------------

#[tokio::test]
async fn a_template_is_saved_and_then_readable() {
    let (router, store) = draft();

    let (status, body) =
        call(&router, post("/api/operator/templates", DESK, engine_800())).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["templates"].as_array().unwrap().len(), 1);

    let (status, body) = call(&router, get("/api/workout-templates/t1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["template"]["name"], "HYROX Engine 800");
    assert_eq!(body["template"]["source"], "COACH");
    assert_eq!(body["template"]["version"], 1);
    assert_eq!(store.audits().pop().expect("an audit").action, "TEMPLATE_CREATE");
}

#[tokio::test]
async fn saving_a_template_without_a_device_name_is_refused() {
    let (router, store) = draft();

    let (status, body) = call(&router, post("/api/operator/templates", "", engine_800())).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "OPERATOR_REQUIRED");
    assert!(store.audits().is_empty());
}

#[tokio::test]
async fn saving_over_a_template_advances_its_version() {
    let (router, _) = draft();
    call(&router, post("/api/operator/templates", DESK, engine_800())).await;

    let (status, _) = call(&router, put("/api/operator/templates/t1", DESK, engine_800())).await;

    assert_eq!(status, StatusCode::OK);
    let (_, body) = call(&router, get("/api/workout-templates/t1")).await;
    assert_eq!(body["template"]["version"], 2);
}

/// The id in the path wins, so a mismatched payload cannot write to a template the URL did
/// not name.
#[tokio::test]
async fn the_path_id_wins_over_the_body_id() {
    let (router, _) = draft();

    call(&router, put("/api/operator/templates/from-path", DESK, engine_800())).await;

    let (status, _) = call(&router, get("/api/workout-templates/from-path")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&router, get("/api/workout-templates/t1")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_template_with_no_work_in_it_is_still_storable_but_will_not_run() {
    // Saving an incomplete plan is normal -- a coach builds one over several taps. It is
    // creating a *class* from it that has to refuse (see below).
    let (router, _) = draft();
    let mut empty = engine_800();
    empty["blocks"] = json!([]);

    let (status, _) = call(&router, post("/api/operator/templates", DESK, empty)).await;

    assert_eq!(status, StatusCode::OK);
}

// --- system templates (brief §4, §13) ------------------------------------------------------

#[tokio::test]
async fn a_system_template_cannot_be_written_over() {
    let (router, store) = draft();
    store.seed_system_template("sys1", "HYROX Engine");

    let mut payload = engine_800();
    payload["id"] = json!("sys1");
    let (status, body) = call(&router, put("/api/operator/templates/sys1", DESK, payload)).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "TEMPLATE_NOT_EDITABLE");
}

#[tokio::test]
async fn a_system_template_cannot_be_deleted() {
    let (router, store) = draft();
    store.seed_system_template("sys1", "HYROX Engine");

    let (status, body) =
        call(&router, del("/api/operator/templates/sys1", DESK, json!({ "reason": "不要了" }))).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "TEMPLATE_NOT_EDITABLE");
}

#[tokio::test]
async fn deleting_a_template_needs_a_reason() {
    let (router, _) = draft();
    call(&router, post("/api/operator/templates", DESK, engine_800())).await;

    let (status, body) = call(&router, del("/api/operator/templates/t1", DESK, json!({}))).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "REASON_REQUIRED");
    let (status, _) = call(&router, get("/api/workout-templates/t1")).await;
    assert_eq!(status, StatusCode::OK, "a refused delete removes nothing");
}

#[tokio::test]
async fn a_coach_template_is_deleted_with_a_reason() {
    let (router, store) = draft();
    call(&router, post("/api/operator/templates", DESK, engine_800())).await;

    let (status, _) =
        call(&router, del("/api/operator/templates/t1", DESK, json!({ "reason": "重複" }))).await;

    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&router, get("/api/workout-templates/t1")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let audit = store.audits().into_iter().find(|a| a.action == "TEMPLATE_DELETE").expect("audited");
    assert_eq!(audit.reason.as_deref(), Some("重複"));
}

/// Scenario A over HTTP: duplicate the system template, then edit the copy.
#[tokio::test]
async fn scenario_a_a_system_template_is_duplicated_into_an_editable_one() {
    let (router, store) = draft();
    store.seed_system_template("sys1", "HYROX Engine 800");

    let (status, body) = call(
        &router,
        post(
            "/api/operator/templates/sys1/duplicate",
            DESK,
            json!({ "new_id": "t9", "name": "Friday Engine Class", "owner_id": "coach-ana" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["templates"].as_array().unwrap().len(), 2);

    let (_, copy) = call(&router, get("/api/workout-templates/t9")).await;
    assert_eq!(copy["template"]["source"], "COACH");
    assert_eq!(copy["template"]["owner_id"], "coach-ana");
    assert_eq!(copy["template"]["name"], "Friday Engine Class");
}

#[tokio::test]
async fn duplicating_a_template_that_does_not_exist_is_a_404() {
    let (router, _) = draft();
    let (status, body) = call(
        &router,
        post("/api/operator/templates/nope/duplicate", DESK, json!({ "new_id": "x", "name": "X" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "UNKNOWN_TEMPLATE");
}

// --- creating a class (brief §15, scenario B) ------------------------------------------------

fn create_class_body() -> serde_json::Value {
    json!({
        "template_id": "t1",
        "session_id": "class-1",
        "name": "FRI 19:00 Engine",
        "finish_policy": { "kind": "COACH_DECIDES" },
        "athletes": [{ "athlete_id": "a1", "display_name": "TONY" }]
    })
}

#[tokio::test]
async fn a_class_is_created_from_a_template_and_comes_back_as_a_draft() {
    let (router, _) = draft();
    call(&router, post("/api/operator/templates", DESK, engine_800())).await;

    let (status, body) = call(&router, post("/api/operator/class", DESK, create_class_body())).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["session"]["status"], "DRAFT");
    assert_eq!(body["session"]["name"], "FRI 19:00 Engine");
    assert_eq!(body["config_editable"], true, "today's tweaks come next");
    let stations: Vec<&str> = body["config"]["course"]["steps"]
        .as_array()
        .expect("compiled steps")
        .iter()
        .map(|s| s["station"].as_str().unwrap())
        .collect();
    assert_eq!(stations, ["RUN", "WALL BALLS"]);
}

#[tokio::test]
async fn creating_a_class_from_an_unrunnable_template_is_refused_by_name() {
    let (router, _) = draft();
    let mut empty = engine_800();
    empty["blocks"] = json!([]);
    call(&router, post("/api/operator/templates", DESK, empty)).await;

    let (status, body) = call(&router, post("/api/operator/class", DESK, create_class_body())).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "TEMPLATE_NOT_RUNNABLE");
}

#[tokio::test]
async fn creating_a_class_while_one_is_running_is_refused() {
    let (router, store) = running();
    store.seed_system_template("t1", "HYROX Engine 800");

    let (status, body) = call(&router, post("/api/operator/class", DESK, create_class_body())).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "CLASS_IN_PROGRESS");
}

#[tokio::test]
async fn creating_a_class_from_a_template_that_does_not_exist_is_a_404() {
    let (router, _) = draft();
    let (status, body) = call(&router, post("/api/operator/class", DESK, create_class_body())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "UNKNOWN_TEMPLATE");
}

// --- stages (brief §10) -----------------------------------------------------------------------

#[tokio::test]
async fn the_stage_list_is_published_per_athlete() {
    let (router, _) = draft();
    call(&router, post("/api/operator/templates", DESK, engine_800())).await;
    call(&router, post("/api/operator/class", DESK, create_class_body())).await;

    let (status, body) = call(&router, get("/api/stages")).await;

    assert_eq!(status, StatusCode::OK);
    let athletes = body["athletes"].as_array().expect("a list");
    assert_eq!(athletes.len(), 1);
    assert_eq!(athletes[0]["athlete_id"], "a1");
    assert_eq!(athletes[0]["current_stage"], 1);
    let stages = athletes[0]["stages"].as_array().expect("stages");
    assert_eq!(stages.len(), 2);
    assert_eq!(stages[0]["status"], "READY");
    assert_eq!(stages[0]["station"], "RUN");
    assert_eq!(stages[1]["status"], "PENDING");
    assert!(athletes[0]["expectation"].is_null(), "nobody is at a station yet");
}

#[tokio::test]
async fn a_class_with_no_course_publishes_no_stages() {
    let (router, _) = draft();
    let (status, body) = call(&router, get("/api/stages")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["athletes"].as_array().unwrap().is_empty());
}

// --- the maintenance window's question (ADR 0009 §6) ----------------------------------------

/// The one field a shell script reads. Deliberately outside the `freshness` envelope every
/// other read carries: `safe_to_stop` must be the whole answer.
#[tokio::test]
async fn health_reports_a_running_class_as_unsafe_to_stop() {
    let (router, _) = running();

    let (status, body) = call(&router, get("/api/health")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["session_status"], "RUNNING");
    assert_eq!(body["class_live"], true);
    assert_eq!(body["safe_to_stop"], false);
    assert_eq!(body["blocked_by"], json!(["CLASS_RUNNING"]));
    assert_eq!(body["version"], "test");
}

#[tokio::test]
async fn health_reports_a_draft_class_as_safe_to_stop() {
    let (router, _) = draft();

    let (status, body) = call(&router, get("/api/health")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["safe_to_stop"], true);
    assert!(body["blocked_by"].as_array().expect("a list").is_empty());
    assert_eq!(body["devices_with_backlog"], 0);
}

/// A completed class that a coach then reopens must stop being safe again -- the window
/// asks immediately before it acts, not once at the start of the night.
#[tokio::test]
async fn health_follows_the_session_rather_than_being_cached() {
    let (router, _) = draft();
    assert_eq!(call(&router, get("/api/health")).await.1["safe_to_stop"], true);

    call(&router, post("/api/operator/session/ready", DESK, json!({}))).await;

    let (_, body) = call(&router, get("/api/health")).await;
    assert_eq!(body["safe_to_stop"], false, "a READY class is a coach about to press start");
    assert_eq!(body["session_status"], "READY");
}
