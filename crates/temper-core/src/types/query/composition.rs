//! The composition envelope: an ordered list of stages plus the things that ride alongside them.
//!
//! The PRINCIPAL is deliberately absent. Visibility applies inside each act's execution — one
//! known application point per stage — and jaq reshapes what visibility admitted without ever
//! seeing the credential. There is no field here for it, by construction.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::envelope::ActInvocation;
use super::stage::{StageInput, StageName};
use crate::types::graph::EdgeKind;
use crate::types::resource_view::ResourceSection;

/// One find act's question: its text, and the caller's vector when there is one.
///
/// `[2026-08-12]` This line read *"computed once at composition start and threaded to every
/// stage"* — the envelope-placement claim spec ⟨7⟩ retired. It is corrected rather than left
/// standing because **this doc comment IS a published schema description**
/// (`tests/fixtures/query/intention.schema.json`), so a stale sentence here is a lie shipped to
/// every client that reads the contract.
///
/// **Its absence refuses, and that is about the QUESTION, not the vector.** A find stage with no
/// intention has no words to search for, so it comes back `MissingIntention`. That refusal is
/// forced rather than chosen: `find-exact` sources its query *text* from here — it becomes
/// `p_query` — and there is nowhere else to get it.
///
/// An absent EMBEDDING is a different absence and does **not** refuse. The CLI can embed; the ruby
/// gem, the TypeScript package and MCP structurally cannot, so refusing a vector search for want
/// of a precomputed vector would deny this surface to every non-CLI client. The server embeds when
/// none arrives, exactly as `/api/search` already does, and only a FAILED embed refuses — as
/// [`super::disposition::RefusalReason::EmbeddingUnavailable`], the one runtime refusal in the
/// contract. `[decided — 2026-08-08, Pete]`
/// **This is a per-STAGE field, carried by [`super::envelope::ActInvocation`].** `[decided —
/// 2026-08-12, Pete]`, spec ⟨7⟩. It sat on the composition envelope until then, which meant a
/// composition could ask exactly ONE question: every find stage in a DAG interrogated the same
/// string, and *"find A, find B, intersect them"* was inexpressible. That placement was never
/// ruled — it entered as a first-person commit paragraph and hardened into a test name.
///
/// **No `Eq`, only `PartialEq`** — [`Self::embedding`] holds `f32`. Same reason
/// [`super::envelope::StageResult`] derives neither, one derive milder: equality on a vector of
/// floats is well-defined enough for a test, total equality is not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Intention {
    /// The question, in the caller's own words.
    ///
    /// At most [`MAX_INTENTION_QUERY_BYTES`] bytes of it, refused as
    /// [`super::disposition::RefusalReason::IntentionTooLong`].
    // `max_length` is load-bearing for the same reason `max_items` is on `stages` below: it is
    // what makes the refusal legal in the SHAPE pass, where a client may raise it against a
    // server it does not share a binary with. See the comment on that field.
    //
    // **`max_length` counts CHARACTERS in JSON Schema and this cap counts BYTES**, and the two
    // agree only on ASCII. The divergence is in the safe direction and is stated rather than
    // papered over: a plan the contract admits on its character count can still be refused on its
    // byte count, never the reverse, because a UTF-8 string is at least as many bytes as
    // characters. Publishing the byte bound as a character bound therefore promises LESS than the
    // server admits, which is the direction a stale client may be wrong in.
    #[cfg_attr(feature = "web-api", schema(max_length = 4096))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 4096)))]
    pub query: String,
    /// The query vector, when the caller computed one. Mirrors `SearchParams.embedding`: the CLI
    /// links temper-ingest and embeds locally, which is faster than making the server do it; the
    /// ruby gem, the TypeScript package and MCP structurally cannot, so the server embeds on their
    /// behalf and its absence is not a refusal.
    ///
    /// **It rides beside the text it was computed FROM, and that pairing is the point.** At
    /// composition level a vector and its query could drift apart; here they cannot.
    ///
    /// This never reaches a response: [`super::trace::CompositionTrace`] carries only `stages` and
    /// echoes no intention. Should a trace ever carry one, that stops being incidental and becomes
    /// a constraint — a 768-float array must not serialize back to the caller.
    ///
    /// **Exactly [`MAX_EMBEDDING_DIM`] floats, every value finite, norm within
    /// [`MIN_EMBEDDING_NORM`]..[`MAX_EMBEDDING_NORM`]**, refused as
    /// [`super::disposition::RefusalReason::MalformedEmbedding`]. `[added — 2026-08-28, found in
    /// review]` This carried no bound of any kind, which made it the largest unbounded field on the
    /// contract: a million floats on one stage is 4 MB that validates cleanly, and there are
    /// [`MAX_STAGES`] stages. A wrong-sized vector also reached pgvector and came back as an
    /// **opaque 500** — the caller told nothing, in the door whose promise is a typed refusal.
    /// `[widened — 2026-09-02]` Length alone still let impossible values through: a NaN poisons
    /// every cosine computed from it, and the all-zero vector's cosine is 0/0 — both surfaced as
    /// driver errors behind the same opaque 500. The norm window is wide enough that any
    /// consistently scaled direction passes; only values that are not vectors for this space at
    /// all are refused.
    ///
    /// **Published as a min AND a max, because the check is an equality.** `[corrected —
    /// 2026-08-28, found in review]` Publishing only the maximum stated half the rule: a 384-float
    /// vector cleared every generated client and was then refused by the server, which is exactly
    /// the gap the shape pass exists to close — a client must be able to refuse what the server
    /// would refuse. The two bounds are the same number because a vector of any other length is
    /// not a large question, it is a vector for a different space. The norm window is published in
    /// the field's description text rather than as numeric constraints: a client that scales
    /// legitimately is not an error the bounds are for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-api", schema(min_items = 768, max_items = 768))]
    #[cfg_attr(feature = "mcp", schemars(length(min = 768, max = 768)))]
    pub embedding: Option<Vec<f32>>,
}

/// One stage whose rows come back, and how much of each row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ReturnSpec {
    pub stage: StageName,
    /// Which sections to hydrate onto each row, in the SAME vocabulary `temper resource show
    /// --with` uses. Empty means the kind's default projection.
    ///
    /// Replaces `fields: Vec<String>`, which promised field-level subselection over a projection,
    /// had nothing implementing it, and duplicated a vocabulary that already works.
    ///
    /// **This door admits a subset, and the rest are REFUSED rather than unsupported** — see
    /// [`Self::ADMITTED_SECTIONS`]. The refusal lands at validation rather than at deserialization
    /// on purpose: `/api/query` promises every refusal in one response, and a serde failure
    /// short-circuits before validation runs, so a caller with four problems would learn about one
    /// of them in a deserializer's vocabulary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub with: Vec<ResourceSection>,
}

impl ReturnSpec {
    /// The hydration sections `/api/query` offers, and therefore the only ones it accepts.
    ///
    /// Per-door subsetting of one shared vocabulary, exactly as [`ResourceSection::LIST`] already
    /// does for `list` — not a second enum. Adding a section later is a change to this const.
    ///
    /// [`ResourceSection::Body`] is refused rather than merely absent. `fill_sections` records
    /// that the body arm "is an N+1 by construction" and that what keeps it honest is "the door,
    /// not the loop"; `/api/query` is a new door and is narrow from its first line. Ask `show`.
    ///
    /// [`ResourceSection::Edges`] is refused because edge listing has its own commands, and
    /// `follow-from` plus an `EdgeFilter` walk and filter on edges without returning them.
    pub const ADMITTED_SECTIONS: [ResourceSection; 1] = [ResourceSection::OpenMeta];
}

/// Which stages come back, and how much of each row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct OutcomeDeclaration {
    // There is deliberately no `description`. It was a required prose string — "a composition's
    // pocket outcome register" — which is goal-authoring discipline leaked into a wire contract.
    // Nobody should have to write a sentence about what being served looks like in order to run a
    // query. The register discipline belongs to goals, which are resources, not to request bodies.
    /// The stages whose rows are hydrated and returned. DECLARED, not inferred from graph shape:
    /// inferring from out-degree zero makes returning an intermediate impossible without a dummy
    /// consumer, and means adding a downstream stage silently stops returning what you used to get
    /// back. The composition's produced kind(s) are DERIVED from these, replacing the old single
    /// `produces` field which could only ever be right for a one-arm plan.
    // Same reason as [`CombineNode::inputs`]: `validate` refuses an empty list as `no_returns`, and
    // a contract that admits one describes a request that cannot succeed.
    #[cfg_attr(feature = "web-api", schema(min_items = 1))]
    pub returns: Vec<ReturnSpec>,
}

/// A set combinator's operation. Every member takes two or more inputs; no act does, which is why
/// a combinator is its own node kind rather than an act invocation.
///
/// **Two of the three are commutative and one is not, and that split is the whole of what
/// [`CombineNode::inputs`] means.** For `union` and `intersect` the input list is a SET — reordering
/// it cannot change the answer, and arity above two is a fold with nothing to say about order. For
/// `difference` it is an ordered PAIR: `A − B ≠ B − A`, so `inputs[0]` is the minuend and
/// `inputs[1]` the subtrahend, and a third input is refused rather than folded (see
/// [`CombineNode::inputs`]).
///
/// The set is CLOSED and adding a member is a contract change — the same rule §12 states for
/// `PropertyOp`. `union` and `intersect` were chosen as a pair; `difference` joins them because the
/// question *"in A, and in neither B nor C"* is set-expressible, and `EdgeFilter`'s rule is that a
/// narrowing expressible as a set must be an act rather than a predicate. That is why this is not a
/// `PropertyOp::LacksKey`, which is the tempting shape — it sits beside `HasKey` and reads
/// naturally, and it would inherit the open-key type hazard for a question that has no type
/// question at all. `[decided — 2026-08-15, Pete]`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CombineOp {
    Union,
    Intersect,
    /// `inputs[0]` minus `inputs[1]`. Exactly two inputs — see [`CombineNode::inputs`].
    Difference,
}

impl CombineOp {
    /// Does reordering this op's inputs change its answer?
    ///
    /// Exists so the arity rule and the emitter cannot disagree about which ops are ordered: both
    /// read this rather than each matching on the variant. A new op declares its answer here or
    /// fails to compile.
    pub const fn is_ordered(self) -> bool {
        match self {
            CombineOp::Union | CombineOp::Intersect => false,
            CombineOp::Difference => true,
        }
    }
}

/// A set combination over two-or-more upstream stages. Its own node kind because no act takes more
/// than one input, so modelling it as an act would lie about what an act is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CombineNode {
    pub name: StageName,
    pub op: CombineOp,
    /// Two or more. One input is not a combination; validation refuses it (beat B).
    ///
    /// **For `difference` it is exactly two, and it is ORDERED**: `inputs[0]` minus `inputs[1]`.
    /// `[decided — 2026-08-15, Pete]`
    ///
    /// A left fold was the alternative and it is the one Postgres would have given away free —
    /// `A EXCEPT B EXCEPT C` already evaluates as `A − (B ∪ C)`, which is the exact shape of the
    /// question that motivated this op, with one stage fewer. It is refused for two reasons that
    /// compound:
    ///
    /// - **This field would mean two different things at once.** It is a set for `union` and
    ///   `intersect` and would be a distinguished head plus a set for `difference` — one `Vec` with
    ///   two readings, in a struct that carries no marker saying which is in force.
    /// - **Nothing would disclose the size of what was subtracted.** With `B ∪ C` written as its
    ///   own union stage, that stage carries a tally and a reader can see `|B ∪ C|`. Folded into
    ///   the difference, the union has no stage, no tally and no name — the same
    ///   no-stage-no-disclosure shape [`super::disposition::RefusalReason::DuplicateInputRelation`]
    ///   refuses for act inputs, one node kind over.
    ///
    /// The cost is stated rather than hidden: `A − (B ∪ C)` is three stages here and two under a
    /// fold.
    // `min_items` publishes the arity the doc sentence above already states and `validate` already
    // enforces as `combinator_arity`. The derive cannot infer a bound from a refusal, so without
    // this the contract admits a one-input combination the server always rejects.
    //
    // There is no `max_items`, and its absence is FORCED rather than chosen: the ceiling is
    // per-op, this schema describes the struct shared by all three, and a derive cannot say "two,
    // but only when `op` is `difference`". A blanket `max_items = 2` would publish a bound that
    // forbids the three-way union `validate` admits. So the upper bound is `validate`'s alone —
    // the same division `min_items` and `combinator_arity` already have, one bound over.
    #[cfg_attr(feature = "web-api", schema(min_items = 2))]
    pub inputs: Vec<StageName>,
}

/// A node in the composition DAG: an act invocation, or a set combination over other nodes.
///
/// **No `Eq`, only `PartialEq`** — the act variant carries an intention, which carries the query
/// vector. See [`Intention`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
// Untagged so a plan's JSON reads naturally, with no synthetic node-kind discriminator. The two
// variants are unambiguous: a `CombineNode` carries `op`, an `ActInvocation` carries `act`.
#[serde(untagged)]
#[expect(
    clippy::large_enum_variant,
    reason = "Act is the overwhelmingly common node; boxing it to shave the rare Combine variant \
              would add a heap indirection on the hot path and force Box::new at every \
              construction and match site downstream, for no gain at a handful-of-nodes-per-\
              composition scale. The size asymmetry is inherent to ActInvocation carrying the \
              full per-act envelope, and the wire representation is identical either way."
)]
pub enum StageNode {
    Act(ActInvocation),
    Combine(CombineNode),
}

impl StageNode {
    pub fn name(&self) -> &StageName {
        match self {
            StageNode::Act(a) => &a.name,
            StageNode::Combine(c) => &c.name,
        }
    }

    /// Every upstream stage name this node reads. Empty for a caller-fed or root act.
    pub fn upstream_names(&self) -> Vec<&StageName> {
        match self {
            // **Every upstream input, not the first.** `[widened — 2026-08-14]` A stage may now
            // carry a seed and a bound at once, and both can name an upstream stage — so a
            // first-match here would drop a real DAG edge, and the cycle check and the topological
            // order are both built from this list.
            StageNode::Act(a) => a
                .inputs
                .iter()
                .filter_map(|i| match i {
                    super::stage::StageInput::Upstream { stage, .. } => Some(stage),
                    super::stage::StageInput::Caller { .. } => None,
                })
                .collect(),
            StageNode::Combine(c) => c.inputs.iter().collect(),
        }
    }
}

/// A composition, declared before execution.
///
/// **No `Eq`, only `PartialEq`** — it transitively holds the query vector. See [`Intention`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Composition {
    pub outcome: OutcomeDeclaration,
    // There is deliberately no `intention` here — it moved onto each `ActInvocation`
    // `[decided — 2026-08-12, Pete]`, spec ⟨7⟩. On the envelope it was computed once and threaded,
    // which made "every find stage asks the same question" structural — and made asking TWO
    // questions in one DAG impossible. That trade is now taken the other way: two stages may ask
    // different things, and each stage's question is declared where the stage is.
    //
    // Removed under the same remove-and-tolerate precedent as `on_stage_refusal` and `bounds`: an
    // envelope-level `intention` key on an old payload is ignored like any unknown field, pinned by
    // the legacy-payload test below. It must NOT come back as a DEFAULT a stage inherits when it
    // declares none — that is tasker-grammar's prev-else-context fallback under another name, and
    // this contract exists partly to refuse it.
    // There is deliberately no `on_stage_refusal`. It was a required
    // `RefusalDisposition {halt | degrade_and_disclose}` describing a case that cannot occur:
    // every `RefusalReason` bar one is decidable statically, so a composition that could refuse
    // never runs at all — it comes back 400 with all of its refusals.
    //
    // The single runtime refusal, `EmbeddingUnavailable`, does not want a caller-declared
    // disposition either. Its two settings are observationally near-identical (both answer 200
    // with a full trace; the only difference is whether downstream stages run against empty sets),
    // and it is not per-stage — the intention is computed ONCE and threaded, so an embedding
    // failure fails every find-about-* stage at the same instant. A per-composition disposition
    // over a per-stage refusal is machinery for a shape that does not exist.
    //
    // ONE BEHAVIOUR: the refused stage reports `refused` with no rows, every other stage runs, and
    // a stage downstream of a refusal receives an EMPTY set — never an absent one. Empty is
    // bounded-to-nothing; absent is unbounded, and collapsing them turns a failed stage into a
    // global search wearing a full page of plausible results. That distinction needs no new
    // mechanism: the fragments already read `p_bound_ids = '{}'` as zero rows and
    // `p_bound_ids IS NULL` as unbounded.
    //
    // If a later runtime refusal genuinely wants a choice, the field returns as an OPTIONAL
    // addition, which is additive rather than breaking. `[decided — 2026-08-08, Pete]`
    //
    // There is also no `meta_detail`. It selected how much per-resource meta the trace would
    // retain — a metadata-budget concept whose job nobody could state (YAGNI); nothing ever
    // honoured it. Removed by ADJ-4 `[2026-08-10, Pete]`.
    //
    // And no `bounds`. A composition cannot meaningfully have bounds: its output is nothing but
    // the returned stages' outputs, each already bounded by its own `terms`, and a cross-stage
    // budget would be a different, undesigned feature. Removed by ADJ-4 `[2026-08-10, Pete]`, per
    // the `on_stage_refusal` remove-and-tolerate precedent: an unknown `bounds` key is ignored
    // like any unknown field — pinned by the legacy-payload test below.
    /// The DAG's nodes. Each references its inputs explicitly by stage name — there is no
    /// prev-else-fallback, and no single execution order (a DAG has none). Beat B's topological
    /// sort derives the order; there is deliberately no `act_sequence` method, which would be a
    /// false claim that a DAG has one sequence.
    ///
    /// At most [`MAX_STAGES`] of them, refused as
    /// [`super::disposition::RefusalReason::TooManyStages`].
    // **Published on BOTH doors** `[schemars added — 2026-08-28, found in review]`. The `utoipa`
    // attribute reaches `openapi.json` and the three SDKs; the `schemars` one reaches the MCP tool's
    // input schema, which IS this type (`temper-mcp`'s `run_query`). Until the second was added, an
    // MCP client read the doc line *"at most [`MAX_STAGES`] of them"* — a Rust symbol with no
    // resolvable value — and had no number to hold the server to. Two attributes for one fact is
    // not duplication to remove: they are two published contracts, and
    // `the_mcp_door_publishes_the_ceilings_it_enforces` pins both against the constant.
    //
    // `max_items` is not decoration here — it is what makes the refusal legal where it is raised.
    // The seam guard (`tests/query_validate_seam.rs`) admits a reason into the SHAPE pass only if
    // "asserting it cannot change without a wire-contract change", and the shape module's own rule
    // is that a client running against a NEWER server must not refuse a plan that server would
    // run. A cap enforced but unpublished would fail both: raising it later would silently turn
    // every stale `temper query --check` into a false refusal. Published, widening it is an
    // ordinary contract change that `openapi.json` and the drift gates see.
    //
    // No `min_items = 1` beside it, and that gap is NOT a ruling — nobody has taken one. `validate`
    // refuses an empty list as `no_stages`, so the contract admits a request the server always
    // rejects, which is the exact argument the `returns` field's own `min_items` comment makes one
    // struct up. It is untouched here only so this change carries one bound rather than two.
    #[cfg_attr(feature = "web-api", schema(max_items = 64))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 64)))]
    pub stages: Vec<StageNode>,
}

/// The most stages one composition may declare.
///
/// # Chosen against the scale the code already assumes, in both directions
///
/// The lower anchor is written down two hundred lines above: [`StageNode`]'s `large_enum_variant`
/// exemption argues from "a handful-of-nodes-per-composition scale", and the widest plan anything
/// in this repo builds is three. 64 is an order of magnitude above that — far above any question
/// anyone has asked, so no real caller meets it.
///
/// The upper anchor is what a stage list costs to run. Every declared stage executes whether or
/// not `returns` names it, so the count is the plan's cost, and 64 sits orders of magnitude below
/// where that cost becomes interesting.
///
/// Same method as `MAX_PER_CANDIDATE_PREDICATES`: far above the question, far below the harm. It
/// is a starting point rather than a measured cliff, and unlike the per-candidate caps it has no
/// measurement behind it — widening it is a contract change, which is where the argument for a
/// different number should be made.
pub const MAX_STAGES: usize = 64;

/// The most bytes one stage's question may carry.
///
/// # What it bounds is work paid for and then discarded
///
/// The embedder tokenizes the WHOLE string and truncates the resulting encoding to the model's
/// 512-token window (`temper-ingest::embed`'s `embed_batch` → `truncate_encoding`). So every byte
/// past that window is tokenized at the caller's request and thrown away — the caller chooses the
/// cost and receives none of it.
///
/// # Chosen against the window it feeds, in both directions
///
/// 512 tokens of English is roughly 2 KB. 4 KB is comfortably above that — a question long enough
/// to be fully consumed by the model, with room for a script that tokenizes less efficiently — and
/// far below the point where tokenizing the excess is interesting. Same method as [`MAX_STAGES`]
/// and `MAX_PER_CANDIDATE_PREDICATES`: far above the question, far below the harm.
///
/// **Refused rather than truncated.** Shortening a question silently would answer a different
/// question than the one asked, which is the substitution this contract keeps closing — and unlike
/// the model's own truncation, a refusal is something the caller can see and repair.
pub const MAX_INTENTION_QUERY_BYTES: usize = 4096;

/// The most floats a caller-supplied query vector may carry.
///
/// `[added — 2026-08-28, found in review]` It is the model's dimension — bge-base-en-v1.5 emits
/// 768, which `temper-ingest`'s `EMBEDDING_DIM` names on the other side of a dependency this crate
/// does not have — and the number is repeated here rather than shared for that reason, with a test
/// pinning the two together where both are reachable.
///
/// **The bound is on the COUNT and the refusal is about the SHAPE**, which is why it is not one of
/// the `TooMany…` family. A vector of any other length is not a large question, it is a vector for
/// a different space: `pgvector` rejects it, and this door redacts that to an opaque 500. So the
/// check is `!= MAX_EMBEDDING_DIM` and the refusal says *wrong shape*, since a caller who sends 767
/// floats has the same problem as one who sends 769 and neither is helped by being told about a
/// maximum.
///
/// `[corrected — 2026-08-28, found in review]` The field publishes this as **both** `min_items` and
/// `max_items`. An earlier revision published only the maximum, which let a short vector clear every
/// generated client and be refused by the server — the enforced rule was an equality and the
/// published one was an inequality, so a client could not refuse what the server would.
pub const MAX_EMBEDDING_DIM: usize = 768;

/// The norm window a caller-supplied query vector must land in.
///
/// The corpus's space is unit-normalized — every embedding this system computes for itself has
/// norm 1.0 — so a caller's vector is plausible only near that. The window is many
/// orders of magnitude wide on both sides: a caller who pre-scaled by a constant is sending
/// **direction**, which is all a cosine reads, and is not worth refusing; the window exists for
/// the values that are not questions at all. Below it sits the all-zero vector, whose cosine is
/// 0/0 — a NaN that orders as "unknown" and turns every score computed from it into a score
/// nobody can act on. Above it, float accumulation in the distance computation itself overflows:
/// 768 components of 1e9 square-sum to ~8e20, which survives `f32`, and a caller with no such
/// ceiling has no reason not to send one that does not. Both ends today reached pgvector and came
/// back as driver errors this door renders as an opaque 500. Values that are not finite are
/// refused by the same check — a NaN poisons every cosine it touches, for the same reason.
pub const MIN_EMBEDDING_NORM: f32 = 1.0e-6;
pub const MAX_EMBEDDING_NORM: f32 = 1.0e6;

/// The most question text ONE COMPOSITION may hand the server to embed, summed across its stages.
///
/// # Why a per-stage cap is not enough, measured
///
/// [`MAX_INTENTION_QUERY_BYTES`] and [`MAX_STAGES`] shipped together, and their product is 256 KB
/// of text the server must embed inside **one** wall-clock budget — `DEFAULT_QUERY_EMBED_BUDGET_MS`,
/// 8,000 ms, whose own doc calls it the budget for *"a single server-side query embed"* and which
/// was already tight enough for one that the production fix was more memory and a keep-warm cron
/// rather than a larger number. Over budget, no stage gets a vector and every find stage refuses
/// `embedding_unavailable`.
///
/// # 40 KB, and why the first number was wrong
///
/// `[recalibrated — 2026-08-28, found in review]` This was 64 KB, chosen against a measurement of
/// **16 × 4,096 bytes = 6,309 ms**. That is one of TWO shapes that sit at 64 KB, and it is the
/// cheaper one. The other was not measured:
///
/// | shape | total bytes | wall clock |
/// |---|---|---|
/// | 64 × 640 B | 40,960 | 4,284 ms |
/// | 16 × 4,096 B | 65,536 | 6,309 ms |
/// | **64 × 1,024 B** | **65,536** | **7,384 ms — 92% of the budget** |
/// | 64 × 4,096 B | 262,144 | 25,603 ms |
///
/// `[measured — 2026-08-28, ORT loaded, threads=1, realistic English, cold load EXCLUDED]`
///
/// **The mechanism is token saturation.** `MAX_MODEL_TOKENS` is 512 and the encoder truncates each
/// question to it, so per-question cost stops growing at roughly 2 KB of English — 640 B is 113
/// tokens, 1,024 B is 180, and 4,096 B is 512 and saturated. Bytes therefore *overstate* the cost
/// of long questions and *understate* many short ones, and the worst case at any byte total is
/// [`MAX_STAGES`] questions, not the fewest that fit. A tokenized bound would be the honest
/// denominator and is not available here: counting tokens needs the model, and this pass runs
/// without one.
///
/// So the number is re-derived against the 64-stage row and scaled to leave the budget's remaining
/// 40% for what the measurement excluded — the cold model load, which `embed_service`'s own doc
/// calls a *"best-effort"* warmth because Vercel does not promise the keep-warm cron and a user
/// request land on the same instance. 40 KB predicts ~4,600 ms at that shape.
///
/// It costs nothing real: the largest composition anywhere in this repository totals **702 bytes**
/// of question text `[surveyed — 2026-08-28]`, 1.7% of this cap.
///
/// **Raising the embed budget instead was the alternative and was declined**
/// `[decided — 2026-08-28, Pete]`: covering the worst legal plan needs ≥25 s measured, likely 40 s+
/// on a ~1.7 vCPU function, against a 60 s `maxDuration` shared with compile, execute and hydrate.
/// That number would have been invented rather than validated; this one keeps a number that has
/// survived production.
///
/// # It is a floor, not a fixed ceiling — see [`intention_budget_bytes`]
///
/// The refusal is capability's precisely because this is a property of the machine, and
/// `TEMPER_QUERY_EMBED_BUDGET_MS` — the budget it is derived from — is already an env override. A
/// deployment that raises one and not the other buys nothing, so this moves the same way.
pub const MAX_COMPOSITION_INTENTION_BYTES: usize = 40_960;

/// [`MAX_COMPOSITION_INTENTION_BYTES`], or the deployment's own number.
///
/// `[added — 2026-08-28, found in review]` The capability placement of
/// `RefusalReason::IntentionBudgetExceeded` rests on the claim that *"a beefier deployment could
/// raise it"* — which the code did not honour: the budget it derives from is
/// `TEMPER_QUERY_EMBED_BUDGET_MS`, an env var, while this was a hard `const` nobody could move. A
/// justification the code contradicts is worse than no justification, because it is the sentence a
/// later reader cites when deciding the refusal could live in the shape pass after all. It cannot:
/// two deployments may now legitimately disagree about this number, which is exactly why a client
/// must never raise it.
///
/// Zero and unparseable values fall back rather than disabling the bound, matching
/// `query_embed_budget`'s own `filter(|&ms| ms > 0)`.
pub fn intention_budget_bytes() -> usize {
    std::env::var("TEMPER_QUERY_INTENTION_BUDGET_BYTES")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(MAX_COMPOSITION_INTENTION_BYTES)
}

/// What a caller actually asked for, measured before anything decides whether to answer it.
///
/// # Why this exists, and why every field is a COUNT rather than a verdict
///
/// `[added — 2026-08-28]` `/api/query` recorded nothing about the composition it received — not the
/// stage count, not the question size, not how many ids were piped in. So the only answer available
/// to *"is this ceiling above what callers actually send?"* was the fixtures in this repository,
/// which contain no real callers. A bound chosen that way is a guess wearing a measurement's
/// clothes, and the first evidence that it was set too low would be a customer.
///
/// **The fields are raw quantities rather than `would_refuse` booleans.** A verdict
/// answers one question — *does today's cap fire?* — and goes stale the moment the cap moves. The
/// distribution answers every question anyone asks later, including about ceilings nobody has
/// proposed yet, and it can be re-read against a new number without redeploying to collect it
/// again.
///
/// **Recorded BEFORE validation**, which is the whole design: a shape emitted only for plans that
/// pass would show exactly the traffic no ceiling ever refuses, and none of the traffic a ceiling
/// would have. That inverts the question this is here to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompositionShape {
    /// Stages declared. Bounded on the wire since 2026-08-26.
    pub stages: usize,
    /// Entries in `outcome.returns`.
    pub returns: usize,
    /// Every stage's question, summed — whether or not the server would embed it.
    pub intention_bytes: usize,
    /// The subset the SERVER would embed: distinct text on stages carrying no vector. Separate from
    /// the total because a caller who precomputes pays nothing, and a question asked by ten stages
    /// is embedded once — so the two numbers can differ by any factor, and it is this one that a
    /// wall-clock embed budget is a bound on.
    pub embedded_bytes: usize,
    /// The longest single question.
    pub longest_question: usize,
    /// The largest caller-supplied id set on any one stage.
    pub largest_id_set: usize,
    /// Every caller-supplied id across the plan.
    pub caller_ids: usize,
    /// The longest narrowing list — `doc_type`, `tags` or `labels` — on any one stage.
    pub largest_filter_list: usize,
    /// Stages arriving with a vector already computed. Zero for every MCP caller, structurally:
    /// that door cannot run the model, which is why the server embeds on its behalf.
    pub embeddings_supplied: usize,
    /// Floats in the largest supplied vector. A number other than the model's dimension is a
    /// caller sending a vector for a different space.
    pub largest_embedding: usize,
    /// Stages whose combinator, `with` list or `edge_kinds` names the same member twice. A repeat
    /// changes no answer, so this is the one field that is a defect count rather than a size.
    pub repeated_members: usize,
}

impl CompositionShape {
    /// Measure a composition. Pure, allocation-light, and safe to run on every request.
    pub fn of(c: &Composition) -> Self {
        let mut s = Self {
            stages: c.stages.len(),
            returns: c.outcome.returns.len(),
            ..Self::default()
        };

        for ret in &c.outcome.returns {
            let mut seen: BTreeSet<&ResourceSection> = BTreeSet::new();
            if ret.with.iter().any(|sec| !seen.insert(sec)) {
                s.repeated_members += 1;
            }
        }

        let mut distinct_to_embed: BTreeSet<&str> = BTreeSet::new();
        for node in &c.stages {
            let inv = match node {
                StageNode::Act(inv) => inv,
                StageNode::Combine(cn) => {
                    let mut seen: BTreeSet<&str> = BTreeSet::new();
                    if cn.inputs.iter().any(|i| !seen.insert(i.as_str())) {
                        s.repeated_members += 1;
                    }
                    continue;
                }
            };

            if let Some(intention) = &inv.intention {
                s.intention_bytes += intention.query.len();
                s.longest_question = s.longest_question.max(intention.query.len());
                match &intention.embedding {
                    Some(v) => {
                        s.embeddings_supplied += 1;
                        s.largest_embedding = s.largest_embedding.max(v.len());
                    }
                    // Distinct text, matching what the embedder would actually run: two stages
                    // naming one question are one embedding. Not gated on whether the ACT searches
                    // by vector — that reads the registry, and this is a measurement rather than a
                    // decision, so it stays a pure function of the body.
                    None => {
                        distinct_to_embed.insert(intention.query.trim());
                    }
                }
            }

            for input in &inv.inputs {
                if let StageInput::Caller { ids, .. } = input {
                    s.caller_ids += ids.ids.len();
                    s.largest_id_set = s.largest_id_set.max(ids.ids.len());
                }
            }

            if let Some(f) = &inv.resource_filter {
                s.largest_filter_list = s
                    .largest_filter_list
                    .max(f.doc_type.len())
                    .max(f.tags.len());
            }
            if let Some(f) = &inv.edge_filter {
                s.largest_filter_list = s.largest_filter_list.max(f.labels.len());
                let mut seen: Vec<&EdgeKind> = Vec::new();
                if f.edge_kinds.iter().any(|k| {
                    let dup = seen.contains(&k);
                    seen.push(k);
                    dup
                }) {
                    s.repeated_members += 1;
                }
            }
        }
        s.embedded_bytes = distinct_to_embed.iter().map(|t| t.len()).sum();
        s
    }

    /// Emit the shape at INFO, one event per request.
    ///
    /// An event rather than span fields: `/api/query` is a READ, so it opens no act span (span
    /// conventions, clause 2 — *"asserting act ids on every request would encode a fiction"*), and
    /// recording onto `Span::current()` is the trap that document names, since the ids would
    /// silently attach to whatever span happens to be current once a nested one appears.
    ///
    /// `door` distinguishes the HTTP surface from MCP, which matters more than it looks:
    /// `embeddings_supplied` is structurally zero for every MCP caller, so any bound on what the
    /// server must embed binds that door alone and its distribution has to be read separately.
    pub fn record(&self, door: &'static str) {
        tracing::info!(
            door,
            stages = self.stages,
            returns = self.returns,
            intention_bytes = self.intention_bytes,
            embedded_bytes = self.embedded_bytes,
            longest_question = self.longest_question,
            largest_id_set = self.largest_id_set,
            caller_ids = self.caller_ids,
            largest_filter_list = self.largest_filter_list,
            embeddings_supplied = self.embeddings_supplied,
            largest_embedding = self.largest_embedding,
            repeated_members = self.repeated_members,
            "composition shape"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod shape {
        use super::*;
        use crate::types::query::act::ActName;
        use crate::types::query::filter::{EdgeFilter, ResourceFilter};
        use crate::types::query::id_set::{IdKind, IdSet};
        use crate::types::query::stage::StageRelation;

        fn asking(name: &str, query: &str, embedding: Option<Vec<f32>>) -> StageNode {
            StageNode::Act(ActInvocation {
                name: StageName::parse(name).unwrap(),
                act: ActName::FindAboutAnywhere,
                intention: Some(Intention {
                    query: query.to_string(),
                    embedding,
                }),
                inputs: vec![],
                terms: BTreeMap::new(),
                resource_filter: None,
                edge_filter: None,
                properties: vec![],
            })
        }

        fn plan(stages: Vec<StageNode>) -> Composition {
            Composition {
                outcome: OutcomeDeclaration { returns: vec![] },
                stages,
            }
        }

        /// **`embedded_bytes` is what the server would EMBED, and `intention_bytes` is what the
        /// caller SENT.** They differ by any factor, and conflating them is how a wall-clock embed
        /// budget gets sized against the wrong quantity — which is the whole reason both are here.
        #[test]
        fn the_embedded_subset_is_distinct_text_on_stages_carrying_no_vector() {
            let c = plan(vec![
                asking("a", "same question", None),
                asking("b", "same question", None),
                asking("c", "another", None),
                asking("d", "precomputed already", Some(vec![0.1; 768])),
            ]);
            let s = CompositionShape::of(&c);

            assert_eq!(
                s.intention_bytes,
                13 + 13 + 7 + 19,
                "every question, as sent"
            );
            assert_eq!(
                s.embedded_bytes,
                13 + 7,
                "one embedding for the repeated question, none for the precomputed stage"
            );
            assert_eq!(s.embeddings_supplied, 1);
            assert_eq!(s.largest_embedding, 768);
            assert_eq!(s.longest_question, 19);
        }

        /// The size fields take the LARGEST on any one stage, not a sum — a ceiling is per stage,
        /// so a total would be measuring a quantity no bound is expressed in.
        #[test]
        fn the_size_fields_report_the_largest_single_stage_and_the_id_total_beside_it() {
            let ids = |n: usize| StageInput::Caller {
                relation: StageRelation::Bound,
                ids: IdSet {
                    kind: IdKind::Resource,
                    provenance: None,
                    ids: (0..n).map(|_| uuid::Uuid::now_v7()).collect(),
                },
            };
            let mut wide = match asking("a", "q", None) {
                StageNode::Act(mut inv) => {
                    inv.inputs = vec![ids(7), ids(3)];
                    inv.resource_filter = Some(ResourceFilter {
                        doc_type: vec!["t".into(); 4],
                        tags: vec!["x".into(); 9],
                        ..Default::default()
                    });
                    StageNode::Act(inv)
                }
                other => other,
            };
            let narrow = match asking("b", "q", None) {
                StageNode::Act(mut inv) => {
                    inv.inputs = vec![ids(2)];
                    StageNode::Act(inv)
                }
                other => other,
            };
            if let StageNode::Act(inv) = &mut wide {
                inv.edge_filter = Some(EdgeFilter {
                    labels: vec!["l".into(); 5],
                    ..Default::default()
                });
            }
            let s = CompositionShape::of(&plan(vec![wide, narrow]));
            assert_eq!(s.largest_id_set, 7, "the biggest single set, not 7+3+2");
            assert_eq!(s.caller_ids, 12, "and the total beside it");
            assert_eq!(
                s.largest_filter_list, 9,
                "tags, across both filter containers"
            );
        }

        /// Repeats are counted from all three places one can occur, because each is a caller
        /// turning a bounded vocabulary into an unbounded field and the distribution has to say
        /// whether anyone actually does it.
        #[test]
        fn repeated_members_counts_returns_combinators_and_edge_kinds() {
            use crate::types::graph::EdgeKind;
            let combine = StageNode::Combine(CombineNode {
                name: StageName::parse("both").unwrap(),
                op: CombineOp::Union,
                inputs: vec![
                    StageName::parse("a").unwrap(),
                    StageName::parse("a").unwrap(),
                ],
            });
            let walk = match asking("w", "q", None) {
                StageNode::Act(mut inv) => {
                    inv.edge_filter = Some(EdgeFilter {
                        edge_kinds: vec![EdgeKind::LeadsTo, EdgeKind::LeadsTo],
                        ..Default::default()
                    });
                    StageNode::Act(inv)
                }
                other => other,
            };
            let c = Composition {
                outcome: OutcomeDeclaration {
                    returns: vec![ReturnSpec {
                        stage: StageName::parse("w").unwrap(),
                        with: vec![ResourceSection::OpenMeta, ResourceSection::OpenMeta],
                    }],
                },
                stages: vec![asking("a", "q", None), combine, walk],
            };
            assert_eq!(CompositionShape::of(&c).repeated_members, 3);
        }

        /// An empty plan measures zero rather than panicking — it is refused a layer later, and a
        /// measurement that cannot survive the inputs validation exists to reject is a measurement
        /// that stops being taken exactly when something odd arrives.
        #[test]
        fn a_plan_with_nothing_in_it_measures_zero() {
            assert_eq!(
                CompositionShape::of(&plan(vec![])),
                CompositionShape::default()
            );
        }
    }

    use crate::types::query::act::ActName;
    use crate::types::query::envelope::ActInvocation;
    use crate::types::query::stage::{StageInput, StageRelation};
    use std::collections::BTreeMap;

    /// A minimal root act node named `s`, carrying no question.
    fn stage(act: ActName) -> StageNode {
        StageNode::Act(ActInvocation {
            name: StageName::parse("s").unwrap(),
            act,
            intention: None,
            inputs: vec![],
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    }

    /// [`stage`] carrying a question. These are serialization tests, so they never run `validate`
    /// and a questionless find act is representable here — which is exactly what
    /// `an_absent_intention_is_representable_so_a_stage_can_refuse_rather_than_substitute` asserts.
    fn stage_asking(act: ActName, query: &str) -> StageNode {
        match stage(act) {
            StageNode::Act(mut inv) => {
                inv.intention = Some(Intention {
                    query: query.to_string(),
                    embedding: None,
                });
                StageNode::Act(inv)
            }
            other => other,
        }
    }

    fn outcome(returns: Vec<ReturnSpec>) -> OutcomeDeclaration {
        OutcomeDeclaration { returns }
    }

    fn composition(stages: Vec<StageNode>) -> Composition {
        Composition {
            outcome: outcome(vec![]),
            stages,
        }
    }

    #[test]
    fn a_composition_no_longer_declares_what_to_do_when_a_stage_refuses() {
        // `on_stage_refusal` described a case that cannot occur: every refusal but one is static,
        // so a composition that could refuse comes back 400 and never runs. The one runtime
        // refusal is composition-wide (the intention is embedded ONCE), so a per-stage disposition
        // would be machinery for a shape that does not exist.
        //
        // Asserting on the SERIALIZED form rather than the absence of a field: the field being
        // gone is a compile-time fact this test cannot restate, but the wire not carrying it is
        // what a client actually observes.
        let c = composition(vec![stage(ActName::FindExact)]);
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("on_stage_refusal"), "got: {json}");
        assert!(!json.contains("halt"));
        assert_eq!(serde_json::from_str::<Composition>(&json).unwrap(), c);
    }

    #[test]
    fn a_composition_that_still_carries_on_stage_refusal_is_accepted_and_the_field_ignored() {
        // Removing a required field is a breaking change for anyone who was sending it. Nothing
        // ships against this contract yet — no route exists — so the removal is free, and serde's
        // default is to ignore unknown fields rather than reject them. Pinned so that if someone
        // later adds `deny_unknown_fields` for good reasons, they see this decision rather than
        // silently turning an ignored legacy field into a hard 400.
        let legacy = r#"{"outcome":{"returns":[]},"on_stage_refusal":"halt","stages":[]}"#;
        assert!(serde_json::from_str::<Composition>(legacy).is_ok());
    }

    #[test]
    fn a_composition_that_still_carries_bounds_or_meta_detail_is_accepted_and_both_ignored() {
        // Same precedent as `on_stage_refusal` above: `bounds` and `meta_detail` were removed by
        // ADJ-4 `[2026-08-10, Pete]`, nothing ships against this contract yet, and serde's default
        // is to ignore unknown fields rather than reject them. Pinned so that a later
        // `deny_unknown_fields` sees this decision rather than silently turning an ignored legacy
        // field into a hard 400 — and so the removal is visibly remove-and-tolerate, not a parse
        // hazard.
        let legacy = r#"{"outcome":{"returns":[]},"meta_detail":"surviving","bounds":{"limit":10},"stages":[]}"#;
        let parsed = serde_json::from_str::<Composition>(legacy).expect("legacy fields parse");
        assert_eq!(
            parsed,
            composition(vec![]),
            "and neither influences anything"
        );
    }

    #[test]
    fn the_intention_is_a_per_stage_field_and_two_stages_may_ask_different_questions() {
        // `[inverted — 2026-08-12]` This asserted the OPPOSITE, under the name
        // `the_intention_is_a_composition_level_field_not_a_per_stage_one`, on the rationale
        // "computed ONCE at composition start and threaded, so every find-about-* stage provably
        // interrogates the same intention rather than re-embedding a mutated string."
        //
        // That property is GIVEN UP deliberately — spec ⟨7⟩, `[decided — 2026-08-12, Pete]` — and
        // not lost. It made "every find stage asks the same question" structural, and made asking
        // two questions in one DAG impossible, so `find A, find B, intersect them` could not be
        // expressed at all. What replaces it is per-stage declaration: each stage's question sits
        // where the stage is.
        //
        // Kept as an inversion rather than deleted because the old name is what hardened: it was
        // greppable, it read as a settled invariant, and it steered three sessions of planning. A
        // test asserting the opposite is what stops it being re-derived.
        let c = composition(vec![
            stage_asking(ActName::FindAboutAnywhere, "wayfind salience"),
            stage_asking(ActName::FindAboutAnywhere, "region scoring"),
        ]);
        let json = serde_json::to_string(&c).unwrap();
        // TWO intentions, one per stage — and none on the envelope.
        assert_eq!(json.matches("\"intention\"").count(), 2);
        assert!(json.contains("wayfind salience") && json.contains("region scoring"));
        assert_eq!(serde_json::from_str::<Composition>(&json).unwrap(), c);
    }

    #[test]
    fn an_absent_intention_is_representable_so_a_stage_can_refuse_rather_than_substitute() {
        // The absence that refuses is the QUESTION's, not the vector's. With no intention there is
        // no query text, and `find-exact` has nowhere else to get its `p_query` — so the stage
        // refuses `MissingIntention`. An absent EMBEDDING is a different absence entirely: the
        // server computes one, because API callers cannot.
        //
        // Now asserted PER STAGE: the refusal follows the stage that omitted its question, not the
        // composition that failed to thread one.
        let c = composition(vec![stage(ActName::FindExact)]);
        let StageNode::Act(inv) = &c.stages[0] else {
            panic!("act node");
        };
        assert!(inv.intention.is_none());
        assert!(!serde_json::to_string(&c).unwrap().contains("intention"));
    }

    #[test]
    fn an_intention_carries_the_query_and_the_callers_vector_and_asserts_nothing_about_it() {
        // `[rewritten — 2026-08-12]` Was `an_intention_carries_the_fact_of_embedding_and_never_the
        // _vector`, pinning a byte-exact `{"query":…,"embedded":false}`. Both halves are retired.
        //
        // The VECTOR is here now (spec ⟨7⟩): it rides beside the text it was computed from, so the
        // two cannot drift apart. The old rationale — "putting it in the envelope would be a wire
        // contract nobody asked for" — was about the ENVELOPE and about response bloat, and
        // `CompositionTrace` carries only `stages` and echoes no intention, so nothing here reaches
        // a response.
        //
        // And `embedded: bool` is GONE `[decided — 2026-08-13, Pete]`. It claimed to make
        // paraphrase-stability "measurable from outside" while never appearing in any trace; its
        // only live distinction — your vector versus the server's — is already covered on the
        // failing side by `EmbeddingUnavailable`; and the real hazard it looked like it addressed
        // (a CLI embedding with a different model than the corpus) is guarded by build.rs's model
        // sha256 pin, which a boolean could not express anyway.
        let bare = Intention {
            query: "composable search fragments".to_string(),
            embedding: None,
        };
        // An absent vector serializes to nothing at all — a caller that cannot embed sends a
        // question and no key, exactly as the ruby gem and MCP do.
        assert_eq!(
            serde_json::to_string(&bare).unwrap(),
            r#"{"query":"composable search fragments"}"#
        );

        let carried = Intention {
            query: "composable search fragments".to_string(),
            embedding: Some(vec![0.25, -0.5]),
        };
        assert_eq!(
            serde_json::to_string(&carried).unwrap(),
            r#"{"query":"composable search fragments","embedding":[0.25,-0.5]}"#
        );
        assert_eq!(
            serde_json::from_str::<Intention>(&serde_json::to_string(&carried).unwrap()).unwrap(),
            carried
        );
    }

    #[test]
    fn a_combinator_is_its_own_node_kind_because_no_act_takes_two_inputs() {
        let c = CombineNode {
            name: StageName::parse("both").unwrap(),
            op: CombineOp::Union,
            inputs: vec![
                StageName::parse("quoted").unwrap(),
                StageName::parse("wide").unwrap(),
            ],
        };
        let node = StageNode::Combine(c.clone());
        assert_eq!(node.name(), &c.name);
        assert_eq!(node.upstream_names().len(), 2);
        assert_eq!(
            serde_json::from_str::<StageNode>(&serde_json::to_string(&node).unwrap()).unwrap(),
            node
        );
    }

    #[test]
    fn a_difference_is_ordered_and_names_the_set_it_subtracts_from_first() {
        // `Union` and `Intersect` are commutative, so `inputs` has been readable as a SET since it
        // was written. `Difference` is the first op for which it is a SEQUENCE — `A − B ≠ B − A` —
        // and nothing in the struct's shape says so, because the field is shared by all three.
        //
        // Asserted on the SERIALIZED form because the order a client sends is the only thing that
        // decides which set survives, and a `Vec` that happened to round-trip in the wrong order
        // would be a silently different question answered.
        let d = CombineNode {
            name: StageName::parse("gap").unwrap(),
            op: CombineOp::Difference,
            inputs: vec![
                StageName::parse("tasks").unwrap(),
                StageName::parse("declared").unwrap(),
            ],
        };
        let json = serde_json::to_string(&StageNode::Combine(d.clone())).unwrap();
        assert!(json.contains(r#""op":"difference""#), "got: {json}");
        // The minuend precedes the subtrahend on the wire, in the order the caller wrote them.
        assert!(
            json.find(r#""tasks""#).unwrap() < json.find(r#""declared""#).unwrap(),
            "input order is semantic for a difference: {json}"
        );
        assert_eq!(
            serde_json::from_str::<StageNode>(&json).unwrap(),
            StageNode::Combine(d)
        );
    }

    #[test]
    fn a_difference_reports_both_arms_upstream_so_the_dag_sees_the_subtrahend() {
        // The subtrahend is a real DAG edge — the cycle check and the topological order are both
        // built from `upstream_names`, and a difference whose right arm went unreported would be
        // emitted before the CTE it reads.
        let d = StageNode::Combine(CombineNode {
            name: StageName::parse("gap").unwrap(),
            op: CombineOp::Difference,
            inputs: vec![
                StageName::parse("tasks").unwrap(),
                StageName::parse("declared").unwrap(),
            ],
        });
        assert_eq!(
            d.upstream_names()
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            vec!["tasks", "declared"]
        );
    }

    #[test]
    fn an_act_node_reports_its_single_upstream_and_a_caller_fed_one_reports_none() {
        let seeded = StageNode::Act(ActInvocation {
            name: StageName::parse("near").unwrap(),
            intention: None,
            inputs: vec![StageInput::Upstream {
                relation: StageRelation::Seed,
                stage: StageName::parse("hits").unwrap(),
            }],
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
            act: ActName::FollowFrom,
        });
        assert_eq!(seeded.upstream_names().len(), 1);

        let rooted = StageNode::Act(ActInvocation {
            name: StageName::parse("hits").unwrap(),
            intention: None,
            inputs: vec![],
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
            act: ActName::FindExact,
        });
        assert!(rooted.upstream_names().is_empty());
    }

    #[test]
    fn a_composition_carries_nodes_and_no_longer_claims_a_single_sequence() {
        // `act_sequence()` is gone on purpose: a DAG has no one order, and a method returning one
        // would be a false claim that reads as true. Beat B's topological order replaces it.
        assert!(composition(vec![]).stages.is_empty());
    }

    #[test]
    fn an_outcome_declares_which_stages_come_back_and_nothing_about_why() {
        // `description` is gone. It was a required prose string, which is goal-authoring
        // discipline leaked into a wire contract — nobody should write a sentence about what
        // being served looks like in order to run a query.
        let o = outcome(vec![ReturnSpec {
            stage: StageName::parse("near").unwrap(),
            with: vec![],
        }]);
        let json = serde_json::to_string(&o).unwrap();
        assert_eq!(o.returns.len(), 1);
        assert!(!json.contains("description"), "got: {json}");
        assert_eq!(
            serde_json::from_str::<OutcomeDeclaration>(&json).unwrap(),
            o
        );
    }

    #[test]
    fn an_empty_section_list_means_the_default_projection_and_serializes_to_nothing() {
        let r = ReturnSpec {
            stage: StageName::parse("near").unwrap(),
            with: vec![],
        };
        assert!(!serde_json::to_string(&r).unwrap().contains("with"));
    }

    #[test]
    fn hydration_sections_ride_the_wire_in_the_same_words_show_and_list_use() {
        // ONE vocabulary, not a query-local copy. `open-meta` is kebab here because it is kebab
        // everywhere a human or agent types it; a second spelling would be a second vocabulary
        // wearing the first one's name.
        let r = ReturnSpec {
            stage: StageName::parse("near").unwrap(),
            with: vec![ResourceSection::OpenMeta],
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""with":["open-meta"]"#), "got: {json}");
        assert_eq!(serde_json::from_str::<ReturnSpec>(&json).unwrap(), r);
    }

    #[test]
    fn a_section_this_door_refuses_still_deserializes_so_validation_can_refuse_it() {
        // The load-bearing half of choosing ONE vocabulary over a narrow query-local enum. Were
        // `with` a single-member enum, `body` would fail to DESERIALIZE — and serde
        // short-circuits before validation runs, so a caller with four problems would learn about
        // one, phrased by a deserializer rather than by this contract.
        //
        // So `body` parses here and is refused at validation, which is what lets it come back
        // alongside every other refusal with an explanation that names `show`.
        let parsed: ReturnSpec =
            serde_json::from_str(r#"{"stage":"near","with":["body","edges"]}"#).unwrap();
        assert_eq!(
            parsed.with,
            vec![ResourceSection::Body, ResourceSection::Edges]
        );
        assert!(!ReturnSpec::ADMITTED_SECTIONS.contains(&ResourceSection::Body));
        assert!(!ReturnSpec::ADMITTED_SECTIONS.contains(&ResourceSection::Edges));
    }

    #[test]
    fn the_admitted_sections_are_a_subset_of_the_shared_vocabulary_never_a_parallel_one() {
        // The guard against this door quietly growing a word `show` and `list` do not know. If a
        // section is ever added here, it has to be added to `ResourceSection` first.
        for section in ReturnSpec::ADMITTED_SECTIONS {
            assert!(
                ResourceSection::ALL.contains(&section),
                "{section:?} is not part of the shared section vocabulary"
            );
        }
    }

    #[test]
    fn a_composition_no_longer_declares_one_produced_kind() {
        // A resource arm beside a region arm has no single answer. `produces` was a field that
        // could only ever be right for a single-arm plan — it is derived from `returns` now, not
        // declared.
        assert!(!serde_json::to_string(&outcome(vec![]))
            .unwrap()
            .contains("produces"));
    }
}
