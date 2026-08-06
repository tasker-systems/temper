//! `ResourceView` — the one shape a resource has.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::managed_meta::ManagedMeta;
use super::resource::{BodyStorage, IngestState};
use crate::operations::decorated_ref;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

/// Everything a resource is, except its body — one shape for every read and write
/// surface.
///
/// Replaces the six near-identical projections a caller used to have to tell apart
/// (`ResourceRow`, `ResourceDetail`, `ResourceMetaResponse`, `ResourceSummary`, the
/// search `HitIdentity`, and the inlined identity half of `ExactHit`/`WideHit`). A
/// human or agent learns this shape once and it is the shape `list`, `show`, `create`,
/// `update`, `annotate` and both search arms all answer in.
///
/// Three properties are load-bearing and each has its own test below:
///
/// 1. **The anchor is `id`, never `resource_id`.** [`super::managed_meta::ResourceMetaResponse`]
///    records why: "With two different anchor names the subset relation is unachievable."
///    Here the relation is stronger than subset — it is identity.
/// 2. **No workflow field is hoisted.** `stage`/`mode`/`effort`/`seq` were flat columns on
///    `ResourceRow`; they live in [`ManagedMeta`] under their canonical `temper-*` names and
///    nowhere else. Dropping them is lossless *because* of property 3.
/// 3. **`managed_meta` is not an `Option`.** A consumer reads `view.managed_meta.stage`
///    without first distinguishing "no meta" from "no stage".
///
/// **No `#[serde(flatten)]` anywhere**, deliberately: ts-rs cannot codegen a flattened
/// field, which is why `ResourceDetail` had to drop its `TS` derive and why the identity
/// fields on `ExactHit`/`WideHit` were hand-inlined. This type is ts-rs-exported, so the
/// constraint is structural, not stylistic.
///
/// **No `FromRow` derive, deliberately.** `ref` and `context_ref` are derived rather than
/// selected and `content` arrives from a different read, so a `query_as` into this type
/// could never be complete. `ResourceRow` carried a `FromRow` derive with **zero**
/// `query_as` call sites anywhere in the repo — decorative, and an invitation to try
/// something that cannot work. Construction is by hand, as `substrate_read` already does.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceView {
    // ---- identity -------------------------------------------------------------
    pub id: ResourceId,
    /// The decorated, self-resolving address: `sluggify(title)-<uuid>`.
    ///
    /// Not a column — derived by [`ResourceView::with_derived_refs`] from `title` + `id`.
    /// Until this task it was injected render-time by the CLI alone
    /// (`temper-cli/src/commands/resource.rs`), so MCP callers never received one even
    /// though the shipped skill instructs agents to use it.
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub title: String,
    /// Original source URL or file reference.
    ///
    /// There is deliberately no `kb_uri` beside this. `ExactHit`/`WideHit` carried one
    /// whose doc comment promised a "canonical `kb://` URI ... from `kb_resource_uri`",
    /// but `kb_resource_uri` does not exist in `migrations/` and the assembly wrote
    /// `kb_uri: self.origin_uri.clone()` (`substrate_read.rs:898,912`) — the same value
    /// under a second name and a false description. It joins `origin` (the constant
    /// `"unified"`) and `slug` (always `String::new()`) as a field this arc dropped for
    /// never having carried what it claimed.
    pub origin_uri: String,

    // ---- home: `context_*` for a context home, `cogmap_*` for a cognitive map -----
    /// `Some` for a context-homed resource, `None` when homed in a cognitive map
    /// (Surface B). Mutually exclusive with the `cogmap_*` fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb_context_id: Option<ContextId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_name: Option<String>,
    /// Slug of the home context (the natural-key half of `@owner/slug`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_slug: Option<String>,
    /// Already-sigil'd owner: `@<handle>` for profiles, `+<team-slug>` for teams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_owner_ref: Option<String>,
    /// The composed home-context address, `{context_owner_ref}/{context_slug}`.
    ///
    /// Not a column — derived by [`ResourceView::with_derived_refs`]. `None` for a
    /// cogmap-homed resource, which has no context half; never a half-formed ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_ref: Option<String>,
    /// Set when the resource is homed in a cognitive map. Mutually exclusive with
    /// the `context_*` fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cogmap_id: Option<Uuid>,
    /// Display name of the home cognitive map. `Some` iff `cogmap_id` is `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cogmap_name: Option<String>,

    // ---- type, attribution, lifecycle ------------------------------------------
    pub doc_type_name: String,
    pub owner_handle: String,
    pub owner_profile_id: ProfileId,
    pub originator_profile_id: ProfileId,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,

    // ---- body state (never the body itself, except via `content`) ---------------
    /// SHA-256 hash of the resource body, from `kb_resource_manifests`. `None` when no
    /// manifest row exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_hash: Option<String>,
    /// Are all the bytes here? `Option` purely for version skew — the column is
    /// `NOT NULL` server-side, so `None` means the server predates W2 PR 1. Do not
    /// read `None` as "incomplete".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest_state: Option<IngestState>,
    /// Do the bytes read back exactly, or only approximately? `Option` purely for
    /// version skew, as with `ingest_state`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_storage: Option<BodyStorage>,

    // ---- the two metadata tiers, and the body ----------------------------------
    /// Typed managed (`temper-*`) frontmatter — the closed Property vocabulary.
    ///
    /// **Not an `Option`.** This is what makes dropping the hoisted `stage`/`mode`/
    /// `effort`/`seq` columns lossless: the values did not go away, they went home. A
    /// resource with no managed metadata carries an all-`None` `ManagedMeta`, which
    /// serializes as `{}` — present and empty, never absent.
    #[serde(default)]
    pub managed_meta: ManagedMeta,
    /// Open (user-defined) frontmatter — the free-form tier, intentionally untyped.
    /// Absent means the `open-meta` section was not requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
    /// The reconstructed markdown body — the `body` section.
    ///
    /// Absent means **not requested**, never "empty body". An empty body is
    /// `Some(String::new())`, which serializes as `""`; the two are distinguishable on
    /// the wire and after a round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

impl ResourceView {
    /// Fill `ref` and `context_ref` from the columns already on the view.
    ///
    /// The single derivation for both. They are not columns, so every construction site
    /// would otherwise re-derive them — which is how the CLI came to be the only surface
    /// emitting a `ref` at all. `ref` goes through
    /// [`decorated_ref`] rather than re-deriving
    /// the slug, so there is exactly one `sluggify` in the address path.
    ///
    /// `context_ref` is `Some` only when **both** halves are present: a cogmap-homed
    /// resource gets `None` rather than a ref with an empty side.
    #[must_use]
    pub fn with_derived_refs(mut self) -> Self {
        self.r#ref = decorated_ref(&self.title, self.id);
        self.context_ref = match (&self.context_owner_ref, &self.context_slug) {
            (Some(owner_ref), Some(slug)) => Some(format!("{owner_ref}/{slug}")),
            _ => None,
        };
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_view() -> ResourceView {
        ResourceView {
            id: ResourceId::from(Uuid::nil()),
            r#ref: String::new(),
            title: "A Node".to_string(),
            origin_uri: String::new(),
            kb_context_id: None,
            context_name: None,
            context_slug: None,
            context_owner_ref: None,
            context_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            doc_type_name: "concept".to_string(),
            owner_handle: "someone".to_string(),
            owner_profile_id: ProfileId::from(Uuid::nil()),
            originator_profile_id: ProfileId::from(Uuid::nil()),
            is_active: true,
            created: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            updated: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            body_hash: None,
            ingest_state: Some(IngestState::Complete),
            body_storage: Some(BodyStorage::Derived),
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            content: None,
        }
    }

    /// The anchor is `id`, never `resource_id`.
    ///
    /// `managed_meta.rs:102-105` records why: with two different anchor names the
    /// subset relation between the meta projection and the full read is
    /// unachievable. `ResourceView` is what makes that relation an identity
    /// instead, so the anchor name is load-bearing for the whole convergence.
    #[test]
    fn anchor_is_id_not_resource_id() {
        let v = serde_json::to_value(sample_view()).expect("serialize");

        assert!(v.get("id").is_some(), "anchor is `id`: {v}");
        assert!(
            v.get("resource_id").is_none(),
            "`resource_id` is the other anchor name and must not appear: {v}"
        );
    }

    /// No workflow field is hoisted to the top level — `stage`/`mode`/`effort`/`seq`
    /// live in `managed_meta` under their `temper-*` names and nowhere else.
    ///
    /// The second half of this test is what makes dropping them lossless rather than
    /// lossy: every value is still reachable, one level down.
    #[test]
    fn no_workflow_field_is_hoisted() {
        let view = ResourceView {
            managed_meta: ManagedMeta {
                stage: Some("doing".to_string()),
                mode: Some("build".to_string()),
                effort: Some("small".to_string()),
                seq: Some(7),
                ..ManagedMeta::default()
            },
            ..sample_view()
        };
        let v = serde_json::to_value(&view).expect("serialize");
        let obj = v.as_object().expect("object");

        for hoisted in ["stage", "mode", "effort", "seq"] {
            assert!(
                !obj.contains_key(hoisted),
                "`{hoisted}` is a managed_meta key and must not be hoisted to the top level: {v}"
            );
        }

        // Nothing was lost — each value is still carried, under its canonical name.
        assert_eq!(v["managed_meta"]["temper-stage"], "doing");
        assert_eq!(v["managed_meta"]["temper-mode"], "build");
        assert_eq!(v["managed_meta"]["temper-effort"], "small");
        assert_eq!(v["managed_meta"]["temper-seq"], 7);
    }

    /// `managed_meta` is always present, even when every field inside it is `None`.
    ///
    /// This is the conjunct that makes dropping the hoisted fields safe: a consumer
    /// reads `view.managed_meta.stage` unconditionally and never has to distinguish
    /// "no managed_meta" from "no stage". An `Option<ManagedMeta>` with
    /// `skip_serializing_if` would reintroduce exactly that distinction.
    #[test]
    fn managed_meta_is_always_present() {
        let view = sample_view();
        assert_eq!(
            view.managed_meta,
            ManagedMeta::default(),
            "precondition: every managed field is None"
        );

        let v = serde_json::to_value(&view).expect("serialize");
        let obj = v.as_object().expect("object");

        assert!(
            obj.contains_key("managed_meta"),
            "an all-None ManagedMeta still emits the key: {v}"
        );
        assert!(
            v["managed_meta"].is_object(),
            "it emits an object (empty), not null: {v}"
        );

        let back: ResourceView = serde_json::from_value(v).expect("deserialize");
        assert_eq!(back.managed_meta, ManagedMeta::default());
    }

    /// An absent body and an empty body are different answers, so they get different
    /// wire representations: `None` omits the key (the `body` section was not
    /// requested), `Some("")` emits `""` (it was requested, and the body is empty).
    #[test]
    fn body_absent_is_distinguishable_from_body_empty() {
        let absent = serde_json::to_value(ResourceView {
            content: None,
            ..sample_view()
        })
        .expect("serialize");
        assert!(
            !absent.as_object().expect("object").contains_key("content"),
            "content: None means the body section was not requested — omit the key: {absent}"
        );

        let empty = serde_json::to_value(ResourceView {
            content: Some(String::new()),
            ..sample_view()
        })
        .expect("serialize");
        assert_eq!(
            empty["content"], "",
            "content: Some(\"\") is a requested, empty body: {empty}"
        );

        // And the distinction survives a round-trip, which is what a consumer relies on.
        let back: ResourceView = serde_json::from_value(absent).expect("deserialize absent");
        assert!(back.content.is_none());
        let back: ResourceView = serde_json::from_value(empty).expect("deserialize empty");
        assert_eq!(back.content.as_deref(), Some(""));
    }

    /// The two address fields are derived from the columns beside them, in one place.
    ///
    /// Beyond the plan's four conjuncts: `with_derived_refs` is new logic introduced
    /// by this task (`ref` and `context_ref` are not columns — they were injected
    /// render-time at `temper-cli/src/commands/resource.rs:68-108` before this), so it
    /// carries its own witness.
    #[test]
    fn derived_refs_are_computed_from_the_columns_beside_them() {
        let id = ResourceId::from(
            Uuid::parse_str("019e84ab-26ba-7560-9d34-c60d74a9fbe2").expect("uuid"),
        );
        let homed = ResourceView {
            id,
            title: "My Task".to_string(),
            context_owner_ref: Some("@me".to_string()),
            context_slug: Some("notes".to_string()),
            ..sample_view()
        }
        .with_derived_refs();

        assert_eq!(
            homed.r#ref, "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "ref is sluggify(title)-<uuid>, via the one decorated_ref"
        );
        assert_eq!(homed.context_ref.as_deref(), Some("@me/notes"));

        // A cogmap-homed resource has no context half, so it has no context_ref —
        // never an empty or half-formed one.
        let cogmapped = ResourceView {
            id,
            title: "My Task".to_string(),
            context_owner_ref: None,
            context_slug: None,
            cogmap_id: Some(Uuid::nil()),
            ..sample_view()
        }
        .with_derived_refs();

        assert_eq!(
            cogmapped.r#ref,
            "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2"
        );
        assert_eq!(cogmapped.context_ref, None);
    }

    /// The serialized key is `ref`, not Rust's raw-identifier spelling.
    #[test]
    fn ref_serializes_under_the_bare_name() {
        let v = serde_json::to_value(sample_view()).expect("serialize");
        let obj = v.as_object().expect("object");
        assert!(obj.contains_key("ref"), "{v}");
        assert!(!obj.contains_key("r#ref"), "{v}");
    }
}
