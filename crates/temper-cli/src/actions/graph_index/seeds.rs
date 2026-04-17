//! Field-weighted TF-IDF seed phrase extraction.
//!
//! Walks the vault (via `discover_vault` from `index_build`), parses each
//! markdown file into four structural fields (frontmatter `title`, H1 heading
//! text, H2/H3 heading text, body prose), tokenizes each field independently,
//! and computes TF-IDF over a weighted token-frequency map. Higher-structure
//! fields (title, H1) amplify their terms' counts before TF/IDF math runs.
//!
//! ISO date tokens (`YYYY-MM-DD`, `YYYY/MM/DD`, bare `YYYY`) are stripped
//! from every field before tokenization so that slug-style titles like
//! `2026-04-16-wire-and-fix-...` do not boost year/month/day numerics.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use regex::Regex;
use tantivy::tokenizer::{
    Language, LowerCaser, SimpleTokenizer, Stemmer, StopWordFilter, TextAnalyzer,
};
use temper_core::frontmatter::Frontmatter;
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

/// Structural slices of a vault markdown file used for field-weighted TF-IDF.
///
/// Each field holds the raw text that should be tokenized independently and
/// weighted before merging into the per-doc term-frequency map. Heading text
/// is excluded from `body` so it is not double-counted.
#[derive(Debug, Default, Clone)]
struct DocFields {
    title: String,
    h1: String,
    h2_h3: String,
    body: String,
}

/// Extract seed phrases from the vault using field-weighted TF-IDF.
///
/// Cross-document frequency filter: phrase must appear in ≥ `seed_min_doc_frequency`
/// docs. The top `seed_top_n` phrases (by aggregate TF-IDF) are returned.
pub fn extract_seeds(
    vault_root: &Path,
    config: &GraphIndexConfig,
    context_filter: Option<&str>,
) -> Vec<SeedPhrase> {
    let discovered = discover_vault_files(vault_root, context_filter);

    // Collect documents: rel_path -> parsed fields
    let mut documents: Vec<(String, DocFields)> = Vec::new();
    for (path, rel_path) in discovered {
        if let Ok(raw) = fs::read_to_string(&path) {
            let fields = parse_doc_fields(&raw);
            if !fields_empty(&fields) {
                documents.push((rel_path, fields));
            }
        }
    }

    if documents.is_empty() {
        return Vec::new();
    }

    let total_docs = documents.len();

    // Build the tokenizer pipeline once and reuse it for every document.
    // `token_stream` takes `&mut self`, so the analyzer is threaded through by
    // mutable reference rather than cloned per call. The context name, when
    // supplied, is installed as an extra stopword for this run.
    let mut analyzer = build_analyzer(context_filter);

    // Build weighted term frequency per document and document frequency per term.
    // `doc_weighted_tf[i][term] = weighted count of term across all fields of doc i`
    // `doc_weighted_len[i] = sum over fields of (field_weight * field_token_count)`
    // `term_doc_freq[term] = number of docs in which term appears in ANY field`
    let mut doc_weighted_tf: Vec<HashMap<String, f32>> = Vec::new();
    let mut doc_weighted_len: Vec<f32> = Vec::new();
    let mut term_doc_freq: HashMap<String, usize> = HashMap::new();

    for (_rel_path, fields) in &documents {
        let title_tokens = tokenize(&mut analyzer, &strip_dates(&fields.title));
        let h1_tokens = tokenize(&mut analyzer, &strip_dates(&fields.h1));
        let h23_tokens = tokenize(&mut analyzer, &strip_dates(&fields.h2_h3));
        let body_tokens = tokenize(&mut analyzer, &strip_dates(&fields.body));

        let field_inputs: [(&[String], f32); 4] = [
            (&title_tokens, config.seed_title_weight),
            (&h1_tokens, config.seed_h1_weight),
            (&h23_tokens, config.seed_h2_h3_weight),
            (&body_tokens, config.seed_body_weight),
        ];

        let mut weighted_tf: HashMap<String, f32> = HashMap::new();
        let mut weighted_len: f32 = 0.0;

        for (tokens, weight) in field_inputs {
            if tokens.is_empty() || weight == 0.0 {
                continue;
            }
            // Unigrams
            for tok in tokens {
                *weighted_tf.entry(tok.clone()).or_insert(0.0) += weight;
                weighted_len += weight;
            }
            // Bigrams (stay within a single field — no cross-field bigrams)
            for pair in tokens.windows(2) {
                let bigram = format!("{} {}", pair[0], pair[1]);
                *weighted_tf.entry(bigram).or_insert(0.0) += weight;
                weighted_len += weight;
            }
        }

        // Document frequency: count this doc once per distinct term across all fields.
        for term in weighted_tf.keys() {
            *term_doc_freq.entry(term.clone()).or_insert(0) += 1;
        }

        doc_weighted_tf.push(weighted_tf);
        doc_weighted_len.push(weighted_len);
    }

    // Compute TF-IDF per (doc, term) over the weighted frequency map.
    let mut phrase_scores: HashMap<String, PhraseAggregate> = HashMap::new();

    for (i, (rel_path, _fields)) in documents.iter().enumerate() {
        let weighted_len = doc_weighted_len[i];
        if weighted_len <= 0.0 {
            continue;
        }

        let mut seen: HashSet<String> = HashSet::new();
        for (phrase, weighted_tf) in &doc_weighted_tf[i] {
            if !seen.insert(phrase.clone()) {
                continue;
            }

            let df = *term_doc_freq.get(phrase).unwrap_or(&1);
            let idf = ((total_docs as f32) / (df as f32)).ln() + 1.0;
            let tfidf = (weighted_tf / weighted_len) * idf;

            let entry = phrase_scores.entry(phrase.clone()).or_default();
            entry.doc_frequency += 1;
            entry.aggregate_score += tfidf;
            entry.doc_scores.push((rel_path.clone(), tfidf));
        }
    }

    let min_df = config.seed_min_doc_frequency;
    // Max-df filter: drop phrases appearing in more than this fraction of
    // docs. Catches "gravity well" terms (e.g. repeated slug tokens) whose
    // IDF can't overcome title-weighting.
    let max_df_ratio = config.seed_max_doc_frequency_ratio.max(0.0);
    let max_df = ((total_docs as f32) * max_df_ratio).floor() as usize;

    // Filter by min and max doc frequency; build (agg_score, SeedPhrase) pairs for sorting.
    let mut scored: Vec<(f32, SeedPhrase)> = phrase_scores
        .into_iter()
        .filter(|(_, agg)| agg.doc_frequency >= min_df && agg.doc_frequency <= max_df)
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
                let rel_path = format!("{owner}/{context}/{doc_type}/{file_name}");
                out.push((path, rel_path));
            }
        }
    }

    out
}

/// Parse a vault markdown file into its four structural fields.
///
/// Frontmatter is parsed via [`Frontmatter::try_from`] so that the `title`
/// field comes from the canonical YAML parser rather than a hand-rolled split.
/// Files whose frontmatter fails to parse (missing `temper-type`, malformed
/// YAML) fall back to treating the whole file as body — this keeps seed
/// extraction robust against mid-edit files.
///
/// Body parsing uses pulldown-cmark's event stream. Heading text is routed
/// to `h1` or `h2_h3` by level (H4+ is treated as body, per SG-5). All
/// non-heading `Event::Text` goes to `body`.
fn parse_doc_fields(raw: &str) -> DocFields {
    let (title, body_src) = match Frontmatter::try_from(raw) {
        Ok(fm) => {
            let title = fm
                .value()
                .as_mapping()
                .and_then(|m| m.get(serde_yaml::Value::String("title".to_string())))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            (title, fm.body().to_string())
        }
        Err(_) => (String::new(), raw.to_string()),
    };

    let mut fields = DocFields {
        title,
        ..DocFields::default()
    };

    let parser = Parser::new(&body_src);
    let mut in_heading: Option<HeadingLevel> = None;

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = None;
            }
            Event::Text(text) => match in_heading {
                Some(HeadingLevel::H1) => push_with_sep(&mut fields.h1, &text),
                Some(HeadingLevel::H2 | HeadingLevel::H3) => {
                    push_with_sep(&mut fields.h2_h3, &text)
                }
                // H4+ and body prose both land in `body`.
                _ => push_with_sep(&mut fields.body, &text),
            },
            Event::Code(text) => {
                // Inline code inside a heading still counts as heading text.
                match in_heading {
                    Some(HeadingLevel::H1) => push_with_sep(&mut fields.h1, &text),
                    Some(HeadingLevel::H2 | HeadingLevel::H3) => {
                        push_with_sep(&mut fields.h2_h3, &text)
                    }
                    _ => push_with_sep(&mut fields.body, &text),
                }
            }
            _ => {}
        }
    }

    fields
}

fn push_with_sep(buf: &mut String, text: &str) {
    if !buf.is_empty() {
        buf.push('\n');
    }
    buf.push_str(text);
}

fn fields_empty(f: &DocFields) -> bool {
    f.title.trim().is_empty()
        && f.h1.trim().is_empty()
        && f.h2_h3.trim().is_empty()
        && f.body.trim().is_empty()
}

/// Regex matching ISO-style date tokens we want to strip before tokenization.
///
/// Matches:
/// - `YYYY` (year only, 1900–2099)
/// - `YYYY-MM`, `YYYY/MM`
/// - `YYYY-MM-DD`, `YYYY/MM/DD`
///
/// Does NOT match four-digit numbers outside the 1900–2099 window
/// (`11434` port, `3500` context, `1024`) or longer numeric runs.
fn date_re() -> &'static Regex {
    static DATE_RE: OnceLock<Regex> = OnceLock::new();
    DATE_RE.get_or_init(|| {
        Regex::new(r"\b(?:19|20)\d{2}(?:[-/]\d{1,2}){0,2}\b").expect("date regex is valid")
    })
}

/// Replace ISO date tokens in `text` with a single space so the tokenizer
/// never sees them. Used per-field before feeding the analyzer.
fn strip_dates(text: &str) -> String {
    date_re().replace_all(text, " ").to_string()
}

/// Extended stopword list layered on top of tantivy's built-in English filter.
///
/// Tantivy's default English list is Lucene's 33-word minimalist set; it lets
/// common auxiliary/discourse words through ("from", "has", "should", etc.)
/// which survive TF-IDF weighting when they appear in titles. Listed here in
/// their unstemmed surface form — the stopword filter runs before stemming.
const EXTENDED_STOPWORDS: &[&str] = &[
    "from", "has", "have", "had", "having", "can", "could", "may", "might", "must", "shall",
    "should", "would", "do", "does", "did", "done", "been", "being", "am", "we", "us", "our",
    "ours", "you", "your", "yours", "i", "me", "my", "mine", "he", "him", "his", "she", "her",
    "hers", "them", "theirs", "after", "before", "when", "where", "while", "which", "who", "whom",
    "whose", "why", "how", "what", "also", "just", "only", "so", "than", "too", "very", "some",
    "any", "each", "every", "more", "most", "other", "others", "over", "under", "about", "above",
    "below", "between", "during", "through", "up", "down", "out", "off", "again", "further",
    "here", "there", "now", "all", "both", "few", "many", "same", "own",
];

/// Build the shared tokenizer pipeline:
/// `SimpleTokenizer` → `LowerCaser` → default English stopwords → extended
/// stopwords → (optional) context-name stopword → English Snowball stemmer.
///
/// Stopword filtering runs before stemming, so each stopword list must contain
/// raw surface forms (e.g. `"has"`, not `"ha"`). The context name, when
/// supplied, is lowercased and added as a one-off stopword so that every doc
/// in an active context doesn't get that context's name as a top seed (the
/// gravity-well case).
fn build_analyzer(context_filter: Option<&str>) -> TextAnalyzer {
    let mut builder = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .filter(StopWordFilter::new(Language::English).expect("English stopwords available"))
        .filter(StopWordFilter::remove(
            EXTENDED_STOPWORDS.iter().map(|&w| w.to_string()),
        ))
        .dynamic();
    if let Some(ctx) = context_filter {
        builder = builder
            .filter_dynamic(StopWordFilter::remove(vec![ctx.to_lowercase()]))
            .dynamic();
    }
    builder
        .filter_dynamic(Stemmer::new(Language::English))
        .build()
}

/// Tokenize `text` using the supplied analyzer. Returns stemmed tokens with
/// stopwords removed. The analyzer is passed by `&mut` because
/// `TextAnalyzer::token_stream` mutates internal buffer state.
fn tokenize(analyzer: &mut TextAnalyzer, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stream = analyzer.token_stream(text);
    while let Some(tok) = stream.next() {
        out.push(tok.text.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_filters_stopwords_and_stems() {
        let mut analyzer = build_analyzer(None);
        let tokens = tokenize(&mut analyzer, "Hello, world! This is a test.");
        // Content words survive lowercasing + stemming (these stem to themselves).
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // Stopwords are removed by the English stopword filter.
        assert!(!tokens.contains(&"this".to_string()));
        assert!(!tokens.contains(&"is".to_string()));
        assert!(!tokens.contains(&"a".to_string()));

        // Extended stopwords layered after tantivy's default English list.
        let probe = "we should know from where it has gone when it can";
        let probe_tokens = tokenize(&mut analyzer, probe);
        for dropped in ["from", "has", "should", "can", "when", "where"] {
            assert!(
                !probe_tokens.contains(&dropped.to_string()),
                "extended stopword '{dropped}' leaked through: {probe_tokens:?}"
            );
        }
    }

    #[test]
    fn test_tokenize_stems_inflected_forms() {
        let mut analyzer = build_analyzer(None);
        let tokens = tokenize(&mut analyzer, "testing running indexes");
        // Snowball stems: testing → test, running → run, indexes → index.
        assert!(tokens.contains(&"test".to_string()));
        assert!(tokens.contains(&"run".to_string()));
        assert!(tokens.contains(&"index".to_string()));
    }

    #[test]
    fn test_parse_doc_fields_splits_title_h1_h23_body() {
        let raw = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Graph Indexing Pipeline
slug: graph-indexing
---

# Top Level Heading

Some introductory prose here.

## Second Level Heading

More body prose.

### Third Level Heading

Body three.

#### Fourth Level Ignored

Extra body text.
"#;
        let fields = parse_doc_fields(raw);

        assert_eq!(fields.title, "Graph Indexing Pipeline");
        assert!(
            fields.h1.contains("Top Level Heading"),
            "h1 missing: {:?}",
            fields.h1
        );
        assert!(
            fields.h2_h3.contains("Second Level Heading"),
            "h2_h3 missing H2: {:?}",
            fields.h2_h3
        );
        assert!(
            fields.h2_h3.contains("Third Level Heading"),
            "h2_h3 missing H3: {:?}",
            fields.h2_h3
        );
        // H4 heading text routes to body (we do not boost H4+).
        assert!(
            fields.body.contains("Fourth Level Ignored"),
            "body missing H4 heading text: {:?}",
            fields.body
        );
        assert!(
            fields.body.contains("introductory prose"),
            "body missing prose: {:?}",
            fields.body
        );
        assert!(
            fields.body.contains("More body prose"),
            "body missing prose after H2: {:?}",
            fields.body
        );
        // Heading text must not be double-counted in body.
        assert!(
            !fields.body.contains("Top Level Heading"),
            "body should not contain H1 text: {:?}",
            fields.body
        );
        assert!(
            !fields.body.contains("Second Level Heading"),
            "body should not contain H2 text: {:?}",
            fields.body
        );
    }

    #[test]
    fn test_parse_doc_fields_no_frontmatter_routes_to_body() {
        let raw = "# Just a heading\n\nSome content with no frontmatter.\n";
        let fields = parse_doc_fields(raw);

        assert_eq!(fields.title, "");
        // Without valid frontmatter the whole file is body_src, so headings
        // still get routed via pulldown-cmark.
        assert!(fields.h1.contains("Just a heading"));
        assert!(fields.body.contains("Some content"));
    }

    #[test]
    fn test_strip_dates_removes_iso_dates() {
        let input = "2026-04-16 the ship 11434 port 2026 graph-index 2024/11/01 and 1899";
        let stripped = strip_dates(input);
        // ISO dates gone.
        assert!(!stripped.contains("2026-04-16"), "got: {stripped}");
        assert!(!stripped.contains("2024/11/01"), "got: {stripped}");
        // Bare year within 1900-2099 also gone.
        assert!(
            !stripped.split_whitespace().any(|w| w == "2026"),
            "got: {stripped}"
        );
        // Five-digit numbers untouched — port numbers survive.
        assert!(stripped.contains("11434"), "got: {stripped}");
        // Hyphenated words with non-date slugs survive.
        assert!(stripped.contains("graph-index"), "got: {stripped}");
        // Years outside the 1900–2099 window survive.
        assert!(stripped.contains("1899"), "got: {stripped}");
    }

    #[test]
    fn test_strip_dates_preserves_large_numbers() {
        // Cases that would trip up a naive `\b\d{4}\b` regex.
        assert_eq!(strip_dates("port 11434"), "port 11434");
        assert_eq!(strip_dates("model 397B"), "model 397B");
        assert_eq!(strip_dates("ctx 3500"), "ctx 3500");
    }

    #[test]
    fn test_extract_seeds_title_boost_surfaces_title_terms() {
        // Two docs: one with a highly distinctive term in the title only,
        // another with filler prose. With field-weighted TF-IDF the title
        // term becomes a seed even though it only appears once in the doc.
        let tmp = tempfile::tempdir().unwrap();
        let task_dir = tmp.path().join("@me").join("temper").join("task");
        fs::create_dir_all(&task_dir).unwrap();

        let doc_a = r#"---
temper-id: "01900000-0000-7000-8000-000000000001"
temper-type: task
temper-context: temper
temper-created: "2026-01-01T00:00:00Z"
temper-owner: "@me"
title: "Graph Indexing Pipeline"
temper-stage: backlog
slug: graph-indexing-pipeline
---

We will build a pipeline for graph indexing. The pipeline has many stages.
"#;
        let doc_b = r#"---
temper-id: "01900000-0000-7000-8000-000000000002"
temper-type: task
temper-context: temper
temper-created: "2026-01-01T00:00:00Z"
temper-owner: "@me"
title: "Pipeline Notes"
temper-stage: backlog
slug: pipeline-notes
---

The pipeline runs nightly. The pipeline emits logs. The pipeline has stages.
"#;
        fs::write(task_dir.join("a.md"), doc_a).unwrap();
        fs::write(task_dir.join("b.md"), doc_b).unwrap();

        // Low min_df=1 so single-doc seeds still surface for the assertion.
        // Disable max-df filter: this fixture has only 2 docs and
        // "pipeline" (stem "pipelin") appears in both — the production 0.5
        // default would drop it as a gravity well. This test exercises the
        // title-boost mechanic, not the max-df filter.
        let cfg = GraphIndexConfig {
            seed_min_doc_frequency: 1,
            seed_top_n: 100,
            seed_max_doc_frequency_ratio: 1.1,
            ..GraphIndexConfig::default()
        };

        let seeds = extract_seeds(tmp.path(), &cfg, Some("temper"));
        assert!(!seeds.is_empty(), "expected seeds but got none");

        let phrases: Vec<&str> = seeds.iter().map(|s| s.phrase.as_str()).collect();

        // "pipeline" stems to "pipelin" with the Snowball English stemmer —
        // both docs contain it, so the stem should be among the seeds.
        assert!(
            phrases.contains(&"pipelin"),
            "expected 'pipelin' in seeds, got: {phrases:?}"
        );

        // "graph" only appears in doc A and only in the title + one body mention,
        // but the title boost makes it a prominent seed. At minimum it must
        // appear in the top-N — this asserts title-only weighting works.
        assert!(
            phrases.contains(&"graph"),
            "expected 'graph' (title-weighted) in seeds, got: {phrases:?}"
        );
    }

    #[test]
    fn test_extract_seeds_filters_gravity_wells() {
        // 4 docs in an "alpha" context. "ubiquitous" appears in all 4
        // (100% > 50%) and should get dropped as a gravity well.
        // "alpha" appears in every doc's prose — the context-name auto-stopword
        // should strip it. "rarephrase" appears in only doc A.
        let tmp = tempfile::tempdir().unwrap();
        let task_dir = tmp.path().join("@me").join("alpha").join("task");
        fs::create_dir_all(&task_dir).unwrap();

        let make_doc = |id: &str, slug: &str, title: &str, body: &str| -> String {
            format!(
                "---\n\
temper-id: \"01900000-0000-7000-8000-{id}\"\n\
temper-type: task\n\
temper-context: alpha\n\
temper-created: \"2026-01-01T00:00:00Z\"\n\
temper-owner: \"@me\"\n\
title: \"{title}\"\n\
temper-stage: backlog\n\
slug: {slug}\n\
---\n\
\n\
{body}\n"
            )
        };

        fs::write(
            task_dir.join("a.md"),
            make_doc(
                "000000000001",
                "doc-a",
                "Doc A",
                "Ubiquitous alpha content here. Rarephrase surfaces only once in the alpha corpus.",
            ),
        )
        .unwrap();
        fs::write(
            task_dir.join("b.md"),
            make_doc(
                "000000000002",
                "doc-b",
                "Doc B",
                "Ubiquitous alpha content again. Nothing special here in alpha.",
            ),
        )
        .unwrap();
        fs::write(
            task_dir.join("c.md"),
            make_doc(
                "000000000003",
                "doc-c",
                "Doc C",
                "Ubiquitous alpha words. The alpha world keeps spinning.",
            ),
        )
        .unwrap();
        fs::write(
            task_dir.join("d.md"),
            make_doc(
                "000000000004",
                "doc-d",
                "Doc D",
                "Ubiquitous alpha mention. Alpha work continues daily.",
            ),
        )
        .unwrap();

        let cfg = GraphIndexConfig {
            seed_min_doc_frequency: 1,
            seed_top_n: 100,
            // Default 0.5 max-df: any term in >2 of 4 docs gets dropped.
            ..GraphIndexConfig::default()
        };

        let seeds = extract_seeds(tmp.path(), &cfg, Some("alpha"));
        let phrases: Vec<&str> = seeds.iter().map(|s| s.phrase.as_str()).collect();

        // "ubiquitous" appears in all 4 docs — gravity well, must be filtered.
        assert!(
            !phrases.contains(&"ubiquit"),
            "gravity-well term 'ubiquit' (df=4/4) should be filtered, got: {phrases:?}"
        );
        // "alpha" is the context name — must not appear in any seed (neither
        // as a unigram nor inside a bigram).
        for phrase in &phrases {
            assert!(
                !phrase.split_whitespace().any(|tok| tok == "alpha"),
                "context-name 'alpha' leaked into seed '{phrase}': {phrases:?}"
            );
        }
        // "rarephrase" appears in 1/4 docs (25%) — survives the max-df filter.
        // With seed_min_doc_frequency: 1 it also survives the min-df filter.
        assert!(
            phrases.contains(&"rarephras"),
            "expected rare term 'rarephras' (df=1/4) in seeds, got: {phrases:?}"
        );
    }

    #[test]
    fn test_extract_seeds_handles_empty_vault() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = GraphIndexConfig::default();
        let seeds = extract_seeds(tmp.path(), &cfg, None);
        assert!(seeds.is_empty());
    }
}
