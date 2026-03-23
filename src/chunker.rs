// Markdown-aware chunking

use serde::{Deserialize, Serialize};

pub const MAX_CHUNK_SIZE: usize = 1000;
pub const MIN_CHUNK_SIZE: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMeta {
    pub note_type: String,
    pub cluster: Option<String>,
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub title: String,
}

impl Default for ChunkMeta {
    fn default() -> Self {
        ChunkMeta {
            note_type: "unknown".to_string(),
            cluster: None,
            project: None,
            tags: Vec::new(),
            title: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    pub content: String,
    pub file_path: String,
    pub chunk_index: usize,
    pub header_path: String,
    pub metadata: ChunkMeta,
}

/// Parse YAML frontmatter from markdown content.
/// Returns (yaml_str, body) where yaml_str is the raw YAML between the `---` fences,
/// and body is the rest of the document after the closing fence.
fn split_frontmatter(content: &str) -> (Option<String>, &str) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, content);
    }
    let after_open = &trimmed[3..];
    // Allow optional whitespace/newline after opening ---
    let after_open = after_open
        .strip_prefix("\r\n")
        .or_else(|| after_open.strip_prefix('\n'))
        .unwrap_or(after_open);

    // Find the closing ---
    if let Some(close_pos) = after_open.find("\n---") {
        let yaml_str = &after_open[..close_pos];
        let body_start = close_pos + 4; // skip \n---
        let body = &after_open[body_start..];
        // Skip optional newline after closing ---
        let body = body
            .strip_prefix("\r\n")
            .or_else(|| body.strip_prefix('\n'))
            .unwrap_or(body);
        (Some(yaml_str.to_string()), body)
    } else {
        (None, content)
    }
}

/// Extract ChunkMeta and formatted string from YAML frontmatter.
fn parse_frontmatter_meta(yaml_str: &str) -> (ChunkMeta, String) {
    let value: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap_or(serde_yaml::Value::Null);
    let map = match &value {
        serde_yaml::Value::Mapping(m) => Some(m),
        _ => None,
    };

    let get_str = |key: &str| -> Option<String> {
        map?.get(key)?.as_str().map(String::from)
    };

    let note_type = get_str("type").unwrap_or_else(|| "unknown".to_string());
    let title = get_str("title").unwrap_or_default();
    let cluster = get_str("cluster");
    let project = get_str("project");

    let tags: Vec<String> = map
        .and_then(|m| m.get("tags"))
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let meta = ChunkMeta {
        note_type,
        title: title.clone(),
        cluster,
        project,
        tags: tags.clone(),
    };

    // Build formatted frontmatter string
    let mut parts = Vec::new();
    if !meta.title.is_empty() {
        parts.push(format!("title: {}", meta.title));
    }
    parts.push(format!("type: {}", meta.note_type));
    if !tags.is_empty() {
        parts.push(format!("tags: {}", tags.join(", ")));
    }
    if let Some(ref cluster) = meta.cluster {
        parts.push(format!("cluster: {cluster}"));
    }
    if let Some(ref project) = meta.project {
        parts.push(format!("project: {project}"));
    }
    // Include any remaining scalar fields
    if let Some(m) = map {
        for (k, v) in m {
            if let (Some(k_str), Some(v_str)) = (k.as_str(), v.as_str()) {
                if !matches!(k_str, "type" | "title" | "tags" | "cluster" | "project") {
                    parts.push(format!("{k_str}: {v_str}"));
                }
            }
        }
    }

    let formatted = parts.join(" | ");
    (meta, formatted)
}

/// A parsed section from the body of a markdown document.
#[derive(Debug)]
struct Section {
    /// The full breadcrumb path of headers leading to this section, e.g. "## Core > ### Sub"
    header_path: String,
    /// The text content of this section (excluding the header line itself)
    content: String,
}

/// Parse markdown body into sections keyed by their header breadcrumbs.
/// Sections are split at every line that starts with `#`.
fn parse_sections(body: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    // Track current ancestor headers at each depth level
    // depth 1 = #, depth 2 = ##, depth 3 = ###, etc.
    let mut header_stack: Vec<(usize, String)> = Vec::new(); // (depth, text)
    let mut current_content = String::new();
    let mut current_path = String::new();
    let mut has_current = false;

    for line in body.lines() {
        if let Some(header_depth) = header_level(line) {
            // Save previous section if it has content
            if has_current && !current_content.trim().is_empty() {
                sections.push(Section {
                    header_path: current_path.clone(),
                    content: current_content.trim().to_string(),
                });
            } else if has_current && current_content.trim().is_empty() {
                // Drop empty sections silently
            }

            let header_text = line[header_depth..].trim().to_string();

            // Pop stack entries at same or deeper depth
            while let Some((d, _)) = header_stack.last() {
                if *d >= header_depth {
                    header_stack.pop();
                } else {
                    break;
                }
            }
            header_stack.push((header_depth, header_text.clone()));

            // Build breadcrumb path from stack
            current_path = header_stack
                .iter()
                .map(|(d, t)| format!("{} {t}", "#".repeat(*d)))
                .collect::<Vec<_>>()
                .join(" > ");

            current_content = String::new();
            has_current = true;
        } else {
            current_content.push_str(line);
            current_content.push('\n');
        }
    }

    // Don't forget the last section
    if has_current && !current_content.trim().is_empty() {
        sections.push(Section {
            header_path: current_path,
            content: current_content.trim().to_string(),
        });
    }

    // Also capture any leading content before the first header
    // (re-scan: find leading lines before first #)
    let leading = leading_content(body);
    if !leading.trim().is_empty() {
        // Insert at front
        sections.insert(
            0,
            Section {
                header_path: String::new(),
                content: leading.trim().to_string(),
            },
        );
    }

    sections
}

/// Extract content before the first header line.
fn leading_content(body: &str) -> String {
    let mut lines = Vec::new();
    for line in body.lines() {
        if header_level(line).is_some() {
            break;
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// Return the header depth (number of leading `#`) if this line is a header, else None.
fn header_level(line: &str) -> Option<usize> {
    if !line.starts_with('#') {
        return None;
    }
    let depth = line.chars().take_while(|&c| c == '#').count();
    // Must be followed by a space to be a header
    if line.len() > depth && line.as_bytes()[depth] == b' ' {
        Some(depth)
    } else {
        None
    }
}

/// Split a large section into sub-chunks at paragraph boundaries.
fn split_on_paragraphs(content: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();

    for paragraph in content.split("\n\n") {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        if current.is_empty() {
            current.push_str(paragraph);
        } else if current.len() + paragraph.len() + 2 > MAX_CHUNK_SIZE {
            result.push(current.trim().to_string());
            current = paragraph.to_string();
        } else {
            current.push_str("\n\n");
            current.push_str(paragraph);
        }
    }
    if !current.trim().is_empty() {
        result.push(current.trim().to_string());
    }
    result
}

/// Main entry point: chunk a markdown document into semantically coherent pieces.
pub fn chunk_document(file_path: &str, content: &str) -> Vec<Chunk> {
    if content.trim().is_empty() {
        return Vec::new();
    }

    let (frontmatter_yaml, body) = split_frontmatter(content);

    let (meta, fm_formatted) = frontmatter_yaml
        .as_deref()
        .map(parse_frontmatter_meta)
        .unwrap_or_else(|| (ChunkMeta::default(), String::new()));

    let mut raw_chunks: Vec<(String, String)> = Vec::new(); // (header_path, content)

    // Chunk 0: frontmatter
    if !fm_formatted.is_empty() {
        raw_chunks.push((String::new(), fm_formatted));
    }

    // Parse body sections
    let sections = parse_sections(body);

    for section in sections {
        if section.content.len() > MAX_CHUNK_SIZE {
            // Split on paragraph boundaries
            let sub = split_on_paragraphs(&section.content);
            for part in sub {
                raw_chunks.push((section.header_path.clone(), part));
            }
        } else {
            raw_chunks.push((section.header_path.clone(), section.content));
        }
    }

    // Merge tiny chunks with next sibling (same header_path prefix)
    let raw_chunks = merge_small_chunks(raw_chunks);

    // Assign indices and build Chunk structs
    raw_chunks
        .into_iter()
        .enumerate()
        .map(|(i, (header_path, content))| Chunk {
            id: format!("{}#chunk:{}", file_path, i),
            content,
            file_path: file_path.to_string(),
            chunk_index: i,
            header_path,
            metadata: meta.clone(),
        })
        .collect()
}

/// Merge chunks that are smaller than MIN_CHUNK_SIZE with adjacent chunks sharing
/// the same header path.
fn merge_small_chunks(chunks: Vec<(String, String)>) -> Vec<(String, String)> {
    if chunks.is_empty() {
        return chunks;
    }

    let mut result: Vec<(String, String)> = Vec::new();

    for (path, content) in chunks {
        let should_merge = if let Some(last) = result.last() {
            // Don't merge frontmatter chunk (first chunk with empty path and type: marker)
            let last_is_frontmatter = result.len() == 1 && last.0.is_empty() && last.1.contains("type:");
            !last_is_frontmatter && last.1.len() < MIN_CHUNK_SIZE && last.0 == path
        } else {
            false
        };

        if should_merge {
            let last = result.last_mut().unwrap();
            last.1.push_str("\n\n");
            last.1.push_str(&content);
        } else {
            result.push((path, content));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_level() {
        assert_eq!(header_level("# Title"), Some(1));
        assert_eq!(header_level("## Section"), Some(2));
        assert_eq!(header_level("### Sub"), Some(3));
        assert_eq!(header_level("not a header"), None);
        assert_eq!(header_level("#no-space"), None);
    }

    #[test]
    fn test_split_frontmatter_basic() {
        let content = "---\ntype: concept\ntitle: Test\n---\n\n# Body\n";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_some());
        assert!(fm.unwrap().contains("type: concept"));
        assert!(body.contains("# Body"));
    }

    #[test]
    fn test_split_frontmatter_none() {
        let content = "# Just a header\n\nSome text.\n";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_none());
        assert!(body.contains("# Just a header"));
    }
}
