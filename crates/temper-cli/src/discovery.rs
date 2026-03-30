use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    #[serde(rename = "note_create")]
    NoteCreate {
        ts: String,
        note_type: String,
        title: String,
        path: String,
        project: String,
    },
    #[serde(rename = "task_create")]
    TaskCreate {
        ts: String,
        context: String,
        task: String,
        goal: String,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },
    #[serde(rename = "task_move")]
    TaskMove {
        ts: String,
        context: String,
        task: String,
        from_stage: String,
        to_stage: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        from_goal: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_goal: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from_effort: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_effort: Option<String>,
    },
    #[serde(rename = "task_done")]
    TaskDone {
        ts: String,
        context: String,
        task: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pr: Option<String>,
    },
    #[serde(rename = "goal_create")]
    GoalCreate {
        ts: String,
        context: String,
        goal: String,
        title: String,
    },
    #[serde(rename = "goal_update")]
    GoalUpdate {
        ts: String,
        context: String,
        goal: String,
        status: String,
    },
    #[serde(rename = "normalize")]
    Normalize {
        ts: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        project: Option<String>,
        ids_backfilled: u32,
        files_moved: u32,
        stages_migrated: u32,
        slugs_fixed: u32,
        frontmatter_fixed: u32,
    },
}

pub fn append_event(state_dir: &Path, event: &Event) -> Result<()> {
    let log_path = state_dir.join("events.jsonl");
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(event)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    writeln!(file, "{json}")?;
    Ok(())
}
