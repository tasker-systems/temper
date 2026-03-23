use temper_cli::embedder::{preprocess_chunk, preprocess_frontmatter};

// --- Preprocessing tests (no model needed) ---

#[test]
fn test_preprocess_chunk_prepends_header_path() {
    let result = preprocess_chunk("Some content", "Techniques > Branching");
    assert!(
        result.starts_with("Techniques > Branching: "),
        "expected result to start with 'Techniques > Branching: ', got: {:?}",
        result
    );
    assert!(
        result.contains("Some content"),
        "expected result to contain original content"
    );
}

#[test]
fn test_preprocess_chunk_empty_header_path() {
    let result = preprocess_chunk("Just some content", "");
    assert_eq!(
        result, "Just some content",
        "empty header_path should return content as-is"
    );
}

#[test]
fn test_preprocess_frontmatter() {
    let input = "type: concept\ntitle: Dialogue Systems\ntags:\n  - narrative";
    let result = preprocess_frontmatter(input);
    assert_eq!(result, input, "preprocess_frontmatter should be a pass-through");
}

#[test]
fn test_preprocess_strips_markdown_links() {
    // [[wikilinks]] -> text only
    let result = preprocess_chunk("See [[Branching Dialogue]] for details", "");
    assert!(
        !result.contains("[["),
        "wikilink brackets should be stripped"
    );
    assert!(
        result.contains("Branching Dialogue"),
        "wikilink text should be preserved"
    );

    // [text](url) -> text only
    let result2 = preprocess_chunk("Visit [the docs](https://example.com) for more", "");
    assert!(
        !result2.contains("]("),
        "markdown link syntax should be stripped"
    );
    assert!(
        result2.contains("the docs"),
        "link text should be preserved"
    );
    assert!(
        !result2.contains("https://example.com"),
        "link URL should be stripped"
    );
}

#[test]
fn test_preprocess_normalizes_whitespace() {
    let input = "First paragraph.\n\n\n\nSecond paragraph.\n\n\n\n\nThird paragraph.";
    let result = preprocess_chunk(input, "");
    // Multiple blank lines should be collapsed to at most one blank line
    assert!(
        !result.contains("\n\n\n"),
        "multiple blank lines should be collapsed, got: {:?}",
        result
    );
    assert!(
        result.contains("First paragraph."),
        "content should be preserved"
    );
    assert!(
        result.contains("Second paragraph."),
        "content should be preserved"
    );
    assert!(
        result.contains("Third paragraph."),
        "content should be preserved"
    );
}

// --- Model tests (require large download, always skipped in CI) ---

#[test]
#[ignore]
fn test_embedder_creates_and_loads_model() {
    use temper_cli::embedder::Embedder;
    use std::path::PathBuf;

    let cache_dir = PathBuf::from(std::env::var("TEMPER_MODEL_CACHE").unwrap_or_else(|_| {
        dirs::cache_dir()
            .unwrap()
            .join("temper")
            .join("models")
            .to_string_lossy()
            .into_owned()
    }));

    let mut embedder = Embedder::new(cache_dir);
    embedder.ensure_model().expect("model should load");
    assert_eq!(embedder.dimensions(), 384, "all-MiniLM-L6-v2 has 384 dimensions");
}

#[test]
#[ignore]
fn test_embed_single_text() {
    use temper_cli::embedder::Embedder;
    use std::path::PathBuf;

    let cache_dir = PathBuf::from(
        dirs::cache_dir()
            .unwrap()
            .join("temper")
            .join("models")
            .to_string_lossy()
            .into_owned(),
    );

    let mut embedder = Embedder::new(cache_dir);
    let vec = embedder.embed("Hello world").expect("embed should succeed");
    assert_eq!(vec.len(), 384, "embedding should have 384 dimensions");

    // Check L2 norm is approximately 1.0
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (norm - 1.0).abs() < 1e-4,
        "embedding should be L2-normalized, got norm={}",
        norm
    );
}

#[test]
#[ignore]
fn test_embed_batch() {
    use temper_cli::embedder::Embedder;
    use std::path::PathBuf;

    let cache_dir = PathBuf::from(
        dirs::cache_dir()
            .unwrap()
            .join("temper")
            .join("models")
            .to_string_lossy()
            .into_owned(),
    );

    let mut embedder = Embedder::new(cache_dir);
    let texts = vec!["First text", "Second text", "Third text"];
    let vecs = embedder
        .embed_batch(&texts)
        .expect("embed_batch should succeed");
    assert_eq!(vecs.len(), 3, "should return one vector per input");
    for (i, v) in vecs.iter().enumerate() {
        assert_eq!(v.len(), 384, "vector {} should have 384 dimensions", i);
    }
}

#[test]
#[ignore]
fn test_similar_texts_have_higher_cosine() {
    use temper_cli::embedder::Embedder;
    use std::path::PathBuf;

    let cache_dir = PathBuf::from(
        dirs::cache_dir()
            .unwrap()
            .join("temper")
            .join("models")
            .to_string_lossy()
            .into_owned(),
    );

    let mut embedder = Embedder::new(cache_dir);

    let query = embedder
        .embed("narrative design in games")
        .expect("embed query");
    let similar = embedder
        .embed("storytelling and game design")
        .expect("embed similar");
    let dissimilar = embedder
        .embed("database connection pooling in rust")
        .expect("embed dissimilar");

    let cosine_similar = cosine_similarity(&query, &similar);
    let cosine_dissimilar = cosine_similarity(&query, &dissimilar);

    assert!(
        cosine_similar > cosine_dissimilar,
        "similar text cosine ({}) should be higher than dissimilar ({})",
        cosine_similar,
        cosine_dissimilar
    );
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm_a * norm_b)
}
