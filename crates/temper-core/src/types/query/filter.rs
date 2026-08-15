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
///
/// # Every field here constrains a HOP, and that is why they live in a container
///
/// `[decided — 2026-08-14, Pete]` *A narrowing that can be expressed as a set must be an act. A
/// narrowing that cannot be a set belongs to the act whose semantics it constrains.* An edge
/// predicate has no set-shaped substitute: binding a walk by *"nodes that participate in an edge
/// matching P"* admits a node because it has a matching edge **somewhere** and then walks it through
/// a different, non-matching one — a different question, returning plausible rows and looking like
/// it narrowed. So these constrain the traversal from inside it, and the only act that traverses an
/// edge is `follow-from`.
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
    /// `kb_properties` rows owned by the edge itself: open key space, closed operator set.
    /// AND across the list, OR within a [`PropertyOp::Contains`].
    ///
    /// **This is where an edge property predicate lives, and the container is the point.** It moved
    /// off [`super::ActInvocation::properties`], where the same field meant different things
    /// depending on which act carried it — which is what a [`PropertySubject`] tag existed to
    /// disambiguate. Given a container the tag has no job; the subject is the container.
    ///
    /// **Zero edge-owned properties exist in this deployment** `[measured on prod — 2026-08-14]`,
    /// and the storage has admitted them since the schema's first migration (`kb_properties.
    /// owner_table` includes `'kb_edges'`, whose DDL comment has said *"§4a edges carry facets"*
    /// throughout) with a shipped write path `[verified — 20260727000030]`. So this slot narrows
    /// nothing today by data rather than by design.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyPredicate>,
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

/// What a [`SubjectedPropertyPredicate`] addresses.
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

/// A property predicate: which key, and how. **The subject is the CONTAINER it sits in** — an
/// [`EdgeFilter`] means the edge's own `kb_properties` rows, and nothing else has to be said.
///
/// `[2026-08-15]` This name previously belonged to the subject-tagged variant that floats free on
/// the invocation, now [`SubjectedPropertyPredicate`]. The rename runs this direction so that the
/// transitional type carries the transitional name: when the open-key resource half lands and
/// deletes it, nothing is renamed a second time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct PropertyPredicate {
    pub key: String,
    pub op: PropertyOp,
}

/// A property predicate that names its own subject, because it sits on the invocation rather than
/// in a container.
///
/// The subject is CARRIED, never inferred, because inference is ambiguous exactly where it matters:
/// a `follow-from` stage walks edges and produces resources, so "the properties of this stage's
/// subject" has two answers.
///
/// # This type is transitional, and every arm of it is refused today
///
/// `[decided — 2026-08-14, Pete]` Both halves get containers and this type disappears with
/// [`PropertySubject`]. The edge half landed on 2026-08-15 — an edge predicate now belongs in
/// [`EdgeFilter::properties`], and the refusal for a `subject: edge` predicate here REDIRECTS
/// there rather than merely declining. The resource half is task
/// `01a00502-a774-7001-b5b2-0ce462158f1c`, which deletes this type, [`PropertySubject`], its
/// `Other(String)` arm and the `UnknownFilterValue` refusal together.
///
/// It survives in the meantime **so that a stale caller gets a named refusal that says where the
/// capability went**. Deleting the field instead would route the same request into
/// `ActInvocation`'s `deny_unknown_fields`, and serde short-circuits before `validate` — so the
/// caller would receive a deserializer 400 outside the `ErrorBody` shape, which is a worse answer
/// than the one being replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct SubjectedPropertyPredicate {
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
            properties: vec![],
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
    fn an_edge_property_predicate_names_no_subject_because_its_container_is_one() {
        // The whole argument for the container, at the type level: there is nowhere to put a
        // subject tag, so an edge predicate cannot claim to be about a resource.
        let f = EdgeFilter {
            edge_kinds: vec![EdgeKind::LeadsTo],
            labels: vec![],
            properties: vec![PropertyPredicate {
                key: "confidence".to_string(),
                op: PropertyOp::Contains {
                    values: vec![serde_json::json!("high")],
                },
            }],
        };
        let back: EdgeFilter = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
        // And the three axes stay separable — a property predicate is not a label by another name.
        assert_eq!(back.properties.len(), 1);
        assert!(back.labels.is_empty());
    }

    #[test]
    fn an_edge_filter_with_no_properties_still_serializes_to_nothing() {
        // `skip_serializing_if`, so adding a third axis did not start emitting an empty array on
        // every edge filter that has none — the property `an_empty_filter_serializes_to_nothing`
        // asserts for `ResourceFilter`, now owed by this type too.
        let f = EdgeFilter::default();
        assert_eq!(serde_json::to_string(&f).unwrap(), "{}");
        assert_eq!(serde_json::from_str::<EdgeFilter>("{}").unwrap(), f);
    }

    #[test]
    fn has_key_and_contains_are_the_whole_v1_vocabulary() {
        // No operator takes a fragment of a query language. Both bind.
        let hk = SubjectedPropertyPredicate {
            subject: PropertySubject::Resource,
            key: "keywords".to_string(),
            op: PropertyOp::HasKey,
        };
        let ct = SubjectedPropertyPredicate {
            subject: PropertySubject::Edge,
            key: "confidence".to_string(),
            op: PropertyOp::Contains {
                values: vec![serde_json::json!("high")],
            },
        };
        for p in [hk, ct] {
            assert_eq!(
                serde_json::from_str::<SubjectedPropertyPredicate>(
                    &serde_json::to_string(&p).unwrap()
                )
                .unwrap(),
                p
            );
        }
    }

    #[test]
    fn the_subjected_predicates_wire_shape_is_unchanged_by_the_rename() {
        // The rename is a RUST name. `ActInvocation.properties` still parses exactly what it parsed
        // before, which is what keeps a stale caller reaching the named refusal that redirects it
        // rather than `deny_unknown_fields`'s deserializer 400.
        // Nested, not flat: `PropertyOp` is internally tagged and sits in a field named `op`.
        let wire = r#"{"subject":"edge","key":"confidence","op":{"op":"has_key"}}"#;
        let p: SubjectedPropertyPredicate = serde_json::from_str(wire).expect("unchanged shape");
        assert_eq!(p.subject, PropertySubject::Edge);
        assert_eq!(p.key, "confidence");
        assert_eq!(p.op, PropertyOp::HasKey);
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
        let p = SubjectedPropertyPredicate {
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
