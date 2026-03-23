use std::path::Path;
use serde::{Deserialize, Serialize};
use crate::error::Result;

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
    #[serde(rename = "ticket_create")]
    TicketCreate {
        ts: String,
        project: String,
        ticket: String,
        milestone: String,
        title: String,
    },
    #[serde(rename = "ticket_move")]
    TicketMove {
        ts: String,
        project: String,
        ticket: String,
        from_stage: String,
        to_stage: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        from_milestone: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_milestone: Option<String>,
    },
    #[serde(rename = "ticket_done")]
    TicketDone {
        ts: String,
        project: String,
        ticket: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pr: Option<String>,
    },
    #[serde(rename = "milestone_create")]
    MilestoneCreate {
        ts: String,
        project: String,
        milestone: String,
        title: String,
    },
    #[serde(rename = "milestone_update")]
    MilestoneUpdate {
        ts: String,
        project: String,
        milestone: String,
        status: String,
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
