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
    /// depending on which act carried it — which is what a `PropertySubject` tag existed to
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
/// for. `[2026-08-15]` `UnknownFilterValue` used to be raised here for exactly one thing — an
/// unrecognized `PropertySubject` — and **that reason is now gone with the type**, so nothing on
/// this struct raises it.
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
    /// `kb_properties` rows owned by the resource itself: open key space, closed operator set.
    /// AND across the list, OR within a [`PropertyOp::Contains`].
    ///
    /// **The three named fields above reach three keys; this one reaches the rest.** Sixty-seven of
    /// the seventy live property keys were narrowable by nothing on any act
    /// `[measured on prod — 2026-08-14]`.
    ///
    /// **The container is the point, and it is the same container `EdgeFilter` has.** This is where
    /// a resource property predicate lives; it moved off `ActInvocation::properties`, where the same
    /// field meant different things depending on which act carried it. Given a container the subject
    /// tag has no job, which is why the tag no longer exists.
    ///
    /// **`Contains` reads the value WHOLE — `kb_resource_properties`, never
    /// `kb_property_elements`** `[decided — 2026-08-15, Pete; 20260815000040]`. So it means exactly
    /// what [`EdgeFilter::properties`]'s `Contains` means. The element relation would silently
    /// narrow the operator: an array-shaped probe matches the whole value and matches *nothing*
    /// against an exploded element, and a `[]`-valued key is a row in the one and no rows in the
    /// other. The element view continues to serve `tags` and `facets`, whose semantics genuinely
    /// are AND-containment over elements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyPredicate>,
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

/// A property narrowing operator. CLOSED — the key space is open, the operator set is not. No
/// operator takes a fragment of a query language; all bind their values.
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
    /// `property_value <direction> $value` over jsonb's native ordering, type-guarded.
    ///
    /// **The type is inferred from the caller's bound, never declared on the operator.**
    /// `jsonb_typeof(property_value) = jsonb_typeof($value)` segments by JSON type before the
    /// comparison, so a row whose JSON type differs from the bound's is an **honest empty** rather
    /// than a type-confusion match. jsonb defines a total type ordering
    /// (`null < boolean < number < string < array < object`), so without the guard a numeric
    /// bound against a string-valued key would match **every** string row (`string > number` is
    /// true in jsonb's ordering) — a type-confusion artifact, not an answer. The guard makes each
    /// comparison run only within a homogeneous sub-population.
    ///
    /// **Per-VALUE inference, not per-key — and the distinction is the trap.** `temper-pr` is
    /// 68 string / 7 numeric on ONE key, so no per-key answer exists; each caller sends one bound
    /// with one JSON type, and the guard makes the other-type rows honest empties. A numeric bound
    /// compares the 7 numeric rows; a string bound compares the 68 string rows; neither is wrong.
    ///
    /// **Numbers stored as JSON STRINGS** that need numeric comparison are out of scope: that is a
    /// convention the key should fix (store numbers as JSON numbers), and `temper-seq` (132 numeric
    /// rows) already does. A comparison operator is not a type-coercion mechanism.
    ///
    /// `probe_count` for `Compare` is **1** — one bound, one comparison per row that carries the
    /// key — like `HasKey`, not like `Contains { values }` whose cost is `Σ|values|`. `Between` is
    /// NOT added: a closed range composes from `gte` AND `lte` via the existing AND-across-the-list,
    /// and adding it saves one probe at the cost of a second value slot and a second SQL branch.
    Compare {
        direction: OrdOp,
        value: serde_json::Value,
    },
}

/// The ordering direction for [`PropertyOp::Compare`]. A closed sub-enum, the same shape as
/// `Contains`'s `Vec` — a nested closed set inside one `PropertyOp` discriminant.
///
/// All four directions are needed: inclusivity matters for dates (*"on or after 2026-07-01"* is
/// `gte`, not `gt*). `Gt`/`Lt` are the half-open bounds; `Gte`/`Lte` are the closed ones; a
/// `Between` is `gte` AND `lte` composed through the existing AND-across-the-list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OrdOp {
    Gt,
    Gte,
    Lt,
    Lte,
}

/// A property predicate: which key, and how. **The subject is the CONTAINER it sits in** — an
/// [`EdgeFilter`] means the edge's own `kb_properties` rows, a [`ResourceFilter`] means the
/// resource's own, and nothing else has to be said.
///
/// `[2026-08-15]` Both containers now exist, so the subject-tagged variant that floated free on the
/// invocation is **deleted**, along with the `PropertySubject` tag it carried and the
/// `UnknownFilterValue` refusal that tag's open arm existed to raise. What survives is
/// [`super::ActInvocation::properties`], retyped to this struct: it is a **tombstone**, refusing
/// with a redirect rather than being removed, because `ActInvocation` carries `deny_unknown_fields`
/// and removing the field would route a stale caller into a deserializer 400 outside the
/// `ErrorBody` shape — a worse answer than the one being replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct PropertyPredicate {
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
    fn a_resource_property_predicate_names_no_subject_because_its_container_is_one() {
        // The whole argument for the container, at the type level, on the half that closed it. The
        // subject enum is DELETED — there is nowhere to put a tag, so a resource predicate cannot
        // claim to be about an edge, and `PropertySubject::Other` has no arm left to be unknown in.
        let f = ResourceFilter {
            properties: vec![PropertyPredicate {
                key: "derived_from".to_string(),
                op: PropertyOp::Contains {
                    values: vec![serde_json::json!("spec-a")],
                },
            }],
            ..Default::default()
        };
        let back: ResourceFilter =
            serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
        // The open-key slot is a THIRD thing beside the three named keys, not a spelling of one.
        assert_eq!(back.properties.len(), 1);
        assert!(back.tags.is_empty() && back.doc_type.is_empty() && back.facets.is_empty());
    }

    #[test]
    fn a_resource_filter_with_no_properties_still_serializes_to_nothing() {
        // `skip_serializing_if`, so adding the open-key slot did not start emitting an empty array
        // on every resource filter that has none.
        let f = ResourceFilter::default();
        assert_eq!(serde_json::to_string(&f).unwrap(), "{}");
        assert_eq!(serde_json::from_str::<ResourceFilter>("{}").unwrap(), f);
    }

    #[test]
    fn both_containers_carry_the_same_predicate_type_so_contains_cannot_diverge() {
        // `[2026-08-15]` The point of the ruling, asserted at the type level rather than described:
        // one `PropertyPredicate` in both containers means `contains` serializes identically for
        // both, and both fragments read `property_value @> v` — the value WHOLE. If the resource
        // half had taken the element grain, these two would still typecheck and would MEAN
        // different things, which is the divergence the container design exists to remove.
        let pred = PropertyPredicate {
            key: "derived_from".to_string(),
            op: PropertyOp::Contains {
                values: vec![serde_json::json!("spec-a")],
            },
        };
        let in_resource = ResourceFilter {
            properties: vec![pred.clone()],
            ..Default::default()
        };
        let in_edge = EdgeFilter {
            properties: vec![pred],
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&in_resource.properties).unwrap(),
            serde_json::to_value(&in_edge.properties).unwrap(),
            "one predicate type, so the wire shape cannot drift between the two containers"
        );
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
    fn has_key_contains_and_compare_are_the_whole_vocabulary() {
        // No operator takes a fragment of a query language. All three bind.
        //
        // `[2026-08-16]` Renamed from `has_key_and_contains_are_the_whole_v1_vocabulary` when
        // `Compare` joined the closed set — the "v1" framing is gone with the second addition.
        let hk = PropertyPredicate {
            key: "keywords".to_string(),
            op: PropertyOp::HasKey,
        };
        let ct = PropertyPredicate {
            key: "confidence".to_string(),
            op: PropertyOp::Contains {
                values: vec![serde_json::json!("high")],
            },
        };
        let cmp = PropertyPredicate {
            key: "date".to_string(),
            op: PropertyOp::Compare {
                direction: OrdOp::Gte,
                value: serde_json::json!("2026-07-01"),
            },
        };
        for p in [hk, ct, cmp] {
            assert_eq!(
                serde_json::from_str::<PropertyPredicate>(&serde_json::to_string(&p).unwrap())
                    .unwrap(),
                p
            );
        }
    }

    #[test]
    fn compare_serializes_internally_tagged_and_round_trips_all_four_directions() {
        // The wire shape the SQL fragment parses: `{"op":"compare","direction":"gte","value":...}`.
        // `OrdOp` is `rename_all = "snake_case"`, so `Gte` → `"gte"` (no ambiguity with `Gt`).
        for (direction, wire) in [
            (OrdOp::Gt, "gt"),
            (OrdOp::Gte, "gte"),
            (OrdOp::Lt, "lt"),
            (OrdOp::Lte, "lte"),
        ] {
            let p = PropertyPredicate {
                key: "date".to_string(),
                op: PropertyOp::Compare {
                    direction,
                    value: serde_json::json!("2026-07-01"),
                },
            };
            let serialized = serde_json::to_string(&p).unwrap();
            assert_eq!(
                serialized,
                format!(
                    r#"{{"key":"date","op":{{"op":"compare","direction":"{wire}","value":"2026-07-01"}}}}"#,
                ),
                "direction {direction:?} did not serialize to the expected wire shape"
            );
            assert_eq!(
                serde_json::from_str::<PropertyPredicate>(&serialized).unwrap(),
                p,
                "round-trip failed for direction {direction:?}"
            );
        }
    }

    #[test]
    fn a_stale_body_still_carrying_a_subject_parses_so_the_redirect_can_fire() {
        // **The tombstone's whole reason, asserted rather than assumed.** `ActInvocation` carries
        // `deny_unknown_fields` and serde short-circuits before `validate`, so deleting the field
        // would answer a stale caller with a deserializer 400 OUTSIDE `ErrorBody`. Retyping it to
        // `PropertyPredicate` — which carries no `deny_unknown_fields` — keeps the old body
        // parsing: the now-meaningless `subject` is ignored and the capability pass's redirect
        // still reaches the caller.
        let wire = r#"{"subject":"edge","key":"confidence","op":{"op":"has_key"}}"#;
        let p: PropertyPredicate =
            serde_json::from_str(wire).expect("a stale subject tag must not break the parse");
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
        let p = PropertyPredicate {
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
