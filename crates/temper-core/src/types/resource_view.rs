//! `ResourceView` — the one shape a resource has.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::managed_meta::ManagedMeta;
use super::resource::{BodyStorage, IngestState};
use crate::error::TemperError;
use crate::refs::decorated_ref;
use crate::types::ids::{ContextId, ProfileId, ResourceId};

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
/// 1. **The anchor is `id`, never `resource_id`.** The retired `ResourceMetaResponse`
///    recorded why: "With two different anchor names the subset relation is unachievable."
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
// `export_to` is `resource_view.ts`, NOT the `resource.ts` this type used to share with
// `ResourceRow`. `cargo make generate-ts-types` runs one `cargo test -p <crate>` per crate in
// sequence into a single output directory (tools/cargo-make/main.toml:332), and ts-rs truncates
// a target file on each process's first write. So a type survives the LAST crate's pass only if
// that crate exports it directly or reaches it as a dependency. temper-workflow runs after
// temper-core and reaches `BodyStorage`/`IngestState`, so those two are safe to leave in
// `resource.ts` — its pass re-emits them verbatim. When this type was added, nothing in
// temper-workflow reached `ResourceView`, and sharing the file silently deleted it (measured: 122
// lines, with `generate-ts-types` still exiting 0). `ResourceListResponse.rows` is
// `Vec<ResourceView>` now, so that reachability has changed — but do NOT read that as licence to
// move this back into `resource.ts`. Reachability is a property of whatever field types happen to
// exist on the later crate's exported set; it is not a property anything asserts, so it can be
// removed by an unrelated refactor with no gate saying so. A file only temper-core writes cannot
// be clobbered, whatever temper-workflow does or stops doing.
// A `//` comment, not a doc comment: utoipa renders doc comments into `openapi.json`, and a
// build-system note is not part of the wire contract.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource_view.ts"))]
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
    /// The display name of this resource's home — its context name, or its cognitive-map name
    /// when cogmap-homed. `None` only if neither is set (should not occur).
    ///
    /// The single accessor for the `context_* | cogmap_*` mutual exclusion, carried over from
    /// `ResourceRow::home_display` unchanged: surfaces apply their own placeholder for the
    /// `None` case rather than each re-deriving the fallback chain.
    #[must_use]
    pub fn home_display(&self) -> Option<&str> {
        self.context_name.as_deref().or(self.cogmap_name.as_deref())
    }

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

// ─── Sections ───────────────────────────────────────────────────────────────

/// One addable or removable part of a [`ResourceView`].
///
/// Replaces `--meta-only`, which conflated two axes because the two commands differ in
/// what they otherwise return: on `show` it means *drop the body*
/// (`temper-cli/src/cli.rs:571-575` — "Show everything except the body"), on `list` it
/// means *add both meta tiers* (`:537-542` — "each row carries both the managed and open
/// meta tiers ... on top of the usual row fields"). Once both commands answer in one
/// shape, that flag has two meanings and no coherent one.
///
/// Naming the parts lets a caller add or remove each independently — including
/// `show --with edges --without body`, which is **unreachable today**: `meta_only`
/// carries `conflicts_with = "edges"` (`temper-cli/src/cli.rs:574`), so the present
/// design needs a rule to forbid a combination that is merely useful.
///
/// The managed tier is deliberately **not** a section: [`ResourceView::managed_meta`] is
/// always present, which is what makes dropping the hoisted workflow fields lossless.
///
/// The names are **kebab-case** (`open-meta`), not the `open_meta` of the field the
/// section fills. They are values a human or agent types after `--with`, and the CLI's
/// vocabulary is kebab throughout; the `snake_case` enums in [`super::resource`]
/// ([`IngestState`], [`BodyStorage`]) are DB column values, which these are not. Serde's
/// rename and [`FromStr`] are two independent mappings, so `section_names_are_kebab_case`
/// asserts both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceSection {
    /// The reconstructed markdown body — [`ResourceView::content`].
    Body,
    /// The open (user-defined) frontmatter tier — [`ResourceView::open_meta`].
    OpenMeta,
    /// The resource's graph edges, fetched alongside the view rather than carried on it.
    Edges,
}

impl ResourceSection {
    /// Every section, in the order a refusal lists them.
    ///
    /// The **one** enumeration of the set: [`FromStr`] matches over it and the refusal
    /// message is built from it, so a section cannot be accepted without also being
    /// named. Public so a surface's help text lists the same three rather than
    /// hand-copying them into a fourth place that then drifts.
    pub const ALL: [Self; 3] = [Self::Body, Self::OpenMeta, Self::Edges];

    /// The canonical wire/CLI name — the same string serde emits.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::OpenMeta => "open-meta",
            Self::Edges => "edges",
        }
    }
}

impl fmt::Display for ResourceSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ResourceSection {
    type Err = TemperError;

    /// Parse a section name. Unknown input is refused with the whole valid set named —
    /// the caller is usually an agent that guessed, with nothing else to consult
    /// mid-call.
    ///
    /// The error is [`TemperError::BadRequest`], not the [`TemperError::Project`] that
    /// [`crate::refs::parse_ref`] returns for the same class of failure:
    /// `Project` maps to `ApiError::Internal` (`temper-services/src/error.rs:242`),
    /// and a mistyped section name is the caller's `400`, never the server's `500`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|section| section.as_str() == s)
            .ok_or_else(|| {
                TemperError::BadRequest(format!(
                    "unknown section {s:?} (expected one of: {})",
                    Self::ALL.map(Self::as_str).join(", ")
                ))
            })
    }
}

/// The set of sections a caller asked for.
///
/// A [`BTreeSet`] rather than a `HashSet` so iteration is deterministic: this set gets
/// rendered — into a refusal, a log line, or a serialized request — and an order that
/// varies run to run makes those unreadable and undiffable for no gain at a size of
/// three.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionSet(BTreeSet<ResourceSection>);

impl SectionSet {
    /// Was this section asked for?
    #[must_use]
    pub fn contains(&self, section: ResourceSection) -> bool {
        self.0.contains(&section)
    }

    /// Parse the `sections` query parameter — a comma-separated list of section names.
    ///
    /// A CSV string rather than a `Vec<ResourceSection>` for the same reason `tags` and
    /// `cogmap_ids` are CSV on `ResourceListParams`: the list endpoint is a GET whose params
    /// ride the query string, and serde_urlencoded does not encode sequences. Section names
    /// contain no comma by construction ([`ResourceSection::ALL`]), so the split is total.
    ///
    /// Each piece is trimmed; an empty piece is dropped rather than refused, so `""`,
    /// `"body,"` and `"body, open-meta"` all mean what they look like. An unknown name is
    /// refused through [`ResourceSection::from_str`], which names the whole valid set — this
    /// is the one place a caller's typo is caught, and it must be recoverable from the
    /// message alone.
    pub fn parse_csv(csv: &str) -> Result<Self, TemperError> {
        csv.split(',')
            .map(str::trim)
            .filter(|piece| !piece.is_empty())
            .map(ResourceSection::from_str)
            .collect()
    }

    /// Render as the `sections` query parameter — the inverse of [`SectionSet::parse_csv`].
    ///
    /// `None` for the empty set, because the wire field is an `Option<String>` and "asked
    /// for no sections" is the parameter being *absent*, not present-and-empty. It lives
    /// here rather than at the one call site so the two directions stay next to each other:
    /// a caller that renders the set by hand renders it from `contains` checks, which means
    /// adding a section to [`ResourceSection::ALL`] leaves that caller silently dropping it.
    #[must_use]
    pub fn to_csv(&self) -> Option<String> {
        if self.0.is_empty() {
            return None;
        }
        Some(
            self.0
                .iter()
                .map(|section| section.as_str())
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}

impl FromIterator<ResourceSection> for SectionSet {
    fn from_iter<I: IntoIterator<Item = ResourceSection>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
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

    /// Section names are kebab-case on **both** wire surfaces.
    ///
    /// `open-meta` is the whole reason this test exists. The Rust variant is `OpenMeta`
    /// and the field it names is `open_meta`, so either of the two plausible-looking
    /// spellings would serialize "sensibly" while being the wrong string for
    /// `--with open-meta` to accept. Serde's rename table and [`FromStr`] are two
    /// independent mappings, so both are asserted in the same loop: a drift between them
    /// is what a caller experiences as "the documented name is not the accepted name".
    #[test]
    fn section_names_are_kebab_case() {
        for (section, name) in [
            (ResourceSection::Body, "body"),
            (ResourceSection::OpenMeta, "open-meta"),
            (ResourceSection::Edges, "edges"),
        ] {
            assert_eq!(
                serde_json::to_value(section).expect("serialize"),
                serde_json::Value::String(name.to_string()),
                "{section:?} serializes as `{name}`"
            );
            assert_eq!(
                name.parse::<ResourceSection>().expect("parse"),
                section,
                "`{name}` parses back to {section:?}"
            );
        }

        // The two spellings a reader would otherwise reach for are not accepted — the
        // field name and the Rust variant name are both wrong on the wire.
        assert!("open_meta".parse::<ResourceSection>().is_err());
        assert!("openMeta".parse::<ResourceSection>().is_err());
    }

    /// A rejected name is rejected with the whole valid set in the message.
    ///
    /// The caller here is usually an agent that guessed, and it has no schema to consult
    /// mid-call — so the refusal has to be sufficient on its own to get the next attempt
    /// right. Naming only *what was wrong* leaves it guessing a second time.
    #[test]
    fn unknown_section_name_is_rejected_naming_the_valid_set() {
        let err = "openMeta"
            .parse::<ResourceSection>()
            .expect_err("`openMeta` is not a section name");
        let msg = err.to_string();

        for valid in ["body", "open-meta", "edges"] {
            assert!(
                msg.contains(valid),
                "the refusal must name `{valid}` so the caller can recover from it alone: {msg}"
            );
        }
        assert!(
            msg.contains("openMeta"),
            "and it repeats the rejected input back: {msg}"
        );
    }

    /// [`SectionSet::contains`] answers for a member and for a non-member.
    ///
    /// New logic introduced by this task, so it carries its own witness (the precedent
    /// `with_derived_refs` set above). The negative half is the load-bearing one: every
    /// caller of this asks "was this section requested?", and a set that answered `true`
    /// for everything would read as correct on the `--with` path and silently defeat
    /// `--without`.
    #[test]
    fn section_set_contains_only_its_members() {
        let set: SectionSet = [ResourceSection::Body, ResourceSection::Edges]
            .into_iter()
            .collect();

        assert!(set.contains(ResourceSection::Body));
        assert!(set.contains(ResourceSection::Edges));
        assert!(
            !set.contains(ResourceSection::OpenMeta),
            "a section that was never added is not in the set"
        );
    }

    /// [`SectionSet::to_csv`] round-trips through [`SectionSet::parse_csv`], and the empty
    /// set renders as an absent parameter rather than an empty one.
    ///
    /// The round-trip is the assertion that matters: it is what lets a CLI hand the server
    /// a set it built from `--with`/`--without` without either end restating the vocabulary.
    #[test]
    fn section_set_csv_round_trips_and_the_empty_set_is_absent() {
        let set: SectionSet = [ResourceSection::OpenMeta, ResourceSection::Body]
            .into_iter()
            .collect();

        let csv = set.to_csv().expect("a non-empty set renders");
        assert_eq!(
            SectionSet::parse_csv(&csv).expect("round-trip parses"),
            set,
            "to_csv and parse_csv must be inverses: {csv}"
        );

        assert_eq!(
            SectionSet::default().to_csv(),
            None,
            "the empty set is an ABSENT `sections` param, never `sections=`"
        );
    }
}
