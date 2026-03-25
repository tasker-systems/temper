use crate::actions::types::{SearchHit, SearchResults};
use crate::config::Config;
use crate::error::Result;
use crate::format::OutputFormat;
use serde::Serialize;
use std::fmt;

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

impl From<SearchHit> for SearchHitOutput {
    fn from(hit: SearchHit) -> Self {
        SearchHitOutput {
            score: hit.score,
            file_path: hit.file_path,
            chunk_index: hit.chunk_index,
            note_type: hit.note_type,
            cluster: hit.cluster,
            project: hit.project,
            content: hit.content,
        }
    }
}

impl From<SearchResults> for SearchOutput {
    fn from(results: SearchResults) -> Self {
        SearchOutput {
            query: results.query,
            hits: results
                .hits
                .into_iter()
                .map(SearchHitOutput::from)
                .collect(),
        }
    }
}

impl fmt::Display for SearchHitOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "[{:.2}] {} #chunk:{}",
            self.score, self.file_path, self.chunk_index
        )?;

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

    let results = crate::actions::search::run(config, query, note_type, project, limit)?;

    if results.hits.is_empty() && fmt == OutputFormat::Text {
        crate::output::warning("No results found. Run 'temper index' if you haven't indexed yet.");
    }

    let output: SearchOutput = results.into();
    crate::format::output(&output, fmt);
    Ok(())
}
