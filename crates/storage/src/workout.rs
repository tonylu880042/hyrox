//! SQLite for the workout library: templates, exercises and machines (ADR 0008).
//!
//! Templates are stored as one JSON document per row, exactly like the session snapshot in
//! migration 0003 and for the same reason: the blocks are a nested, ordered structure the
//! hub reads whole and writes whole, and normalising them would buy joins nobody performs.
//! The columns beside the document are the ones a listing screen filters and sorts on.

use crate::{Store, StoreError};
use domain::{
    Exercise, ExerciseCategory, ExerciseLibrary, PhysicalStation, StationMap, TargetType,
    TemplateCategory, TemplateSource, WorkoutBlock, WorkoutTemplate,
};
use sqlx::Row;

impl Store {
    pub async fn save_template(&self, t: &WorkoutTemplate) -> Result<(), StoreError> {
        let blocks = serde_json::to_string(&t.blocks)?;
        sqlx::query(
            "INSERT INTO workout_templates
                (id, name, description, category, source, owner_id, version, difficulty,
                 estimated_duration_minutes, enabled, blocks_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                category = excluded.category,
                owner_id = excluded.owner_id,
                version = excluded.version,
                difficulty = excluded.difficulty,
                estimated_duration_minutes = excluded.estimated_duration_minutes,
                enabled = excluded.enabled,
                blocks_json = excluded.blocks_json,
                updated_at = excluded.updated_at",
        )
        .bind(&t.id)
        .bind(&t.name)
        .bind(&t.description)
        .bind(template_category_str(t.category))
        .bind(template_source_str(t.source))
        .bind(&t.owner_id)
        .bind(t.version as i64)
        .bind(&t.difficulty)
        .bind(t.estimated_duration_minutes.map(|m| m as i64))
        .bind(t.enabled as i64)
        .bind(blocks)
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn template(&self, id: &str) -> Result<Option<WorkoutTemplate>, StoreError> {
        let row = sqlx::query(TEMPLATE_BY_ID)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(template_from_row).transpose()
    }

    /// Every template, newest edit last. The listing order the screens want is applied in
    /// the application layer, so the fake and the real store cannot disagree about it.
    pub async fn templates(&self) -> Result<Vec<WorkoutTemplate>, StoreError> {
        sqlx::query(TEMPLATE_ALL)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(template_from_row)
            .collect()
    }

    pub async fn delete_template(&self, id: &str) -> Result<bool, StoreError> {
        let done = sqlx::query("DELETE FROM workout_templates WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn save_exercise(&self, e: &Exercise) -> Result<(), StoreError> {
        let supported = serde_json::to_string(&e.supported_target_types)?;
        sqlx::query(
            "INSERT INTO exercises
                (code, display_name, category, station_key, default_target_type,
                 supported_target_types, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(code) DO UPDATE SET
                display_name = excluded.display_name,
                category = excluded.category,
                station_key = excluded.station_key,
                default_target_type = excluded.default_target_type,
                supported_target_types = excluded.supported_target_types,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at",
        )
        .bind(&e.code)
        .bind(&e.display_name)
        .bind(category_str(e.category))
        .bind(&e.station_key)
        .bind(target_type_str(e.default_target_type))
        .bind(supported)
        .bind(e.enabled as i64)
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn exercises(&self) -> Result<ExerciseLibrary, StoreError> {
        let rows = sqlx::query(
            "SELECT code, display_name, category, station_key, default_target_type,
                    supported_target_types, enabled
             FROM exercises ORDER BY rowid",
        )
        .fetch_all(&self.pool)
        .await?;
        let exercises = rows
            .into_iter()
            .map(|r| {
                Ok(Exercise {
                    code: r.get("code"),
                    display_name: r.get("display_name"),
                    category: parse_category(r.get::<String, _>("category").as_str())?,
                    station_key: r.get("station_key"),
                    default_target_type: parse_target_type(
                        r.get::<String, _>("default_target_type").as_str(),
                    )?,
                    supported_target_types: serde_json::from_str(
                        r.get::<String, _>("supported_target_types").as_str(),
                    )?,
                    enabled: r.get::<i64, _>("enabled") != 0,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        Ok(ExerciseLibrary::new(exercises))
    }

    pub async fn save_station(&self, s: &PhysicalStation) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO stations (id, exercise_code, display_name, zone)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                exercise_code = excluded.exercise_code,
                display_name = excluded.display_name,
                zone = excluded.zone",
        )
        .bind(&s.id)
        .bind(&s.exercise_code)
        .bind(&s.display_name)
        .bind(&s.zone)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn stations(&self) -> Result<StationMap, StoreError> {
        let rows =
            sqlx::query("SELECT id, exercise_code, display_name, zone FROM stations ORDER BY id")
                .fetch_all(&self.pool)
                .await?;
        Ok(StationMap::new(
            rows.into_iter()
                .map(|r| PhysicalStation {
                    id: r.get("id"),
                    exercise_code: r.get("exercise_code"),
                    display_name: r.get("display_name"),
                    zone: r.get("zone"),
                })
                .collect(),
        ))
    }
}

/// Both template reads select the same columns, because `template_from_row` needs all of
/// them; keeping the two strings adjacent is what stops one drifting from the other.
const TEMPLATE_BY_ID: &str = "SELECT id, name, description, category, source, owner_id, version,
            difficulty, estimated_duration_minutes, enabled, blocks_json
     FROM workout_templates WHERE id = ?1";

const TEMPLATE_ALL: &str = "SELECT id, name, description, category, source, owner_id, version,
            difficulty, estimated_duration_minutes, enabled, blocks_json
     FROM workout_templates ORDER BY id";

fn template_from_row(r: sqlx::sqlite::SqliteRow) -> Result<WorkoutTemplate, StoreError> {
    let blocks: Vec<WorkoutBlock> =
        serde_json::from_str(r.get::<String, _>("blocks_json").as_str())?;
    Ok(WorkoutTemplate {
        id: r.get("id"),
        name: r.get("name"),
        description: r.get("description"),
        category: parse_template_category(r.get::<String, _>("category").as_str())?,
        source: parse_template_source(r.get::<String, _>("source").as_str())?,
        owner_id: r.get("owner_id"),
        version: r.get::<i64, _>("version") as u32,
        difficulty: r.get("difficulty"),
        estimated_duration_minutes: r
            .get::<Option<i64>, _>("estimated_duration_minutes")
            .map(|m| m as u32),
        enabled: r.get::<i64, _>("enabled") != 0,
        blocks,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn template_category_str(c: TemplateCategory) -> &'static str {
    match c {
        TemplateCategory::Foundational => "FOUNDATIONAL",
        TemplateCategory::Engine => "ENGINE",
        TemplateCategory::Power => "POWER",
        TemplateCategory::Complete => "COMPLETE",
        TemplateCategory::RaceSimulation => "RACE_SIMULATION",
        TemplateCategory::Custom => "CUSTOM",
    }
}

fn parse_template_category(s: &str) -> Result<TemplateCategory, StoreError> {
    Ok(match s {
        "FOUNDATIONAL" => TemplateCategory::Foundational,
        "ENGINE" => TemplateCategory::Engine,
        "POWER" => TemplateCategory::Power,
        "COMPLETE" => TemplateCategory::Complete,
        "RACE_SIMULATION" => TemplateCategory::RaceSimulation,
        "CUSTOM" => TemplateCategory::Custom,
        other => return Err(StoreError::Corrupt(format!("template category {other}"))),
    })
}

fn template_source_str(s: TemplateSource) -> &'static str {
    match s {
        TemplateSource::System => "SYSTEM",
        TemplateSource::Coach => "COACH",
    }
}

fn parse_template_source(s: &str) -> Result<TemplateSource, StoreError> {
    Ok(match s {
        "SYSTEM" => TemplateSource::System,
        "COACH" => TemplateSource::Coach,
        other => return Err(StoreError::Corrupt(format!("template source {other}"))),
    })
}

fn category_str(c: ExerciseCategory) -> &'static str {
    match c {
        ExerciseCategory::Run => "RUN",
        ExerciseCategory::Erg => "ERG",
        ExerciseCategory::Functional => "FUNCTIONAL",
        ExerciseCategory::Other => "OTHER",
    }
}

fn parse_category(s: &str) -> Result<ExerciseCategory, StoreError> {
    Ok(match s {
        "RUN" => ExerciseCategory::Run,
        "ERG" => ExerciseCategory::Erg,
        "FUNCTIONAL" => ExerciseCategory::Functional,
        "OTHER" => ExerciseCategory::Other,
        other => return Err(StoreError::Corrupt(format!("exercise category {other}"))),
    })
}

fn target_type_str(t: TargetType) -> &'static str {
    match t {
        TargetType::Distance => "DISTANCE",
        TargetType::Reps => "REPS",
        TargetType::Time => "TIME",
        TargetType::Calories => "CALORIES",
    }
}

fn parse_target_type(s: &str) -> Result<TargetType, StoreError> {
    Ok(match s {
        "DISTANCE" => TargetType::Distance,
        "REPS" => TargetType::Reps,
        "TIME" => TargetType::Time,
        "CALORIES" => TargetType::Calories,
        other => return Err(StoreError::Corrupt(format!("target type {other}"))),
    })
}
