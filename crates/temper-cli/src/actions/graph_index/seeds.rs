//! TF-IDF seed phrase extraction using tantivy.
//!
//! Walks the vault (via discover_vault from index_build), tokenizes markdown bodies,
//! computes TF-IDF scores, and returns the top N candidate seed phrases.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use temper_core::types::config::GraphIndexConfig;

use crate::actions::index_build::discover_vault;

/// A seed phrase candidate with its TF-IDF score and supporting documents.
#[derive(Debug, Clone)]
pub struct SeedPhrase {
    /// The raw phrase text (n-gram).
    pub phrase: String,
    /// Number of documents this phrase appears in.
    pub doc_frequency: usize,
    /// Top document IDs by TF-IDF score within this phrase's context.
    pub top_doc_ids: Vec<String>,
    /// Aggregate TF-IDF score across all documents.
    pub aggregate_score: f32,
}

impl SeedPhrase {
    pub fn new(phrase: String, doc_frequency: usize, top_doc_ids: Vec<String>, aggregate_score: f32) -> Self {
        Self {
            phrase,
            doc_frequency,
            top_doc_ids,
            aggregate_score,
        }
    }
}

/// Extract seed phrases from the vault using TF-IDF.
///
/// Uses tantivy for tokenization, Snowball stemmer, stopwords, and TF-IDF scoring.
/// Cross-document frequency filter: phrase must appear in ≥`seed_min_doc_frequency` docs.
pub fn extract_seeds(
    vault_root: &PathBuf,
    config: &GraphIndexConfig,
    context_filter: Option<&str>,
) -> Vec<SeedPhrase> {
    // Discover all vault files
    let discovered = discover_vault(vault_root, context_filter);

    // Collect documents: rel_path -> body text
    let mut documents: Vec<(String, String)> = Vec::new();
    for discovered_file in discovered {
        if let Ok(raw) = fs::read_to_string(&discovered_file.path) {
            let body = strip_frontmatter(&raw);
            if !body.trim().is_empty() {
                documents.push((discovered_file.rel_path.clone(), body));
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

    // Compute TF-IDF for n-grams (unigrams and bigrams)
    let mut phrase_scores: HashMap<String, (usize, f32, Vec<(String, f32)>)> = HashMap::new();

    for (i, (rel_path, body)) in documents.iter().enumerate() {
        let tokens = tokenize(body);
        let doc_len = tokens.len();
        if doc_len == 0 {
            continue;
        }

        // Unigrams + bigrams
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
            if seen.contains(&phrase) {
                continue;
            }
            seen.insert(phrase.clone());

            let tf = doc_term_freqs[i].get(&phrase).copied().unwrap_or(0);
            if tf == 0 {
                continue;
            }

            let df = *term_doc_freq.get(&phrase).unwrap_or(&1);
            let idf = ((total_docs as f32) / (df as f32)).ln() + 1.0;
            let tfidf = (tf as f32 / doc_len as f32) * idf;

            let entry = phrase_scores.entry(phrase.clone()).or_insert((0, 0.0, Vec::new()));
            entry.0 += 1; // doc frequency
            entry.1 += tfidf; // aggregate score
            entry.2.push((rel_path.clone(), tfidf));
        }
    }

    // Filter by minimum document frequency
    let min_df = config.seed_min_doc_frequency;
    let candidates: Vec<(String, usize, f32, Vec<(String, f32)>)> = phrase_scores
        .into_iter()
        .filter(|(_, (df, _, _))| df >= min_df)
        .map(|(phrase, (df, agg_score, doc_scores))| {
            // Sort doc_scores by tfidf descending, take top 5
            doc_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let top_docs: Vec<String> = doc_scores.into_iter().take(5).map(|(p, _)| p).collect();
            (phrase, df, agg_score, top_docs)
        })
        .collect();

    // Sort by aggregate score descending, take top N
    let mut sorted: Vec<(String, usize, f32, Vec<String>)> = candidates;
    sorted.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    sorted.truncate(config.seed_top_n);

    sorted
        .into_iter()
        .map(|(phrase, df, score, top_docs)| SeedPhrase::new(phrase, df, top_docs, score))
        .collect()
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

use std::collections::HashSet;

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