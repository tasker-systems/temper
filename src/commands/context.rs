use crate::actions::types::{
    ContextChunk, ContextGroup, ContextHop, ContextNoteDetail, ContextResults,
};
use crate::config::Config;
use crate::error::Result;
use crate::format::OutputFormat;
use serde::Serialize;
use std::fmt;

#[derive(Debug, Serialize)]
struct ChunkResult {
    score: f32,
    header_path: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct GroupedResult {
    file_path: String,
    note_type: String,
    title: String,
    chunks: Vec<ChunkResult>,
}

#[derive(Debug, Serialize)]
struct NoteDetail {
    path: String,
    title: String,
    tags: Vec<String>,
    content: String,
}

#[derive(Debug, Serialize)]
struct ContextOutput {
    topic: String,
    primary: Option<NoteDetail>,
    related_chunks: Vec<GroupedResult>,
    hop: usize,
}

impl From<ContextChunk> for ChunkResult {
    fn from(c: ContextChunk) -> Self {
        ChunkResult {
            score: c.score,
            header_path: c.header_path,
            content: c.content,
        }
    }
}

impl From<ContextGroup> for GroupedResult {
    fn from(g: ContextGroup) -> Self {
        GroupedResult {
            file_path: g.file_path,
            note_type: g.note_type,
            title: g.title,
            chunks: g.chunks.into_iter().map(ChunkResult::from).collect(),
        }
    }
}

impl From<ContextNoteDetail> for NoteDetail {
    fn from(d: ContextNoteDetail) -> Self {
        NoteDetail {
            path: d.path,
            title: d.title,
            tags: d.tags,
            content: d.content,
        }
    }
}

impl From<ContextHop> for ContextOutput {
    fn from(hop: ContextHop) -> Self {
        ContextOutput {
            topic: hop.topic,
            primary: hop.primary.map(NoteDetail::from),
            related_chunks: hop
                .related_chunks
                .into_iter()
                .map(GroupedResult::from)
                .collect(),
            hop: hop.hop,
        }
    }
}

impl fmt::Display for ContextOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== Context: {} ===", self.topic)?;

        if let Some(ref primary) = self.primary {
            if !primary.tags.is_empty() {
                writeln!(f, "tags: {}", primary.tags.join(", "))?;
            }
            writeln!(f)?;
            writeln!(f, "{}", primary.content)?;
        }

        if !self.related_chunks.is_empty() {
            if self.hop > 1 {
                writeln!(f, "--- Related (hop {}) ---", self.hop)?;
            } else {
                writeln!(f, "--- Related ---")?;
            }
            writeln!(f)?;
            for group in &self.related_chunks {
                let best_score = group.chunks.first().map(|c| c.score).unwrap_or(0.0);
                writeln!(
                    f,
                    "{} ({}) [{:.2}]",
                    group.file_path, group.note_type, best_score
                )?;
                for chunk in &group.chunks {
                    let header_display = if chunk.header_path.is_empty() {
                        String::new()
                    } else {
                        format!("[{}]", chunk.header_path)
                    };
                    writeln!(f, "  [{:.2}] {}", chunk.score, header_display)?;
                    if let Some(line) = chunk.content.lines().find(|l| !l.trim().is_empty()) {
                        writeln!(f, "    > {line}")?;
                    }
                }
                writeln!(f)?;
            }
        }

        Ok(())
    }
}

pub fn run(config: &Config, topic: &str, format: &str, depth: usize, limit: usize) -> Result<()> {
    let fmt = OutputFormat::parse(format);

    let results: ContextResults = crate::actions::context::run(config, topic, depth, limit)?;

    if results.hops.is_empty() && fmt == OutputFormat::Text {
        crate::output::warning("No search index found. Run 'temper index' to build it.");
        return Ok(());
    }

    for hop in results.hops {
        let output: ContextOutput = hop.into();
        crate::format::output(&output, fmt);
    }

    Ok(())
}
