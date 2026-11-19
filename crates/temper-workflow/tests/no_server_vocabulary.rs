//! `temper-workflow` is published as `temperkb-workflow`, and it publishes no server vocabulary.
//!
//! The server's write commands, what they return and the events they report are run only by
//! `DbBackend` in `temper-services`, so they live there. A door expresses a write as a
//! `temper_workflow::shaping` input instead. This guard reads the crate's own source and fails if
//! any of those types is declared in it again, or if the shared `Backend` trait returns.
//!
//! The matcher is line-grained and whole-word: it looks for `struct`, `enum` or `trait` followed
//! by a listed name outside a comment.

use std::path::{Path, PathBuf};

/// The server's write-command vocabulary, and the retired backend trait.
const SERVER_VOCABULARY: &[&str] = &[
    "AdvanceStewardWatermark",
    "AnnotateResource",
    "AssertRelationship",
    "AuditorDispatchTick",
    "CloseInvocation",
    "CommitDataArtifact",
    "CompleteAuditorJob",
    "CreateCognitiveMap",
    "CreateResource",
    "DeleteResource",
    "FoldRelationship",
    "GoalPatch",
    "ListResources",
    "MaterializeOnThreshold",
    "MoveSpec",
    "OpenInvocation",
    "ReblockResources",
    "ReconcileCognitiveMap",
    "RecordCitationAudit",
    "RetractFacet",
    "RetypeRelationship",
    "ReweightRelationship",
    "SearchResources",
    "SetFacet",
    "ShowResource",
    "StewardDispatchTick",
    "UpdateResource",
    "BodyUpdate",
    "ListFilter",
    "SearchQuery",
    "CommandOutput",
    "DomainEvent",
    "PushDeferReason",
    "Backend",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The name a line declares as a type, if it declares one: the word after `struct`, `enum` or
/// `trait`, outside a comment.
fn declared_type(line: &str) -> Option<&str> {
    let code = line.split("//").next().unwrap_or("");
    let words: Vec<&str> = code
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    words
        .windows(2)
        .find(|w| matches!(w[0], "struct" | "enum" | "trait"))
        .map(|w| w[1])
}

#[test]
fn the_published_crate_declares_no_server_vocabulary() {
    let mut src = Vec::new();
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut src);
    src.sort();
    let mut hits = Vec::new();
    for path in src {
        let text = std::fs::read_to_string(&path).expect("read source");
        for (n, line) in text.lines().enumerate() {
            if let Some(name) = declared_type(line) {
                if SERVER_VOCABULARY.contains(&name) {
                    hits.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "the published temper-workflow declares server vocabulary; it belongs in \
         temper-services:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_matcher_sees_what_it_claims() {
    assert_eq!(
        declared_type("pub struct CreateResource {"),
        Some("CreateResource")
    );
    assert_eq!(declared_type("pub enum DomainEvent {"), Some("DomainEvent"));
    assert_eq!(
        declared_type("pub trait Backend: Send + Sync {"),
        Some("Backend")
    );
    assert_eq!(declared_type("    // struct CreateResource is gone"), None);
    assert_eq!(
        declared_type("use temper_core::types::ids::ResourceId;"),
        None
    );
}
