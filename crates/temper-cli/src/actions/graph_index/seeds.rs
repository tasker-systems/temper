//! TF-IDF seed phrase extraction.
//!
//! Walks the vault (via `discover_vault` from `index_build`), tokenizes markdown
//! bodies, computes TF-IDF scores, and returns the top N candidate seed phrases.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use temper_core::types::config::GraphIndexConfig;
use temper_llm::types::SeedPhrase;

/// Doc types that live at `{vault}/@me/{context}/{doc_type}/`.
/// Mirrors `graph_build::ENTITY_DOC_TYPES`.
const ENTITY_DOC_TYPES: &[&str] = &["task", "goal", "session", "decision", "concept", "research"];

/// Per-phrase accumulator used while computing TF-IDF across the vault.
#[derive(Debug, Default)]
struct PhraseAggregate {
    doc_frequency: usize,
    aggregate_score: f32,
    /// `(rel_path, tfidf)` for every document in which the phrase appears.
    doc_scores: Vec<(String, f32)>,
}

/// Extract seed phrases from the vault using TF-IDF.
///
/// Cross-document frequency filter: phrase must appear in ≥ `seed_min_doc_frequency`
/// docs. The top `seed_top_n` phrases (by aggregate TF-IDF) are returned.
pub fn extract_seeds(
    vault_root: &Path,
    config: &GraphIndexConfig,
    context_filter: Option<&str>,
) -> Vec<SeedPhrase> {
    let discovered = discover_vault_files(vault_root, context_filter);

    // Collect documents: rel_path -> body text
    let mut documents: Vec<(String, String)> = Vec::new();
    for (path, rel_path) in discovered {
        if let Ok(raw) = fs::read_to_string(&path) {
            let body = strip_frontmatter(&raw);
            if !body.trim().is_empty() {
                documents.push((rel_path, body));
            }
        }
    }

    if documents.is_empty() {
        return Vec::new();
    }

    let total_docs = documents.len();

    // Build term frequency per document and document frequency per term
    let mut doc_term_freqs: Vec<HashMap<String, usize>> = Vec::new();
    let mut term_doc_freq: HashMap<String, usize> = HashMap::new();

    for (_rel_path, body) in &documents {
        let tokens = tokenize(body);
        let mut freq: HashMap<String, usize> = HashMap::new();
        for token in tokens {
            *freq.entry(token).or_insert(0) += 1;
        }
        for term in freq.keys() {
            *term_doc_freq.entry(term.clone()).or_insert(0) += 1;
        }
        doc_term_freqs.push(freq);
    }

    // Compute TF-IDF for n-grams (unigrams and bigrams).
    // Per-phrase aggregate state while building the index.
    let mut phrase_scores: HashMap<String, PhraseAggregate> = HashMap::new();

    for (i, (rel_path, body)) in documents.iter().enumerate() {
        let tokens = tokenize(body);
        let doc_len = tokens.len();
        if doc_len == 0 {
            continue;
        }

        let phrases: Vec<String> = tokens
            .iter()
            .enumerate()
            .flat_map(|(j, t)| {
                let mut ph = vec![t.clone()];
                if j + 1 < tokens.len() {
                    ph.push(format!("{} {}", t, tokens[j + 1]));
                }
                ph
            })
            .collect();

        let mut seen: HashSet<String> = HashSet::new();
        for phrase in phrases {
            if !seen.insert(phrase.clone()) {
                continue;
            }

            let tf = doc_term_freqs[i].get(&phrase).copied().unwrap_or(0);
            if tf == 0 {
                continue;
            }

            let df = *term_doc_freq.get(&phrase).unwrap_or(&1);
            let idf = ((total_docs as f32) / (df as f32)).ln() + 1.0;
            let tfidf = (tf as f32 / doc_len as f32) * idf;

            let entry = phrase_scores.entry(phrase.clone()).or_default();
            entry.doc_frequency += 1;
            entry.aggregate_score += tfidf;
            entry.doc_scores.push((rel_path.clone(), tfidf));
        }
    }

    let min_df = config.seed_min_doc_frequency;

    // Filter by minimum doc frequency; build (agg_score, SeedPhrase) pairs for sorting.
    let mut scored: Vec<(f32, SeedPhrase)> = phrase_scores
        .into_iter()
        .filter(|(_, agg)| agg.doc_frequency >= min_df)
        .map(|(phrase, mut agg)| {
            agg.doc_scores
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let top_docs: Vec<String> =
                agg.doc_scores.into_iter().take(5).map(|(p, _)| p).collect();
            (
                agg.aggregate_score,
                SeedPhrase::new(phrase, agg.doc_frequency, top_docs),
            )
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(config.seed_top_n);

    scored.into_iter().map(|(_, s)| s).collect()
}

/// Walk `{vault_root}/@me/{context}/{doc_type}/*.md` and return
/// `(abs_path, rel_path)` pairs. `rel_path` uses forward slashes and is
/// rooted at `@me` so it matches the shape used by the index manifest.
fn discover_vault_files(vault_root: &Path, context_filter: Option<&str>) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    let owner = "@me";
    let owner_root = vault_root.join(owner);

    let contexts: Vec<(PathBuf, String)> = if let Some(ctx) = context_filter {
        let path = owner_root.join(ctx);
        if path.exists() {
            vec![(path, ctx.to_string())]
        } else {
            Vec::new()
        }
    } else {
        let Ok(entries) = fs::read_dir(&owner_root) else {
            return out;
        };
        entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                let name = p.file_name().and_then(|n| n.to_str())?.to_string();
                if p.is_dir() && !name.starts_with('.') {
                    Some((p, name))
                } else {
                    None
                }
            })
            .collect()
    };

    for (ctx_root, context) in contexts {
        for doc_type in ENTITY_DOC_TYPES {
            let type_dir = ctx_root.join(doc_type);
            let Ok(entries) = fs::read_dir(&type_dir) else {
                continue;
            };
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }
                let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let rel_path = format!("{owner}/{context}/{file_name}");
                out.push((path, rel_path));
            }
        }
    }

    out
}

/// Strip YAML frontmatter from markdown, returning just the body.
fn strip_frontmatter(raw: &str) -> String {
    let mut lines = raw.lines();
    if lines.next() != Some("---") {
        return raw.to_string();
    }
    for line in lines.by_ref() {
        if line == "---" {
            break;
        }
    }
    lines.collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Simple tokenizer: lowercase, strip punctuation, split on whitespace.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize("Hello, world! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
    }

    #[test]
    fn test_strip_frontmatter() {
        let raw = "---\ntemper-id: abc123\n---\n# Hello\n\nBody text.";
        let body = strip_frontmatter(raw);
        assert!(body.starts_with("# Hello"));
        assert!(body.contains("Body text"));
    }

    #[test]
    fn test_strip_frontmatter_no_frontmatter() {
        let raw = "# Just a heading\n\nSome content";
        let body = strip_frontmatter(raw);
        assert!(body.starts_with("# Just a heading"));
    }
}
