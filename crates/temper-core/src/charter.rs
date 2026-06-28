//! The charter prose-assembly rule, defined ONCE. A telos charter is real role-tagged content blocks:
//! block-0 is the statement (role `"statement"`), then each question (role `"question"`, the question
//! plus its context when present), then the framing lines (role `"framing"`). Both the CLI
//! (`temper cogmap reconcile`) and the substrate genesis path (`TelosDef::block_specs`) call this, so the
//! delivered charter and the genesis-born charter cannot drift.
use serde::{Deserialize, Serialize};

/// One charter question with its disambiguating context (context defaults empty).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharterQuestion {
    pub question: String,
    #[serde(default)]
    pub context: String,
}

/// Assemble `{statement, questions, framing}` into ordered `(block_role, prose)` specs.
pub fn charter_block_specs(
    statement: &str,
    questions: &[CharterQuestion],
    framing: &[String],
) -> Vec<(&'static str, String)> {
    let mut specs = Vec::with_capacity(1 + questions.len() + framing.len());
    specs.push(("statement", statement.to_owned()));
    for q in questions {
        let prose = if q.context.is_empty() {
            q.question.clone()
        } else {
            format!("{}\n\n{}", q.question, q.context)
        };
        specs.push(("question", prose));
    }
    for f in framing {
        specs.push(("framing", f.clone()));
    }
    specs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_statement_questions_framing_in_order() {
        let qs = vec![
            CharterQuestion {
                question: "What transfers?".into(),
                context: "prior knowledge".into(),
            },
            CharterQuestion {
                question: "Bare?".into(),
                context: String::new(),
            },
        ];
        let specs = charter_block_specs("The statement.", &qs, &["Framing one.".to_string()]);
        assert_eq!(specs[0], ("statement", "The statement.".to_string()));
        // context appended with a blank line when present…
        assert_eq!(
            specs[1],
            ("question", "What transfers?\n\nprior knowledge".to_string())
        );
        // …and omitted entirely when empty.
        assert_eq!(specs[2], ("question", "Bare?".to_string()));
        assert_eq!(specs[3], ("framing", "Framing one.".to_string()));
        assert_eq!(specs.len(), 4);
    }
}
