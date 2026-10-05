//! The document types a resource can be: the closed vocabulary every surface names a resource's
//! kind with. A wire type, so temper-core's; the per-type JSON Schema text the type carries in
//! temper-workflow (`DocTypeSchema`) stays there, beside the schema files it embeds.

use serde::{Deserialize, Serialize};

use crate::error::{Result, TemperError};

/// Typed vault doctype. All valid values are enumerated exhaustively —
/// unknown doctypes fail at parse, not at validation.
///
/// Includes the 8 cognitive-map node labels (spec D3: fact, memory,
/// question, theme, concern, principle, commitment, domain) alongside the
/// original resource doctypes. The set is still closed here — Task A2 loosens
/// the parse gate to an open tail; this variant list is the recognized core.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "doc_type.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocType {
    Task,
    Goal,
    Session,
    Research,
    Decision,
    Concept,
    // Cognitive-map node labels (spec D3). Open tail handled at the
    // validation gates (Task A2).
    Fact,
    Memory,
    Question,
    Theme,
    Concern,
    Principle,
    Commitment,
    Domain,
}

#[expect(
    clippy::should_implement_trait,
    reason = "inherent from_str returns anyhow::Result, not the std FromStr contract"
)]
impl DocType {
    /// Every doc-type variant, in canonical order. Single source of truth for
    /// callers that enumerate the known doc types (e.g. the MCP `list_doc_types`
    /// tool, post-collapse — the substrate stores doc-type as a property name,
    /// so there is no `kb_doc_types` table to list).
    pub const ALL: &'static [DocType] = &[
        DocType::Task,
        DocType::Goal,
        DocType::Session,
        DocType::Research,
        DocType::Decision,
        DocType::Concept,
        DocType::Fact,
        DocType::Memory,
        DocType::Question,
        DocType::Theme,
        DocType::Concern,
        DocType::Principle,
        DocType::Commitment,
        DocType::Domain,
    ];

    /// Canonical string form as used in YAML frontmatter and vault paths.
    pub fn as_str(&self) -> &'static str {
        match self {
            DocType::Task => "task",
            DocType::Goal => "goal",
            DocType::Session => "session",
            DocType::Research => "research",
            DocType::Decision => "decision",
            DocType::Concept => "concept",
            DocType::Fact => "fact",
            DocType::Memory => "memory",
            DocType::Question => "question",
            DocType::Theme => "theme",
            DocType::Concern => "concern",
            DocType::Principle => "principle",
            DocType::Commitment => "commitment",
            DocType::Domain => "domain",
        }
    }

    /// Parse from canonical string form. Case-sensitive.
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "task" => Ok(DocType::Task),
            "goal" => Ok(DocType::Goal),
            "session" => Ok(DocType::Session),
            "research" => Ok(DocType::Research),
            "decision" => Ok(DocType::Decision),
            "concept" => Ok(DocType::Concept),
            "fact" => Ok(DocType::Fact),
            "memory" => Ok(DocType::Memory),
            "question" => Ok(DocType::Question),
            "theme" => Ok(DocType::Theme),
            "concern" => Ok(DocType::Concern),
            "principle" => Ok(DocType::Principle),
            "commitment" => Ok(DocType::Commitment),
            "domain" => Ok(DocType::Domain),
            other => Err(TemperError::Config(format!(
                "unknown doctype '{other}'; expected one of: task, goal, session, research, decision, concept, fact, memory, question, theme, concern, principle, commitment, domain"
            ))),
        }
    }
}
