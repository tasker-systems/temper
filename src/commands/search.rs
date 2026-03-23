use serde::Serialize;
use std::fmt;
use crate::config::Config;
use crate::embedder::{self, Embedder};
use crate::error::Result;
use crate::format::OutputFormat;
use crate::hnsw::{SearchFilter, SearchIndex};

#[derive(Debug, Serialize)]
struct SearchHitOutput {
    score: f32,
    file_path: String,
    chunk_index: usize,
    note_type: String,
    cluster: Option<String>,
    project: Option<String>,
    content: String,
}

#[derive(Debug, Serialize)]
struct SearchOutput {
    query: String,
    hits: Vec<SearchHitOutput>,
}

impl fmt::Display for SearchHitOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "[{:.2}] {} #chunk:{}", self.score, self.file_path, self.chunk_index)?;

        let mut meta = Vec::new();
        meta.push(format!("type: {}", self.note_type));
        if let Some(ref cluster) = self.cluster {
            meta.push(format!("cluster: {cluster}"));
        }
        if let Some(ref project) = self.project {
            meta.push(format!("project: {project}"));
        }
        writeln!(f, "  {}", meta.join(" | "))?;

        // Render content lines indented with "> ", max 4 lines
        let lines: Vec<&str> = self.content.lines().collect();
        let max_lines = 4;
        let truncated = lines.len() > max_lines;
        for line in lines.iter().take(max_lines) {
            writeln!(f, "  > {line}")?;
        }
        if truncated {
            writeln!(f, "  > ...")?;
        }

        Ok(())
    }
}

impl fmt::Display for SearchOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.hits.is_empty() {
            return writeln!(f, "No results for \"{}\"", self.query);
        }
        for hit in &self.hits {
            write!(f, "{hit}")?;
            writeln!(f)?;
        }
        Ok(())
    }
}

pub fn run(
    config: &Config,
    query: &str,
    format: &str,
    note_type: Option<&str>,
    project: Option<&str>,
    limit: usize,
) -> Result<()> {
    let fmt = OutputFormat::parse(format);

    let state_dir = &config.state_dir;

    let index = match SearchIndex::load(state_dir) {
        Ok(idx) => idx,
        Err(_) => {
            println!("No search index found. Run 'temper index' to build it.");
            return Ok(());
        }
    };

    if index.entry_count() == 0 {
        println!("Index is empty. Run 'temper index' to populate it.");
        return Ok(());
    }

    let mut embedder = Embedder::new(config.model_cache_dir.clone());
    let preprocessed = embedder::preprocess_chunk(query, "");
    let query_vector = embedder.embed(&preprocessed)?;

    let filter = if note_type.is_some() || project.is_some() {
        Some(SearchFilter {
            note_type: note_type.map(String::from),
            cluster: None,
            project: project.map(String::from),
            tags: None,
        })
    } else {
        None
    };

    let hits = index.search(&query_vector, limit, filter.as_ref());

    let hit_outputs: Vec<SearchHitOutput> = hits
        .into_iter()
        .map(|h| SearchHitOutput {
            score: h.score,
            file_path: h.entry.file_path,
            chunk_index: h.entry.chunk_index,
            note_type: h.entry.metadata.note_type,
            cluster: h.entry.metadata.cluster,
            project: h.entry.metadata.project,
            content: h.entry.content,
        })
        .collect();

    let output = SearchOutput {
        query: query.to_string(),
        hits: hit_outputs,
    };

    crate::format::output(&output, fmt);
    Ok(())
}
