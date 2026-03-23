use temper_cli::chunker::chunk_document;

const CONCEPT_NOTE: &str = r#"---
type: concept
title: Dialogue Systems
tags:
  - narrative
  - game-design
  - branching
---

# Dialogue Systems

Overview of dialogue systems in interactive fiction.

## Core Concepts

Dialogue systems allow players to interact with NPCs through structured choices.

### Branching Dialogue

Branching dialogue creates trees of conversation paths.

Each branch can lead to different outcomes.

### Linear Dialogue

Linear dialogue progresses in a fixed sequence without player choice.

## Techniques

Advanced techniques for building dialogue systems.

### Branching Dialogue

The branching approach uses conditional logic to select responses.

Implementation involves tracking state flags and evaluating conditions.

### Contextual Responses

Responses change based on prior game state.

This creates the illusion of a living world.

## Implementation Notes

Technical considerations for implementing dialogue systems in practice.
"#;

const TICKET_NOTE: &str = r#"---
type: ticket
title: Implement Dialogue Parser
project: storyteller-core
cluster: storyteller
milestone: v1-foundation
stage: design
seq: 42
---

# Implement Dialogue Parser

Design and implement the core dialogue parsing system.
"#;

const NO_FRONTMATTER: &str = r#"# Just a Header

Some content without any frontmatter.

## Section

More content here.
"#;

#[test]
fn test_chunk_extracts_frontmatter_as_first_chunk() {
    let chunks = chunk_document("concepts/Dialogue Systems.md", CONCEPT_NOTE);
    assert!(!chunks.is_empty(), "should produce at least one chunk");
    let fm_chunk = &chunks[0];
    assert!(
        fm_chunk.content.contains("title:"),
        "frontmatter chunk should contain title key"
    );
    assert!(
        fm_chunk.content.contains("Dialogue Systems"),
        "frontmatter chunk should contain the title value"
    );
    assert!(
        fm_chunk.content.contains("type:") || fm_chunk.content.contains("concept"),
        "frontmatter chunk should contain note type"
    );
    assert_eq!(fm_chunk.metadata.note_type, "concept");
    assert_eq!(fm_chunk.metadata.title, "Dialogue Systems");
    assert!(
        fm_chunk.metadata.tags.contains(&"narrative".to_string()),
        "tags should include 'narrative'"
    );
}

#[test]
fn test_chunk_splits_on_headers() {
    let chunks = chunk_document("concepts/Dialogue Systems.md", CONCEPT_NOTE);
    // Should have: frontmatter + intro + ## Core Concepts + ### Branching Dialogue
    // + ### Linear Dialogue + ## Techniques + ### Branching Dialogue
    // + ### Contextual Responses + ## Implementation Notes = at least 4 chunks
    assert!(
        chunks.len() >= 4,
        "expected at least 4 chunks, got {}",
        chunks.len()
    );
}

#[test]
fn test_chunk_preserves_header_breadcrumb() {
    let chunks = chunk_document("concepts/Dialogue Systems.md", CONCEPT_NOTE);
    // Find a chunk that falls under "## Techniques > ### Branching Dialogue"
    let branching_in_techniques = chunks.iter().find(|c| {
        c.header_path.contains("Techniques") && c.header_path.contains("Branching Dialogue")
    });
    assert!(
        branching_in_techniques.is_some(),
        "expected a chunk with header_path containing 'Techniques' and 'Branching Dialogue', found paths: {:?}",
        chunks.iter().map(|c| &c.header_path).collect::<Vec<_>>()
    );
}

#[test]
fn test_chunk_ids_are_sequential() {
    let file_path = "concepts/Dialogue Systems.md";
    let chunks = chunk_document(file_path, CONCEPT_NOTE);
    for (i, chunk) in chunks.iter().enumerate() {
        let expected_id = format!("{}#chunk:{}", file_path, i);
        assert_eq!(
            chunk.id, expected_id,
            "chunk {} has wrong id: {}",
            i, chunk.id
        );
        assert_eq!(chunk.chunk_index, i);
    }
}

#[test]
fn test_chunk_file_path_set_correctly() {
    let file_path = "sessions/2026-03-22-test.md";
    let chunks = chunk_document(file_path, CONCEPT_NOTE);
    for chunk in &chunks {
        assert_eq!(
            chunk.file_path, file_path,
            "chunk file_path should match input"
        );
    }
}

#[test]
fn test_chunk_metadata_from_frontmatter() {
    let chunks = chunk_document("tickets/storyteller/ticket-42.md", TICKET_NOTE);
    assert!(!chunks.is_empty());
    let meta = &chunks[0].metadata;
    assert_eq!(meta.note_type, "ticket");
    assert_eq!(meta.title, "Implement Dialogue Parser");
    assert_eq!(meta.project.as_deref(), Some("storyteller-core"));
    assert_eq!(meta.cluster.as_deref(), Some("storyteller"));
}

#[test]
fn test_chunk_no_frontmatter() {
    let chunks = chunk_document("misc/no-frontmatter.md", NO_FRONTMATTER);
    assert!(!chunks.is_empty());
    for chunk in &chunks {
        assert_eq!(
            chunk.metadata.note_type, "unknown",
            "note_type should be 'unknown' when no frontmatter"
        );
    }
}

#[test]
fn test_chunk_empty_document() {
    let chunks = chunk_document("misc/empty.md", "");
    assert!(chunks.is_empty(), "empty content should produce no chunks");
}

#[test]
fn test_chunk_frontmatter_only() {
    let frontmatter_only = "---\ntype: concept\ntitle: Minimal Note\n---\n";
    let chunks = chunk_document("concepts/Minimal Note.md", frontmatter_only);
    assert_eq!(
        chunks.len(),
        1,
        "frontmatter-only doc should produce exactly 1 chunk"
    );
    assert!(chunks[0].content.contains("Minimal Note"));
}
