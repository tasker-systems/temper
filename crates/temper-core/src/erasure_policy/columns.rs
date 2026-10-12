//! The column models of `personal-data.toml` and `sensitivity-scan.toml`, and their vocabularies.
//!
//! The vocabularies and the achieved-claim grammar moved here from the hand parsers they replace:
//! `personal_data_surface_test.rs`'s `CLASSES`, `CEILINGS`, `ARMS`, `enum Claim`, `enum Achieved`
//! and `fn parse_claim`, and `sensitivity_scan_surface_test.rs`'s `DISPOSITIONS`.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

use super::GrammarError;
use super::resource_erasure::d2_step;

/// A `table.column` key, both halves non-empty. Ordered by its text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColumnKey {
    key: String,
    /// The byte offset of the first `.`, which `from_str` proved is there.
    dot: usize,
}

impl ColumnKey {
    /// The key as written: `table.column`.
    pub fn as_str(&self) -> &str {
        &self.key
    }

    /// The table half.
    pub fn table(&self) -> &str {
        &self.key[..self.dot]
    }

    /// The column half.
    pub fn column(&self) -> &str {
        &self.key[self.dot + 1..]
    }
}

impl FromStr for ColumnKey {
    type Err = GrammarError;

    fn from_str(raw: &str) -> Result<Self, GrammarError> {
        match raw.find('.') {
            Some(dot) if dot > 0 && dot + 1 < raw.len() => Ok(Self {
                key: raw.to_owned(),
                dot,
            }),
            _ => Err(GrammarError(format!("{raw:?} is not `table.column`"))),
        }
    }
}

impl fmt::Display for ColumnKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key)
    }
}

deserialize_from_str!(ColumnKey);

/// What class of personal data a column holds (formerly `CLASSES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    /// A direct identifier of a natural person.
    Identifier,
    /// A pseudonymous UUID pointing at a principal.
    Reference,
    /// Authored content.
    Content,
    /// A representation computed from content (vector, search index).
    Derived,
    /// Free-form that may carry personal data depending on what was written.
    Incidental,
    /// The table-name half of a polymorphic pair: no personal data, but it makes its `_id` sibling
    /// resolvable.
    Discriminator,
    /// Carries no personal data.
    None,
}

/// How far erasure can reach a column (formerly `CEILINGS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ceiling {
    /// Erasable outright.
    Full,
    /// Must be recomputed after an erasure, not deleted.
    Recompute,
    /// A UUID that stops identifying anyone once `kb_profiles` is tombstoned.
    Pseudonym,
    /// On the append-only ledger; cannot be erased at all.
    None,
    /// Nothing to erase (`n-a`).
    #[serde(rename = "n-a")]
    NotApplicable,
}

/// The person act's arms, as its body's comments number them (20261023100000). A label for the
/// reader: the binding test checks only that a claim names one.
const ARMS: &[&str] = &[
    "1", "2", "3", "4", "4a", "5", "6", "6a", "7", "7a", "8", "9", "9a", "10", "11", "12",
];

fn arm(raw: &str) -> Result<String, GrammarError> {
    if ARMS.contains(&raw) {
        Ok(raw.to_owned())
    } else {
        Err(GrammarError(format!(
            "no arm {raw:?} in the person act (known: {ARMS:?})"
        )))
    }
}

/// One way the person act reaches a column. Each is bound to a statement in the live act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// `erased:<arm>`: `_erasure_apply_redaction` assigns the column an erasure value.
    Erased { arm: String },
    /// `deleted:<arm>`: `_erasure_apply_redaction` deletes from the column's table.
    Deleted { arm: String },
    /// `cascaded:<arm>:<parent>`: `_erasure_apply_redaction` deletes from `parent`, which cascades
    /// to the column's table.
    Cascaded { arm: String, parent: String },
    /// `resource-erasure:D2.<step>`: the resource erasures the act runs over the estate, whose
    /// manifest handles the column.
    ResourceErasure { step: String },
}

impl FromStr for Claim {
    type Err = GrammarError;

    fn from_str(raw: &str) -> Result<Self, GrammarError> {
        if let Some(rest) = raw.strip_prefix("erased:") {
            return Ok(Self::Erased { arm: arm(rest)? });
        }
        if let Some(rest) = raw.strip_prefix("deleted:") {
            return Ok(Self::Deleted { arm: arm(rest)? });
        }
        if let Some(rest) = raw.strip_prefix("cascaded:") {
            let (a, parent) = rest.split_once(':').ok_or_else(|| {
                GrammarError(format!(
                    "expected `cascaded:<arm>:<parent table>`, got {raw:?}"
                ))
            })?;
            return Ok(Self::Cascaded {
                arm: arm(a)?,
                parent: parent.to_owned(),
            });
        }
        let step = raw.strip_prefix("resource-erasure:D2.").ok_or_else(|| {
            GrammarError(format!(
                "unknown achieved claim {raw:?} (known: `erased:<arm>`, `deleted:<arm>`, \
                 `cascaded:<arm>:<table>`, `resource-erasure:D2.<step>`, `remainder: <reason>`, `-`)"
            ))
        })?;
        Ok(Self::ResourceErasure {
            step: d2_step(step)?,
        })
    }
}

/// What a column's entry says the person act achieves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Achieved {
    /// `-`: the ceiling is not `full`, so there is nothing to achieve.
    Unstated,
    /// Claims joined with ` + `.
    Claims(Vec<Claim>),
    /// `remainder: <reason>`: the act does not reach it.
    Remainder(String),
}

impl FromStr for Achieved {
    type Err = GrammarError;

    fn from_str(raw: &str) -> Result<Self, GrammarError> {
        if raw == "-" {
            return Ok(Self::Unstated);
        }
        if let Some(reason) = raw.strip_prefix("remainder:") {
            return Ok(Self::Remainder(reason.trim().to_owned()));
        }
        raw.split(" + ")
            .map(|claim| claim.trim().parse::<Claim>())
            .collect::<Result<Vec<_>, _>>()
            .map(Self::Claims)
    }
}

deserialize_from_str!(Achieved);

/// One `personal-data.toml` column entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalColumn {
    /// What class of personal data the column holds.
    pub class: Class,
    /// How far erasure can reach it.
    pub ceiling: Ceiling,
    /// What the person act does reach.
    pub achieved: Achieved,
    /// Free text for the reader; empty when absent.
    #[serde(default)]
    pub note: String,
}

/// How the sensitivity sweep treats a text column (formerly `DISPOSITIONS`). There is deliberately
/// no `unadjudicated` disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanDisposition {
    /// Authored or user-supplied free text; the sweep reads it.
    Scan,
    /// A hash, enum, discriminator, surrogate, vocabulary, or a value derived from one.
    Structural,
    /// Secret material by design; a sweep must never echo it.
    Credential,
    /// Anything else; the note carries the reason.
    OutOfScope,
}

/// One `sensitivity-scan.toml` column entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanColumn {
    /// How the sweep treats the column.
    pub disposition: ScanDisposition,
    /// Free text for the reader; empty when absent.
    #[serde(default)]
    pub note: String,
}

/// `sensitivity-scan.toml`: the text columns no personal-data derivation nominates.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensitivityScan {
    /// The migration the stated count was taken at.
    pub count_at: String,
    /// The stated number of columns declared in this file.
    pub declared_count: usize,
    /// `table.column` → its disposition.
    pub columns: BTreeMap<ColumnKey, ScanColumn>,
}

impl SensitivityScan {
    /// The columns declared with `disposition`.
    pub fn keys(&self, disposition: ScanDisposition) -> impl Iterator<Item = &ColumnKey> {
        self.columns
            .iter()
            .filter(move |(_, column)| column.disposition == disposition)
            .map(|(key, _)| key)
    }
}
