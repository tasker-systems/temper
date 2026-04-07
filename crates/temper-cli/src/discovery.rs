use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    #[serde(rename = "resource_create")]
    ResourceCreate {
        ts: String,
        doc_type: String,
        title: String,
        path: String,
        context: String,
    },
    #[serde(rename = "resource_update")]
    ResourceUpdate {
        ts: String,
        doc_type: String,
        slug: String,
        context: String,
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
