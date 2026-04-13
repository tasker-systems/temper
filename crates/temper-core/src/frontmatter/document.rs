//! `Frontmatter` aggregate type and `DocType` enum.

use crate::error::{Result, TemperError};

/// Typed vault doctype. All valid values are enumerated exhaustively —
/// unknown doctypes fail at parse, not at validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocType {
    Task,
    Goal,
    Session,
    Research,
    Decision,
    Concept,
}

#[allow(clippy::should_implement_trait)]
impl DocType {
    /// Canonical string form as used in YAML frontmatter and vault paths.
    pub fn as_str(&self) -> &'static str {
        match self {
            DocType::Task => "task",
            DocType::Goal => "goal",
            DocType::Session => "session",
            DocType::Research => "research",
            DocType::Decision => "decision",
            DocType::Concept => "concept",
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
            other => Err(TemperError::Config(format!(
                "unknown doctype '{other}'; expected one of: task, goal, session, research, decision, concept"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_type_round_trip_all_six() {
        for name in ["task", "goal", "session", "research", "decision", "concept"] {
            let dt = DocType::from_str(name).expect("valid doctype");
            assert_eq!(dt.as_str(), name);
        }
    }

    #[test]
    fn doc_type_rejects_unknown() {
        assert!(DocType::from_str("bogus").is_err());
        assert!(DocType::from_str("").is_err());
        assert!(DocType::from_str("Task").is_err()); // case-sensitive
    }
}
