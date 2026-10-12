//! The model of `resource-erasure.toml`: what resource erasure does with every carrier column
//! (`[tables]`) and every free-text ledger path (`[payload]`).
//!
//! The vocabularies and grammars moved here from `resource_erasure_surface_test.rs`'s hand parser:
//! `D2_STEPS`, `REDACT_CLASSES`, `enum Disposition`, `fn parse_disposition`,
//! `enum PayloadDisposition` and `fn payload_lines`. One shape changed: a qualified payload line is
//! nested under its unqualified line as `refine`, so a qualified line that refines nothing cannot
//! be written.

use std::collections::BTreeMap;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};

use super::columns::ColumnKey;
use super::GrammarError;

/// The spec's D2 step numbering, which `handled:`, `repointed:` and `resource-erasure:` cite.
const D2_STEPS: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "7a", "8", "9", "9f"];

/// `step` if it is a D2 step.
pub(super) fn d2_step(step: &str) -> Result<String, GrammarError> {
    if D2_STEPS.contains(&step) {
        Ok(step.to_owned())
    } else {
        Err(GrammarError(format!(
            "no D2 step {step:?} (known: {D2_STEPS:?})"
        )))
    }
}

/// The redaction classes of D4, one per sentinel shape.
const REDACT_CLASSES: &[&str] = &[
    "title",
    "origin-uri",
    "remote-source-url",
    "property-key",
    "property-value",
    "facet-value",
    "doc-type",
    "artifact-family",
    "edge-label",
    "reason",
    "scar",
    "authorship",
];

/// What resource erasure does with a `[tables]` column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// `handled:D2.<step>`: emptied or sentineled by this D2 step.
    Handled { step: String },
    /// `repointed:D2.<step>:<table>.<column>`: its citers are re-pointed through `via` and the
    /// orphaned original deleted.
    Repointed { step: String, via: ColumnKey },
    /// `out-of-scope`: deliberately left; the note carries the reason.
    OutOfScope,
}

impl FromStr for Disposition {
    type Err = GrammarError;

    fn from_str(raw: &str) -> Result<Self, GrammarError> {
        if let Some(step) = raw.strip_prefix("handled:D2.") {
            return Ok(Self::Handled {
                step: d2_step(step)?,
            });
        }
        if let Some(rest) = raw.strip_prefix("repointed:D2.") {
            let (step, via) = rest.split_once(':').ok_or_else(|| {
                GrammarError(format!(
                    "expected `repointed:D2.<step>:<table>.<column>`, got {raw:?}"
                ))
            })?;
            return Ok(Self::Repointed {
                step: d2_step(step)?,
                via: via.parse()?,
            });
        }
        if raw == "out-of-scope" {
            return Ok(Self::OutOfScope);
        }
        Err(GrammarError(format!(
            "unknown disposition {raw:?} (known: `handled:D2.<step>`, \
             `repointed:D2.<step>:<table>.<column>`, `out-of-scope`)"
        )))
    }
}

deserialize_from_str!(Disposition);

/// One `[tables]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableLine {
    /// What the act does with the column.
    pub disposition: Disposition,
    /// Free text for the reader; empty when absent.
    #[serde(default)]
    pub note: String,
}

/// What resource erasure does with a ledger path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadDisposition {
    /// `redact:<class>`: rewritten to the D4 sentinel of the class and recorded in
    /// `redacted_fields`.
    Redact(String),
    /// `structural`: a hash, a vocabulary word or a record's own fixed text; kept.
    Structural,
    /// `out-of-scope`: not the resource's text; the note carries the reason.
    OutOfScope,
    /// `permissive`: the event type has no payload schema; its paths are declared by hand. Goes
    /// with the path `*` and only with it.
    Permissive,
}

impl FromStr for PayloadDisposition {
    type Err = GrammarError;

    fn from_str(raw: &str) -> Result<Self, GrammarError> {
        match raw {
            "structural" => return Ok(Self::Structural),
            "out-of-scope" => return Ok(Self::OutOfScope),
            "permissive" => return Ok(Self::Permissive),
            _ => {}
        }
        let class = raw.strip_prefix("redact:").ok_or_else(|| {
            GrammarError(format!(
                "unknown disposition {raw:?} (known: `redact:<class>`, `structural`, \
                 `out-of-scope`, `permissive`)"
            ))
        })?;
        if REDACT_CLASSES.contains(&class) {
            Ok(Self::Redact(class.to_owned()))
        } else {
            Err(GrammarError(format!(
                "no D4 class {class:?} (known: {REDACT_CLASSES:?})"
            )))
        }
    }
}

deserialize_from_str!(PayloadDisposition);

/// One `[payload."<event_type>"]` entry, under its path key (a JSON pointer, or `*`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadLine {
    /// What the act does with the path.
    pub disposition: PayloadDisposition,
    /// Free text for the reader; empty when absent.
    #[serde(default)]
    pub note: String,
    /// Narrower dispositions for the same path, each matched by its event's own fields. The most
    /// specific line wins.
    #[serde(default)]
    pub refine: Vec<Refinement>,
}

/// A qualified line: the same path, narrowed by its event's fields.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refinement {
    /// Which events it matches.
    #[serde(rename = "where")]
    pub r#where: Qualifier,
    /// What the act does with the path on a matching event.
    pub disposition: PayloadDisposition,
    /// Free text for the reader; empty when absent.
    #[serde(default)]
    pub note: String,
}

/// The event fields a refinement matches on, as `_erasure_redact_paths()` holds them (its
/// `key_qualifier` and `owner_qualifier`). At least one is present.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Qualifier {
    /// The property key the event asserts.
    pub key: Option<String>,
    /// The owner table of the property.
    pub owner: Option<String>,
}

/// `resource-erasure.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceErasure {
    /// `table.column` → what the act does with that carrier column.
    pub tables: BTreeMap<ColumnKey, TableLine>,
    /// Event type (or `metadata`, for the authorship keys on any event) → path → its line.
    #[serde(deserialize_with = "payload")]
    pub payload: BTreeMap<String, BTreeMap<String, PayloadLine>>,
}

impl ResourceErasure {
    /// Each `[tables]` column handled by a D2 step, with the step.
    pub fn handled(&self) -> impl Iterator<Item = (&ColumnKey, &str)> {
        self.tables
            .iter()
            .filter_map(|(key, line)| match &line.disposition {
                Disposition::Handled { step } => Some((key, step.as_str())),
                _ => None,
            })
    }
}

type Payload = BTreeMap<String, BTreeMap<String, PayloadLine>>;

/// `[payload]`, with each path checked against the grammar the TOML shape cannot express.
fn payload<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Payload, D::Error> {
    let payload = Payload::deserialize(deserializer)?;
    for (event_type, paths) in &payload {
        for (path, line) in paths {
            check_payload_line(path, line).map_err(|e| {
                serde::de::Error::custom(format!("[payload.{event_type}] {path:?}: {e}"))
            })?;
        }
    }
    Ok(payload)
}

/// A path is a JSON pointer or `*`; `*` and `permissive` go together, on the line and on each of
/// its refinements; a refinement names a qualifier, and no two refinements name the same one.
fn check_payload_line(path: &str, line: &PayloadLine) -> Result<(), String> {
    if path != "*" && !path.starts_with('/') {
        return Err("a path is a JSON pointer or `*`".to_owned());
    }
    let star_goes_with_permissive = |disposition: &PayloadDisposition| {
        (path == "*") == (*disposition == PayloadDisposition::Permissive)
    };
    if !star_goes_with_permissive(&line.disposition) {
        return Err("`*` and `permissive` go together".to_owned());
    }
    for (i, refinement) in line.refine.iter().enumerate() {
        if !star_goes_with_permissive(&refinement.disposition) {
            return Err("`*` and `permissive` go together, in a refinement too".to_owned());
        }
        let qualifier = &refinement.r#where;
        if qualifier.key.is_none() && qualifier.owner.is_none() {
            return Err("a refinement's `where` names `key`, `owner` or both".to_owned());
        }
        if line.refine[..i].iter().any(|r| r.r#where == *qualifier) {
            return Err(format!("{qualifier:?} refines this path twice"));
        }
    }
    Ok(())
}
