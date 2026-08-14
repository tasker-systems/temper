//! The predicate layer. Bounds are membership and terms are magnitude; neither can narrow by what
//! a thing IS. Everything carries `kb_properties` (`doc_type`, `tags`, `facet`) and every edge
//! carries BOTH an `edge_kind` and a `label`.
//!
//! Typed slots, deliberately NOT a generic `{field, op, value}` grammar: a general predicate
//! language would be more expressive and would immediately re-open every conflation this contract
//! exists to close.

use serde::{Deserialize, Serialize};

// The four members of the DDL's `edge_kind` enum
// (`migrations/20260624000001_canonical_schema.sql:95`) are ALREADY modelled, and re-used here
// rather than restated. `types::graph::EdgeKind` is `sqlx::Type`-bound to that DDL, so it is the
// copy a schema change breaks — which is exactly why the contract must not carry a second one.
// Its closedness is the fix for the audit's #1 finding: an edge `label` such as `advances` cannot
// be passed here.
use crate::types::graph::EdgeKind;

/// Narrowing over edges. `edge_kinds` and `labels` are DIFFERENT AXES and are never merged: the
/// kind is a closed DDL enum, the label is free text the caller actually sees on every edge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct EdgeFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_kinds: Vec<EdgeKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

/// One `kb_properties` facet predicate, at the inner-key grain the facet model uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct FacetPredicate {
    pub key: String,
    pub value: String,
}

/// Narrowing over resources. Every field is AND-composed; an unset field narrows nothing.
///
/// **No field here has a closed vocabulary, and none is checked against one.**
/// `[corrected — 2026-08-10, ADJ-10]` This claimed `doc_type`, `stage` and `status` were closed
/// vocabularies whose unknown values raise `RefusalReason::UnknownFilterValue`. None of the three
/// is: `stage` and `status` are free-form `Option<String>` and are refused wholesale by this door as
/// `FilterNotApplicable`, and `doc_type` is a `kb_properties` row a resource may carry any value
/// for. `UnknownFilterValue` is raised for exactly one thing here — an unrecognized
/// [`PropertySubject`] — and its own doc carries the ruling.
///
/// The rule that replaces the old claim: *an unknown value in a genuinely closed set* is a refusal,
/// because it can never match; *a string that may be perfectly legitimate and matches nothing in the
/// scope you asked about* is an honest empty. `doc_type` is the second kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceFilter {
    /// `kb_properties` where `property_key = 'doc_type'`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc_type: Vec<String>,
    /// `kb_properties` where `property_key = 'tags'`. AND-containment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// `kb_properties` where `property_key = 'facet'`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facets: Vec<FacetPredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
}

/// Which filter slot an act admits. An unadmitted filter is DECLINED, never ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FilterField {
    Resource,
    Edge,
}

/// What a [`PropertyPredicate`] addresses.
///
/// OPEN, deliberately — `kb_properties.owner_table` is a `varchar` mirroring no DDL enum, so a
/// closed set here would be a claim the schema does not make. This is the OPPOSITE call from
/// [`EdgeKind`], and principled rather than inconsistent: `EdgeKind` mirrors a DDL enum, so its
/// closedness is a *fact about the database*; `owner_table` mirrors nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PropertySubject {
    Resource,
    /// Empty in this deployment's data and NOT empty in others — the polymorphic owner is design
    /// intent, not accident. Spec §12.
    Edge,
    /// An unrecognized subject — e.g. `content_block`, which is addressable but deliberately not a
    /// queryable subject (spec §12). Validation renders it as `UnknownFilterValue`.
    #[serde(untagged)]
    Other(String),
}

/// A property narrowing operator. CLOSED — the key space is open, the operator set is not. Neither
/// operator takes a fragment of a query language; both bind their values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum PropertyOp {
    /// The key is present at all. A row-existence check on the `property_key` btree — NOT a jsonb
    /// operator, because `jsonb_path_ops` does not index key-existence and the btree already
    /// answers it.
    HasKey,
    /// `property_value @> $v` for any listed value. OR within the predicate, matching the
    /// established within-field OR of `doc_type` and `EdgeFilter.labels`.
    ///
    /// **Containment is ASYMMETRIC, and the asymmetry runs the useful way.**
    /// `[corrected — 2026-08-14]` This said *"containment does not coerce:
    /// `'["x"]'::jsonb @> '"x"'::jsonb` is FALSE, so a type-unstable key needs both shapes
    /// listed."* Measured against Postgres 18, that expression is **TRUE** — it is the documented
    /// special exception whereby a top-level array contains a primitive. The reverse is the false
    /// one:
    ///
    /// ```text
    ///  '["x"]'::jsonb @> '"x"'::jsonb   -> t     (array contains scalar)
    ///  '"x"'::jsonb   @> '["x"]'::jsonb -> f     (scalar does not contain array)
    /// ```
    ///
    /// The row's value is on the LEFT, so a **scalar** probe matches both the array-shaped rows
    /// and the scalar-shaped ones, while an **array** probe matches only the array-shaped rows.
    /// The conclusion therefore survives inverted and weaker than it was stated: a type-unstable
    /// key needs the scalar shape, not both. Listing both is harmless — the values OR — but it is
    /// not what makes the predicate span the population, and a caller who lists only the array
    /// shape silently answers for one half of it.
    Contains { values: Vec<serde_json::Value> },
}

/// A property predicate: what it addresses, which key, and how.
///
/// The subject is CARRIED, never inferred, because inference is ambiguous exactly where it matters:
/// a `follow-from` stage walks edges and produces resources, so "the properties of this stage's
/// subject" has two answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct PropertyPredicate {
    pub subject: PropertySubject,
    pub key: String,
    pub op: PropertyOp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_kind_is_closed_at_the_four_the_ddl_declares() {
        // migrations/20260624000001_canonical_schema.sql:95
        //   CREATE TYPE edge_kind AS ENUM ('express', 'contains', 'leads_to', 'near');
        //
        // Regression cover over the INCUMBENT `types::graph::EdgeKind`, which this contract
        // re-uses rather than restates. These assertions are what make the re-use safe: they fail
        // if the shared type ever stops having the properties the contract depends on.
        for (k, j) in [
            (EdgeKind::Express, "\"express\""),
            (EdgeKind::Contains, "\"contains\""),
            (EdgeKind::LeadsTo, "\"leads_to\""),
            (EdgeKind::Near, "\"near\""),
        ] {
            assert_eq!(serde_json::to_string(&k).unwrap(), j);
        }
    }

    #[test]
    fn a_label_cannot_be_passed_as_an_edge_kind() {
        // THE audit's #1 finding, fixed at the type level. `advances` is a real LABEL that appears
        // on real edges; it is not an edge_kind. Today `--edge-type advances` silently narrows to
        // nothing with reason: ok. Here it cannot be constructed at all.
        assert!(serde_json::from_str::<EdgeKind>("\"advances\"").is_err());
        assert!(serde_json::from_str::<EdgeKind>("\"derived_from\"").is_err());
    }

    #[test]
    fn labels_and_edge_kinds_are_separate_fields_on_the_filter() {
        // Separate slots, different types — so the caller who means "advances" has exactly one
        // place to put it, and it is the right one.
        let f = EdgeFilter {
            edge_kinds: vec![EdgeKind::LeadsTo],
            labels: vec!["advances".to_string()],
        };
        let back: EdgeFilter = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
        assert_eq!(back.edge_kinds, vec![EdgeKind::LeadsTo]);
        assert_eq!(back.labels, vec!["advances".to_string()]);
    }

    #[test]
    fn an_empty_filter_serializes_to_nothing() {
        let f = ResourceFilter::default();
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "{}", "an unset filter must not emit empty arrays");
        assert_eq!(serde_json::from_str::<ResourceFilter>("{}").unwrap(), f);
    }

    #[test]
    fn resource_filters_compose_and_round_trip() {
        // filters-compose-to-narrow: several predicates on one request, AND semantics.
        let f = ResourceFilter {
            doc_type: vec!["task".to_string()],
            tags: vec!["search".to_string(), "ci".to_string()],
            facets: vec![FacetPredicate {
                key: "domain".to_string(),
                value: "search".to_string(),
            }],
            stage: Some("in-progress".to_string()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_str::<ResourceFilter>(&serde_json::to_string(&f).unwrap()).unwrap(),
            f
        );
    }

    #[test]
    fn filter_fields_name_the_two_slots_an_act_may_admit() {
        assert_eq!(
            serde_json::to_string(&FilterField::Resource).unwrap(),
            "\"resource\""
        );
        assert_eq!(
            serde_json::to_string(&FilterField::Edge).unwrap(),
            "\"edge\""
        );
    }

    #[test]
    fn a_property_subject_is_open_because_owner_table_is_a_varchar() {
        // The opposite call from EdgeKind, and principled rather than inconsistent: EdgeKind mirrors
        // a DDL enum so closedness is a FACT; owner_table mirrors nothing, so closedness would be a
        // claim the schema does not make.
        assert_eq!(
            serde_json::to_string(&PropertySubject::Edge).unwrap(),
            "\"edge\""
        );
        let unknown: PropertySubject =
            serde_json::from_str("\"block\"").expect("open, so it parses");
        assert_eq!(unknown, PropertySubject::Other("block".to_string()));
    }

    #[test]
    fn has_key_and_contains_are_the_whole_v1_vocabulary() {
        // No operator takes a fragment of a query language. Both bind.
        let hk = PropertyPredicate {
            subject: PropertySubject::Resource,
            key: "keywords".to_string(),
            op: PropertyOp::HasKey,
        };
        let ct = PropertyPredicate {
            subject: PropertySubject::Edge,
            key: "confidence".to_string(),
            op: PropertyOp::Contains {
                values: vec![serde_json::json!("high")],
            },
        };
        for p in [hk, ct] {
            assert_eq!(
                serde_json::from_str::<PropertyPredicate>(&serde_json::to_string(&p).unwrap())
                    .unwrap(),
                p
            );
        }
    }

    #[test]
    fn contains_carries_a_list_so_one_predicate_spans_several_values() {
        // `[re-argued — 2026-08-14]` This was named
        // `..._spans_a_type_unstable_key` and rested on *"containment does not coerce, so a
        // single-shape predicate silently answers for one population and not the other."* Measured
        // against Postgres 18, that is backwards — `'["x"]' @> '"x"'` is TRUE, so the SCALAR probe
        // alone already spans both populations of a type-unstable key. See `PropertyOp::Contains`.
        //
        // The list survives because its real job is the one the old rationale never mentioned:
        // OR across genuinely DIFFERENT values, matching the within-field OR that `doc_type` and
        // `EdgeFilter.labels` already have. `derived_from` (an array on 112 resources, a string on
        // 21) is still the fixture, because it is the case that would have gone wrong under the
        // old reading — a caller listing only the array shape answers for 112 and silently misses
        // 21.
        let p = PropertyPredicate {
            subject: PropertySubject::Resource,
            key: "derived_from".to_string(),
            op: PropertyOp::Contains {
                values: vec![serde_json::json!("abc"), serde_json::json!(["abc"])],
            },
        };
        let PropertyOp::Contains { values } = &p.op else {
            panic!("wrong op")
        };
        assert_eq!(values.len(), 2);
    }
}
