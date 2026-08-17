//! Compile a [`ValidatedComposition`] to ONE SQL statement.
//!
//! This is the second runtime-`sqlx` class in this module (the first is the `::vector` bind — see
//! the module note): the SQL SKELETON is assembled at runtime because the DAG shape is not known at
//! compile time, but **every caller value is a positional bind and never interpolated**, and the
//! only identifiers the builder ever emits are stage names, each already proven a safe SQL
//! identifier by [`temper_core::types::query::StageName`]'s parse-only constructor (beat A).
//!
//! The three `find` acts emit real fragments, and they are now the ONLY acts that reach this
//! builder. `follow-from` and `survey` used to emit a PLACEHOLDER referencing a function
//! (`__temper_unbound_act`) that does not exist in the schema — loud rather than silently wrong if
//! executed, but a failure at execution all the same. They have since left `CALLABLE_FRAGMENTS`, so
//! `validate` refuses them and no [`ValidatedComposition`] can carry one. Their fragments take
//! arguments no slot supplies (`p_depth`/`p_gamma`, `p_lens`), which is what wiring them waits on.
//!
//! The placeholder arm survives regardless — unreachable through `validate`, and emitted anyway, as
//! the drift guard for an act that ever declares itself into `search_family()` without a fragment.
//!
//! [`query_exec`](super::query_exec) runs a [`CompiledQuery`] and hands back its two row classes.
//! What does NOT live in either module is the assembly of a `QueryResponse` — deciding a stage's
//! disposition, hydrating the returned arms, building the trace. That needs the composition and the
//! act declarations together, and keeping it out of the substrate is what stops this layer forming
//! an opinion about what a stage MEANT.

use std::collections::BTreeSet;

use temper_core::types::ids::ProfileId;
use temper_core::types::query::{
    search_family, BoundTerm, IdKind, PlanRefusal, RefusalReason, StageInput, StageNode,
    StageRelation, ValidatedComposition,
};
use uuid::Uuid;

/// A compiled statement and its ordered binds. The SQL text is never concatenated with a caller
/// value — every value is a positional bind.
#[derive(Debug, Clone)]
pub struct CompiledQuery {
    pub sql: String,
    pub binds: Vec<QueryBind>,
    /// Stage name -> CTE name, in emission order. A later task uses it to attribute rows to arms.
    pub cte_names: Vec<(String, String)>,
    /// The stages that refused at COMPILE time — today, exactly the `find-about-*` stages whose
    /// embedding the server had to compute and could not.
    ///
    /// **Carried here rather than returned as an `Err`, and that is the contract's rule not a
    /// convenience.** A refusal is per stage: *"Every other stage runs. Stages that do not depend on
    /// it are unaffected and answer normally — a composition holding both a `find-exact` and a
    /// `find-about-*` still returns the exact arm."* An `Err` would refuse the exact arm too, for a
    /// reason that has nothing to do with it.
    ///
    /// A refused stage's CTE is an EMPTY set, so a stage bounded by it is bounded to nothing —
    /// `ARRAY(SELECT id FROM <it>)` is `'{}'`, which the fragments read as zero rows, and never the
    /// `NULL` they read as unbounded. Collapsing those two turns a failed stage into a global
    /// search.
    ///
    /// **This carries the per-stage refusals, and `compile` can still fail whole.** The tempting
    /// sentence here is "static refusals never reach the compiler, they are `validate`'s" — worth
    /// stating carefully, because getting it wrong is how the next refusal lands on the wrong side.
    ///
    /// One case still returns `Err`: `UnsupportedSeedKind` from `StageNarrowing::bound_expr`, and
    /// `AnchorTakesOneId` from the anchor arm. Both are compiler/validator **contradictions** rather
    /// than anything a caller did — `validate` refuses each of them first — so they are unreachable
    /// through any public path, and failing loud if the two crates ever drift is correct.
    ///
    /// The anchor case used to be a genuine caller error reaching `Err` here, with exactly the
    /// defect this field exists to remove. `[moved to `validate` — 2026-08-09, Pete]`
    pub refusals: Vec<PlanRefusal>,
}

/// One positional bind. The compiler emits `$1`, `$2`, … in this order and never renders a value
/// into the SQL text.
#[derive(Debug, Clone)]
pub enum QueryBind {
    Profile(ProfileId),
    Uuids(Vec<Uuid>),
    // Text / Int / Embedding are the scalar and vector args the real act fragments bind in Task 10
    // (edge labels, a funnel width, a query embedding). Declared now so the interface Task 10
    // depends on is fixed; they are public API, so being unconstructed in this task is not dead code.
    Text(String),
    Int(i64),
    /// Rendered through `format_pgvector` and bound as `$n::vector`, the same treatment `search_wide`
    /// gives its embedding.
    Embedding(Vec<f32>),
    /// `text[]` — the selection core's `p_doc_types` and `p_tags`.
    ///
    /// Distinct from [`Self::Text`] rather than a comma-joined string, because a joined string is a
    /// value the caller can put a comma inside. The list endpoint carries that hazard deliberately
    /// (a GET's query string cannot encode sequences) and states it as a constraint on the tag
    /// vocabulary; this transport is a JSON body and has no such excuse.
    Texts(Vec<String>),
    /// A single `uuid` — the selection core's `p_owner_profile`. [`Self::Uuids`] is the `uuid[]`
    /// slots, and binding a one-element array into a scalar slot would not typecheck.
    Id(Uuid),
    /// `jsonb` — the selection core's `p_facets`, a list of `{key, value}` objects.
    Json(serde_json::Value),
}

/// The placeholder function name the fallback act body targets — not any act body: the three find
/// acts emit real ungated cores, and no other act reaches this builder. It intentionally does NOT
/// exist in the schema, so an accidentally-executed skeleton fails loudly rather than returning a
/// silently-empty or silently-wrong result.
const PLACEHOLDER_FN: &str = "__temper_unbound_act";

/// The ungated cores this builder emits for the find acts. Named here as constants so the match in
/// `emit_act_body` cannot drift from what `CALLABLE_FRAGMENTS` maps to.
///
/// These apply NO visibility gate — they are handed the verdict. That is the entire point: the
/// gated twins each compute `resources_visible_to` internally, and the planner does not dedupe those
/// across call sites, so an N-stage composition would pay N recursive team closures. Nothing here
/// may call them without going through `emit_ungated_core_call` (private to this module — these
/// constants are exported for COMPARISON, never as an invitation to emit a call from elsewhere).
///
/// **`pub` because one question outside this module has to be answered against the same constants.**
/// `query_read`'s `wants_a_vector` asks *does any stage in this plan search by vector*, which is
/// `served_by` → [`temper_core::types::query::emitted_fragment_for`] → is it the wide core. It held
/// its own hardcoded `"search_wide"` instead, and that literal did not travel when `served_by` was
/// repointed at the twins on 2026-08-12: it silently answered `false` for every act, which skips
/// server-side embedding and refuses every find-about stage `EmbeddingUnavailable`. A name spelled
/// in a third place is what produced that, so the export exists to stop there being a third place.
pub const EMIT_FIND_EXACT: &str = "__temper_ungated_find_exact";
pub const EMIT_FIND_WIDE: &str = "__temper_ungated_find_wide";
pub const EMIT_FIND_RESOURCES_WITH: &str = "__temper_ungated_find_resources_with";
/// The walk's depth, FIXED — not a bound term and not a caller input.
///
/// `[ruled — 2026-08-14, Pete]` *"Depth 3 is too large for a neighborhood traversal of this kind"*
/// is a claim about what `follow-from` MEANS, which puts it in gamma's category rather than
/// `limit`'s: `BoundTerm` does not grow a `Depth` variant, and no widening of
/// `accepts_bound_terms` reaches this constant. The measurement behind the 2 is spec §5 — path rows
/// went 4,134 to 33,684 for one extra hop.
///
/// `[amended — 2026-08-17]` The ruling stands; the inventory beside it did not. This said
/// `accepts_bound_terms` *"stays `[Limit]`"*, and `20260817000020` added `Offset` to it
/// (`registry.rs:361`). That is the ruling holding rather than bending: a page boundary says
/// nothing about what the act MEANS, only about which part of its answer you are looking at, so
/// `offset` joined `limit`'s category exactly as the ruling sorts them. Depth did not move, and the
/// list is no longer restated here — a copy of it is what went stale.
///
/// It lives HERE, at the one place a walk is emitted, rather than as a fragment default, so there is
/// exactly one answer to "how deep is a neighbourhood" and it is in the compiler that decides it.
const WALK_DEPTH: &str = "2";

/// The walk's decay rate, FIXED.
///
/// `orders_by.means` is a fixed sentence describing what `graph_score` IS. A caller-set rate makes
/// it *"decayed at whatever rate you asked for"* — still true, no longer interpretable — and nothing
/// would stop `gamma > 1`, which inverts the meaning so that distant nodes outscore near ones under
/// a declaration saying "best path". Matches the value the retired `unified_search` used.
const WALK_GAMMA: &str = "0.5::double precision";

/// The provenance-carrying walk (`20260814000030`). Unlike the three above it returns a THIRD
/// column, `via`, which rides the stage contract beside `id`/`kind`/`quantity` — see the final
/// select's shared column list, which every act stage and every tally arm must match.
pub const EMIT_FOLLOW_FROM: &str = "__temper_ungated_follow_from";

/// The survey act (`20260816000020`). Unlike the other cores, this one takes BOTH `p_visible_ids`
/// (the hoisted resource gate) and `p_principal` (for `wayfind_region_scores`'s internal region
/// gate). It produces resources with `region_score` as the quantity and `region_id`/`affinity` as
/// disclosed columns. See `CoreCall::Survey` and the migration for the two-gate design.
pub const EMIT_SURVEY: &str = "__temper_ungated_survey";

/// **Every emitted identifier is double-quoted**, here and at each CTE definition and reference.
/// `[fixed — 2026-08-09]` `StageName::parse` admits `[a-z][a-z0-9_]{0,62}`, which includes `both`,
/// `all`, `order` and every other reserved word in lower case; unquoted, each is a syntax error a
/// caller sees as a 500 — a well-formed composition refused by nothing and failing at the database
/// for a reason the contract does not predict. Quoting closes the class rather than one word, and it
/// is safe precisely BECAUSE the parse-only constructor guarantees the shape: no quote, no dot, no
/// case to fold.
///
/// The hoisted visibility relation. **Deliberately unreachable as a stage name**: `StageName::parse`
/// requires an ASCII lowercase first character, so no caller-chosen stage can ever shadow it. This
/// identifier now carries the RBAC verdict every stage reads, and a collision with it would make
/// authorization turn on a naming accident — closed by construction rather than by a check.
const VIS_CTE: &str = "__temper_vis";

/// The visible-id set, as the cores take it. One row, one `uuid[]`, built once for the whole
/// statement — `ARRAY(SELECT id FROM …)` per stage would compute the gate once but rebuild the array
/// N times.
///
/// `array_agg` over zero rows yields NULL rather than `'{}'`, and that is the correct answer here:
/// the cores read a NULL `p_visible_ids` as admitting nothing, so a principal who sees nothing gets
/// nothing. Fail-closed, and the same value either spelling would produce through `unnest`.
const VISIBLE_IDS: &str = "(SELECT ids FROM __temper_vis)";

/// The unusable tally for a stage whose input needs none — no input at all, an upstream set, or an
/// anchor pair.
///
/// A literal zero rather than NULL, and the two are not interchangeable here:
/// [`temper_core::types::query::StageResult::input_unusable`] is a non-null count, so NULL would
/// have to be rendered as *something* on the wire and the only candidate is zero anyway. Saying it
/// in the SQL keeps the claim where a reader of the statement can see it.
///
/// For an UPSTREAM set the zero is a fact, not a default: the set is what a visibility-gated
/// fragment returned, so every id in it was usable by construction, and re-checking would cost a
/// gate call to confirm a known answer.
///
/// For an ANCHOR it is a **named under-report**. Anchor readability for BOTH kinds — cogmap and
/// context — is decided inside the fragment against `p_anchor_reader` (`anchor_readable_by_profile`),
/// per ADJ-1 `[2026-08-10, Pete]`; finding out here instead would mean a second recursive team
/// closure, which is the exact cost the `vis` hoist exists to avoid. An unreadable anchor comes
/// back as an `empty` stage rather than as one unusable id — the disposition the existence-oracle
/// rule prescribes anyway (an id YOU supplied that you cannot see is `empty`, never `withheld`) —
/// and the unusable tally deliberately does not count it: the loss is one counter, not the
/// disclosure, and the vis-hoist cost argument stands.
const NO_UNUSABLE: &str = "0::bigint";

/// The principal, always `$1` — `compile` pushes it first, before any per-stage bind. The cores read
/// it for **anchor readability, both kinds** — cogmap and context, decided by
/// `anchor_readable_by_profile` since migration `20260810000010_anchor_readability_both_kinds.sql`
/// (ADJ-1 `[2026-08-10, Pete]`). That is one boolean per call and a property of no row, so it cannot
/// ride in `VISIBLE_IDS`. It is not a visibility gate: this parameter exists for anchor readability
/// only, and must never gain another use.
const PRINCIPAL_BIND: &str = "$1";

/// The ANN candidate width handed to the wide core. Carried over from `/api/search`'s own draw and
/// matched by that function's `hnsw.ef_search` pin (200 >= 100) — a k above the pin would make
/// `LIMIT p_k` unreachable and truncate the draw silently.
const ANN_DRAW_K: i32 = 100;

/// Compile a validated composition into one statement. `principal` is bound as `$1` and drives the
/// single visibility relation every stage joins.
///
/// **The query vector is NOT a parameter here.** `[2026-08-12]` It was one, on the reasoning that
/// `Intention` was a wire type carrying only `query` and `embedded` and that *"putting a 768-float
/// array in the envelope would be a contract change nobody asked for."* Spec ⟨7⟩ overturned both
/// halves: the intention moved onto each `ActInvocation` and now carries its own
/// `embedding: Option<Vec<f32>>`, so a stage's vector arrives with the stage.
///
/// Removing the parameter is the point, not tidying. With the vector on the node, a `compile` that
/// ALSO accepted one would have two sources for one fact — and the prev-else-context fallback this
/// whole contract refuses is exactly what two sources for one fact decays into.
///
/// **An absent `intention.embedding` means the vector could not be obtained, not that the caller
/// declined to send one.** `[amended — 2026-08-08, Pete]` Embedding on the caller's behalf is this
/// surface's job: the CLI links temper-ingest and computes vectors client-side, while the ruby gem,
/// the TypeScript package and MCP structurally cannot, so a caller-must-embed rule would deny
/// `find-about-*` to every non-CLI client. The caller fills each stage's missing vector **before**
/// building the `ValidatedComposition`, exactly as `/api/search` does — which is also why this
/// function needs no embedding argument to honour the rule.
///
/// So a `None` at this point has already survived that attempt, and a `find-about-*` stage
/// refuses with `EmbeddingUnavailable` — the contract's ONE runtime refusal. Still a refusal
/// rather than a silent NULL bind: the stage holds a well-formed question it cannot answer, and
/// searching on nothing returns a list that reads like an answer. That is why this returns a
/// `Result`: a refusal here is a disposition, not a panic.
pub fn compile(
    v: &ValidatedComposition,
    principal: ProfileId,
) -> Result<CompiledQuery, PlanRefusal> {
    let mut binds: Vec<QueryBind> = vec![QueryBind::Profile(principal)];
    let mut cte_names: Vec<(String, String)> = Vec::new();
    let mut ctes: Vec<String> = Vec::new();

    // The visibility relation, computed once — decision 019fcd13: one query time, one visibility
    // computation, no per-stage recomputation. `MATERIALIZED` is an optimization fence, not merely
    // "compute once".
    //
    // `[CONSUMED — 2026-08-08]` Every act stage now reads this and nothing else for its verdict, via
    // `emit_ungated_core_call`. It was emitted-and-unread from PR #663 until the ungated cores
    // landed (`20260808000030`); the property the whole hoist exists for is that
    // `resources_visible_to` appears exactly ONCE in the emitted statement no matter how many stages
    // there are, which is what `the_visibility_relation_is_computed_once_no_matter_how_many_stages`
    // asserts.
    //
    // Aggregated to a single `uuid[]` because a CTE cannot be passed to a function — the value is
    // how one verdict reaches N stages. `resource_id` is `resources_visible_to`'s only column; the
    // previous `SELECT id FROM …` here named a column that function does not return, which nothing
    // caught because nothing read the CTE.
    ctes.push(format!(
        "{VIS_CTE} AS MATERIALIZED (\n  SELECT array_agg(resource_id) AS ids FROM \
         resources_visible_to({PRINCIPAL_BIND})\n)"
    ));

    let mut tallies: Vec<StageTally> = Vec::new();
    let mut refusals: Vec<PlanRefusal> = Vec::new();
    // Stages whose output cannot be trusted to be COMPLETE: they refused, or something upstream of
    // them did and they answered over less than the truth. See [`tainted_by_refusal`] for why this
    // is a different question from "did this stage refuse", and why only one position reads it.
    let mut tainted: BTreeSet<String> = BTreeSet::new();
    for node in v.ordered() {
        let (name, body, unusable) = match node {
            StageNode::Act(inv) => {
                let emitted = emit_act_body(inv, &mut binds, &mut refusals)?;
                (inv.name.as_str(), emitted.body, emitted.unusable)
            }
            // A combinator's inputs are upstream stages, so nothing it was handed can be unusable.
            StageNode::Combine(cn) => {
                let body = match subtrahend_refusal(cn, &tainted) {
                    Some(refusal) => {
                        let body = refused_body(REFUSED_DIFFERENCE);
                        refusals.push(refusal);
                        body
                    }
                    None => emit_combine_body(cn),
                };
                (cn.name.as_str(), body, NO_UNUSABLE.to_string())
            }
        };
        if tainted_by_refusal(node, &refusals, &tainted) {
            tainted.insert(name.to_string());
        }
        ctes.push(format!("\"{name}\" AS (\n{body}\n)"));
        cte_names.push((name.to_string(), name.to_string()));
        tallies.push(StageTally {
            stage: name.to_string(),
            unusable,
        });
    }

    let sql = format!("WITH {}\n{}", ctes.join(",\n"), final_select(v, &tallies));
    Ok(CompiledQuery {
        sql,
        binds,
        cte_names,
        refusals,
    })
}

/// What one stage contributes to the statement: its CTE body, and the scalar expression that counts
/// the ids it was handed and could not use.
struct EmittedAct {
    body: String,
    unusable: String,
}

/// One stage's entry in the disclosure half of the final select.
struct StageTally {
    stage: String,
    /// A SQL scalar expression yielding `bigint`. See [`NO_UNUSABLE`] for when it is a literal zero
    /// and why that is a fact rather than a default.
    unusable: String,
}

/// The body of a stage that REFUSED — the stage-contract shape, and no rows.
///
/// **Empty, never absent.** A stage bounded by this one takes `ARRAY(SELECT id FROM <it>)`, which is
/// `'{}'` — bounded to nothing — and never the `NULL` the fragments read as unbounded. Collapsing
/// those two would turn a failed stage into a global search: a different question, answered
/// confidently, with a full page of plausible results and nothing to distinguish it from a real
/// answer.
///
/// It emits no function call at all, deliberately. Binding NULL into the vector core would run a
/// similarity search against nothing and return a list that reads like an answer, which is the
/// distinction between a refusal and an honest empty collapsing at exactly the point it matters.
fn refused_body(act: &str) -> String {
    format!(
        "  -- act: {act} REFUSED (no rows, and an EMPTY set for anything bounded by it)\n  \
         SELECT NULL::uuid AS id, NULL::text AS kind, NULL::double precision AS quantity, \
         NULL::jsonb AS via WHERE false"
    )
}

/// What [`refused_body`] names in place of an act when the refusing stage runs no act.
const REFUSED_DIFFERENCE: &str = "difference (subtrahend refused)";

/// Does this stage's output have to be assumed INCOMPLETE?
///
/// **Not the same question as "did it refuse", and the gap between them is the whole reason this
/// exists.** A stage that refused produces nothing. A stage merely *downstream* of a refusal
/// produces something — it ran, it answered, and its answer is smaller than the truth because one
/// of its inputs was empty when it should not have been. Both are untrustworthy as a SUBTRAHEND;
/// only the first is visible in `refusals`.
///
/// Computed as the transitive closure over upstream edges, which costs nothing here because
/// `v.ordered()` is topological: every upstream of a node has already been decided by the time the
/// node is reached.
///
/// **It changes no stage's own disposition.** A tainted stage still reports whatever it produced —
/// answered, or an honest empty. The set is read at exactly one position, [`subtrahend_refusal`],
/// because that is the only anti-monotone position in the contract. Widening it into a general
/// "refusals propagate downstream" rule would contradict the contract's stated behaviour, which is
/// that a stage bounded by a refused one is bounded to nothing and answers normally.
fn tainted_by_refusal(
    node: &StageNode,
    refusals: &[PlanRefusal],
    tainted: &BTreeSet<String>,
) -> bool {
    let name = node.name();
    refusals.iter().any(|r| r.stage.as_ref() == Some(name))
        || node
            .upstream_names()
            .iter()
            .any(|u| tainted.contains(u.as_str()))
}

/// The refusal a `difference` inherits when what it was told to subtract is not knowable.
///
/// Returns `None` for every other op and for a sound subtrahend. See
/// [`RefusalReason::SubtrahendRefused`] for why an empty right arm is the one case where the
/// contract's empty-never-absent rule produces the maximal answer instead of the minimal one.
///
/// **Only `inputs[1]` is consulted.** The minuend arm is monotone — `∅ − B` is `∅`, the ordinary
/// bounded-to-nothing outcome — so a taint there is already governed correctly by the incumbent
/// rule, and refusing on it would report a refusal for a stage that gave the honest answer.
fn subtrahend_refusal(
    cn: &temper_core::types::query::CombineNode,
    tainted: &BTreeSet<String>,
) -> Option<PlanRefusal> {
    if !cn.op.is_ordered() {
        return None;
    }
    // Index 1 by the arity rule `validate` enforces; `get` rather than `[1]` because `compile` is
    // public and does not require its caller to have run `validate` on the same tick.
    let subtrahend = cn.inputs.get(1)?;
    if !tainted.contains(subtrahend.as_str()) {
        return None;
    }
    Some(PlanRefusal {
        stage: Some(cn.name.clone()),
        reason: RefusalReason::SubtrahendRefused,
        detail: format!(
            "stage `{}` subtracts `{}`, which refused or answered over an incomplete set; \
             subtracting nothing would return the whole of `{}` and read as an answer",
            cn.name.as_str(),
            subtrahend.as_str(),
            cn.inputs
                .first()
                .map(|s| s.as_str())
                .unwrap_or("the minuend"),
        ),
    })
}

/// A placeholder act body in the `(id, kind, quantity, via)` stage-contract shape. IDs only cross a
/// stage boundary — a downstream stage references its upstream as `SELECT id FROM <stage>`, never a
/// quantity and never `via`, which is what keeps `no-cross-act-ranking` structural (spec §4).
///
/// **Every act stage projects all four columns, and the ones that are not walks project
/// `NULL::jsonb AS via`** `[2026-08-14]`. Uniform rather than per-act, because [`final_select`]
/// shares one column list across hit arms, tally arms and the empty fallback — a stage missing a
/// column would fail at UNION time with an error naming the arity rather than the act.
fn emit_act_body(
    inv: &temper_core::types::query::ActInvocation,
    binds: &mut Vec<QueryBind>,
    refusals: &mut Vec<PlanRefusal>,
) -> Result<EmittedAct, PlanRefusal> {
    let act = act_name(&inv.act);
    // This stage's own question. `[2026-08-12]` Was threaded in from the composition; spec ⟨7⟩
    // put it on the node, so a sibling stage's intention can no longer answer for this one.
    let intention = inv.intention.as_ref();
    let embedding = intention.and_then(|i| i.embedding.as_deref());

    let (narrowing, unusable) = narrowing_for(inv, binds)?;
    let emitted = |body: String| EmittedAct {
        body,
        unusable: unusable.clone(),
    };
    let (anchor_table, anchor_id) = narrowing.anchor();
    let (anchor_table, anchor_id) = (anchor_table.to_string(), anchor_id.to_string());
    let (limit, offset) = paging_for(inv, binds);
    let doc_type = doc_type_for();

    // The find acts narrow and never seed, so each takes the bound expression. A seed reaching one
    // is a validator/compiler disagreement rather than a caller error — `bound_expr` says so and
    // errors instead of quietly narrowing.
    let bound_for_find = |inv: &temper_core::types::query::ActInvocation| {
        if narrowing.has_seed() {
            return Err(PlanRefusal {
                stage: Some(inv.name.clone()),
                reason: RefusalReason::UnsupportedSeedKind,
                detail: "this act narrows within a set and cannot grow from one; the validator \
                         should have refused this stage as `unsupported_seed_kind` before \
                         compilation"
                    .to_string(),
            });
        }
        Ok(narrowing.bound_expr().to_string())
    };

    match fragment_for(&inv.act) {
        Some(EMIT_FIND_EXACT) => {
            let bound = bound_for_find(inv)?;
            let q = intention.map(|i| i.query.as_str()).ok_or_else(|| {
                missing_question(
                    inv,
                    "find-exact needs the intention's query text — it becomes `p_query`, and there \
                     is nowhere else to source it. This stage carries no intention",
                )
            })?;
            let qi = binds.len() + 1;
            binds.push(QueryBind::Text(q.to_string()));
            let call = emit_ungated_core_call(&CoreCall::Find {
                core: EMIT_FIND_EXACT,
                doc_type: doc_type.clone(),
                intent_args: format!("${qi}"),
                bound: &bound,
                anchor_table: &anchor_table,
                anchor_id: &anchor_id,
                limit: &limit,
                offset: &offset,
            });
            Ok(emitted(format!(
                "  -- act: {act} -> {EMIT_FIND_EXACT}\n  \
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 fts_norm::double precision AS quantity, NULL::jsonb AS via\n    \
                 FROM {call}"
            )))
        }
        Some(EMIT_FIND_RESOURCES_WITH) => {
            // **No intention, and that is not an omission to guard against.** This is the one act
            // that asks the corpus nothing, so there is no `p_query` and no embedding to source;
            // the shape pass's intention requirement lists three acts by name and this is not among
            // them, so a plan omitting one here is well-formed rather than tolerated.
            //
            // **`NULL::double precision AS quantity`** — the stage contract is `(id, kind,
            // quantity)` and a selection has no quantity, exactly as `refused_body` has none. That
            // NULL is why a stage running this act is refused in `returns` as `StageNotReturnable`:
            // the assembler scores rows by their act's `orders_by`, and this one has none, so a
            // returned selection would come back `answered` over an empty list.
            //
            // No `bound_for_find` and no paging: the fragment has neither slot, because narrowing a
            // selection by an upstream set is `CombineOp::Intersect` and a selection that truncates
            // is a sample. An anchor IS honoured, which is why `narrowing.anchor()` is read.
            // Order matters and is not cosmetic: `binds` is positional, so the narrowing binds must
            // be pushed before the open-key one to match the `$n` indices each renders.
            let narrowings = selection_narrowings_for(inv, binds);
            let properties = resource_properties_for(inv, binds);
            let call = emit_ungated_core_call(&CoreCall::Selection {
                core: EMIT_FIND_RESOURCES_WITH,
                narrowings,
                anchor_table: &anchor_table,
                anchor_id: &anchor_id,
                properties,
            });
            Ok(emitted(format!(
                "  -- act: {act} -> {EMIT_FIND_RESOURCES_WITH}\n  \
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 NULL::double precision AS quantity, NULL::jsonb AS via\n    \
                 FROM {call}"
            )))
        }
        Some(EMIT_FIND_WIDE) => {
            let bound = bound_for_find(inv)?;
            // `[amended — 2026-08-08, Pete]` This used to refuse with "the server does not embed on
            // the caller's behalf". It does embed, and it has to: the CLI links temper-ingest and
            // computes vectors client-side, but the ruby gem, the TypeScript package and MCP carry
            // no embedding ability at all, so refusing here would deny this act to every non-CLI
            // client. `/api/search` already solved it with `embed_query_if_missing`.
            //
            // So by the time a `None` reaches this line, the caller supplied no vector AND the
            // server's own attempt failed. That is `EmbeddingUnavailable` — the contract's one
            // runtime refusal — not `MissingIntention`, which is about the caller omitting a
            // QUESTION and is decided statically long before here.
            //
            // Still a refusal rather than a NULL bind: the stage holds a well-formed question it
            // cannot answer, and searching on nothing would return a list that reads like an answer.
            let Some(emb) = embedding else {
                refusals.push(PlanRefusal {
                    stage: Some(inv.name.clone()),
                    reason: RefusalReason::EmbeddingUnavailable,
                    detail: "a find-about-* stage needs a query embedding; none was supplied and \
                             the server could not compute one"
                        .to_string(),
                });
                return Ok(emitted(refused_body(&act)));
            };
            let ei = binds.len() + 1;
            binds.push(QueryBind::Embedding(emb.to_vec()));
            let ki = binds.len() + 1;
            binds.push(QueryBind::Int(i64::from(ANN_DRAW_K)));
            let call = emit_ungated_core_call(&CoreCall::Find {
                core: EMIT_FIND_WIDE,
                doc_type: doc_type.clone(),
                intent_args: format!("${ei}::vector, ${ki}::int"),
                bound: &bound,
                anchor_table: &anchor_table,
                anchor_id: &anchor_id,
                limit: &limit,
                offset: &offset,
            });
            Ok(emitted(format!(
                "  -- act: {act} -> {EMIT_FIND_WIDE}\n  \
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 vec_norm::double precision AS quantity, NULL::jsonb AS via\n    \
                 FROM {call}"
            )))
        }
        Some(EMIT_FOLLOW_FROM) => {
            // **The seed slot, and the reason `narrowing_for` reads the relation at all.** Routing
            // a seed into `p_bound_ids` compiles a walk that can only return what was already in
            // its own seed set — a stage that looks like it worked and can never reach a neighbour.
            //
            // A walk with no seed is not refused here: `p_seed_ids` NULL reaches nowhere and
            // returns zero rows, which is the honest answer to "walk from nothing" and matches what
            // the fragment does. The act's `accepts_seeds` is what makes a seed expressible; making
            // one MANDATORY is a different rule and is not one anything declares.
            let (edge_kinds, labels, edge_properties) = edge_filter_for(inv, binds);
            let call = emit_ungated_core_call(&CoreCall::Walk {
                core: EMIT_FOLLOW_FROM,
                seeds: narrowing.seed_expr(),
                depth: WALK_DEPTH,
                gamma: WALK_GAMMA,
                edge_kinds,
                labels,
                // The third axis (`20260815000010`). It constrains which edge may be TRAVERSED, so
                // it rides into the walk beside the other two rather than filtering what came out:
                // a node admitted for a matching edge and then walked through a non-matching one
                // has answered a different question and looks like it narrowed.
                edge_properties,
                // Constrains the WHOLE walk, intermediates included — the fragment applies it
                // where visibility is applied. NULL is unbounded here, which is the opposite
                // polarity from the visible set beside it.
                bound: narrowing.bound_expr(),
                limit: &limit,
                // `[2026-08-17]` The page NUMBER, from the same `paging_for` the find arms read —
                // so the offset that runs is the offset `applied_terms` reports, and the walk
                // cannot page differently from what the response says it paged.
                offset: &offset,
            });
            // **`via` crosses into the stage contract as a fourth column.** Every other act emits
            // `NULL::jsonb` for it — see `final_select`, which shares one column list across hit
            // arms, tally arms and the empty fallback.
            Ok(emitted(format!(
                "  -- act: {act} -> {EMIT_FOLLOW_FROM}\n  \
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 graph_score::double precision AS quantity, via\n    \
                 FROM {call}"
            )))
        }
        Some(EMIT_SURVEY) => {
            // Survey requires an intention (enforced by the shape pass). A `None` embedding here
            // means the server's embed attempt failed — `EmbeddingUnavailable`, same as find-about.
            let Some(emb) = embedding else {
                refusals.push(PlanRefusal {
                    stage: Some(inv.name.clone()),
                    reason: RefusalReason::EmbeddingUnavailable,
                    detail: "a survey stage needs a query embedding; none was supplied and \
                             the server could not compute one"
                        .to_string(),
                });
                return Ok(emitted(refused_body(&act)));
            };
            let ei = binds.len() + 1;
            binds.push(QueryBind::Embedding(emb.to_vec()));
            // `regions_n` is bound from the `Regions` bound term, clamped to the ceiling of 20 by
            // `applied_terms`. It is a funnel width (how many regions to match), not a row limit.
            let applied = temper_core::types::query::declaration(&inv.act)
                .map(|d| temper_core::types::query::applied_terms(&inv.terms, &d))
                .unwrap_or_default();
            let regions_n = match applied.get(&BoundTerm::Regions) {
                Some(v) => {
                    let idx = binds.len() + 1;
                    binds.push(QueryBind::Int(*v));
                    format!("${idx}::int")
                }
                None => "NULL::int".to_string(),
            };
            let call = emit_ungated_core_call(&CoreCall::Survey {
                core: EMIT_SURVEY,
                embedding: format!("${ei}::vector"),
                regions_n: &regions_n,
                anchor_table: &anchor_table,
                anchor_id: &anchor_id,
            });
            Ok(emitted(format!(
                "  -- act: {act} -> {EMIT_SURVEY}\n  \
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 region_score::double precision AS quantity, NULL::jsonb AS via\n    \
                 FROM {call}"
            )))
        }
        // **Unreachable through `validate`, and emitted anyway.** `survey` used to reach here; it
        // left `CALLABLE_FRAGMENTS` because its fragment takes an argument no slot supplies
        // (`p_lens`), and `validate` now refuses it with `NotSeparablyReachable` before a
        // `ValidatedComposition` can exist. (`follow-from` was the other, and rejoined the map on
        // 2026-08-14 — its `p_depth`/`p_gamma` turned out to want constants rather than slots.)
        // `[2026-08-16]` survey rejoined too — `p_lens = NULL` is correct, not a slot. Every act
        // this arm could still catch is one that declared itself into `search_family()` with a
        // `served_by` this builder has no case for — an internal inconsistency, not a caller error.
        // It emits a function that deliberately does not exist so Postgres errors loudly, rather
        // than guessing a value and returning plausible rows.
        //
        // **NOTHING TESTS THIS ARM, and that is a second fact rather than the same one restated.**
        // `[declared — 2026-08-12]` "Unreachable through `validate`" and "no test exercises it" are
        // independent, and only the first was stated. The deleted
        // `the_unmodelled_acts_still_emit_the_absent_placeholder` used to compile a `follow-from`
        // plan straight through here and assert the emitted SQL carried `__temper_unbound_act`; its
        // replacement in `query_plan_compile.rs`,
        // `the_unmodelled_acts_are_refused_before_the_compiler_ever_sees_them`, calls `validate` and
        // never `compile` — correct for what it now asserts, and it leaves this arm with ZERO
        // exercising tests. So the drift guard is unwitnessed: an eighth act reaching here would
        // fault at Postgres exactly as designed, and no test in this workspace would have said so
        // first. Reaching it deliberately would take a `ValidatedComposition` built around
        // `validate`, which is the parse-don't-validate seam working as intended — hence declared,
        // not covered.
        _ => Ok(emitted(format!(
            "  -- act: {act} (placeholder body; this builder emits no fragment for it yet)\n  \
             SELECT id, kind, quantity, NULL::jsonb AS via FROM {PLACEHOLDER_FN}({})",
            narrowing.any_set_expr(),
        ))),
    }
}

/// Everything an ungated-core call needs that is NOT an authorization input.
///
/// Note what is absent from BOTH variants: there is no field for the visible-id set and none for
/// the anchor reader. That absence is the design — see [`emit_ungated_core_call`].
///
/// `[widened to an enum — 2026-08-14]` It was a struct shaped for the find cores' one signature.
/// `__temper_ungated_find_resources_with` has a different one — no intention, no bound, no paging,
/// and eight narrowing slots — so the choice was a second emitter or a second variant.
///
/// **A second emitter was the wrong answer**, and not on style grounds: the whole security property
/// of [`emit_ungated_core_call`] is that it is *the one place* `VISIBLE_IDS` and `PRINCIPAL_BIND`
/// are written, so no caller has a wrong set to pass. Two emitters would be two places, and the
/// second one would be the one nobody audits.
enum CoreCall<'a> {
    /// The find cores: an intention, a bound, one doc-type slot, and paging.
    Find {
        core: &'a str,
        /// The arm-specific arguments between the visible set and the narrowing slots: the bound
        /// query text for the exact arm, the embedding and draw width for the wide one.
        intent_args: String,
        bound: &'a str,
        /// The `p_doc_type` argument — a bound `$n::text`, or `NULL` where the plan declares none.
        /// The fragment's slot holds ONE value, which is why a multi-value doc-type filter is
        /// refused at validation rather than silently narrowed to its first element.
        doc_type: String,
        anchor_table: &'a str,
        anchor_id: &'a str,
        limit: &'a str,
        offset: &'a str,
    },
    /// The walk: a seed set, the two definitional constants, both edge axes, a bound, and a page.
    ///
    /// **`depth` and `gamma` are constants this compiler writes, not slots a caller fills.** The
    /// act fixes both — depth at 2, gamma at the rate its `orders_by` sentence describes — so they
    /// are `&'static str` literals here rather than bound parameters. The fragment takes them
    /// because the incumbent `search_graph_expand` signature has both and delegates through them
    /// (`20260814000030`), which is a fact about the SQL family rather than about the act.
    ///
    /// It is the only variant carrying BOTH a seed and a bound, which is what
    /// `ActInvocation::inputs` became a list for.
    Walk {
        core: &'a str,
        seeds: &'a str,
        depth: &'a str,
        gamma: &'a str,
        edge_kinds: String,
        labels: String,
        bound: &'a str,
        limit: &'a str,
        /// `EdgeFilter`'s third axis (`20260815000010`) — a bound `$n::jsonb` carrying the
        /// serialized predicate list, or `NULL::jsonb`.
        ///
        /// **The cast is never optional**, for the reason `slot` records: the widened fragment is
        /// reached by ARITY, and an untyped NULL in the ninth position cannot be resolved against a
        /// name that also has an eight-parameter form.
        edge_properties: String,
        /// The page offset (`20260817000020`) — a bound `$n::int`, or the literal `0`.
        ///
        /// **Declared last because it is rendered last.** Every field of this variant sits in its
        /// positional order, which is the one property that keeps a positional call auditable by
        /// reading the variant; `offset` is the tenth argument, so it is the tenth field. That is
        /// why it is not tucked in beside `limit` the way [`CoreCall::Find`] pairs them.
        ///
        /// **No cast here, and that is a decision rather than an omission** — the rule
        /// `edge_properties` records above is about UNTYPED arguments, and neither spelling
        /// `paging_for` produces is one. `$n::int` casts itself; the absent sentinel is the bare
        /// literal `0`, which Postgres types as `integer` at parse time (unlike `NULL` and a quoted
        /// string, which are `unknown`), so it matches `p_offset int` exactly. The literal `2` in
        /// `depth` has ridden this same call into `p_depth int` since `20260814000030` and is the
        /// standing witness that a bare integer resolves here. Arity settles it independently: the
        /// 10-arity carries no default on any parameter (`20260817000020`), the 9-arity carries
        /// none either, and the 8-arity defaults only parameters 5-8 (`20260815000010:139-142`), so
        /// nothing but the 10-arity can absorb ten arguments and there is no second candidate for
        /// an argument's type to choose between.
        offset: &'a str,
    },
    /// The selection core: eight narrowing slots, an anchor pair, and nothing else.
    ///
    /// No `bound` and no paging, matching the fragment: narrowing a selection by an upstream set is
    /// `CombineOp::Intersect`, and a selection that truncates is a sample. Both absences are the
    /// act's declaration (`accepts_bounds: [Context, Cogmap]`, `accepts_bound_terms: []`) showing
    /// through, so a slot appearing here later would mean the declaration moved first.
    Selection {
        core: &'a str,
        /// The eight narrowing expressions in signature order, each a bound `$n::type` or a typed
        /// `NULL`. Rendered as one string rather than nine fields because they are positional and
        /// uniform — eight `&'a str` fields would invite a caller to mis-order them silently, which
        /// is the failure this whole module is shaped against.
        narrowings: String,
        anchor_table: &'a str,
        anchor_id: &'a str,
        /// `ResourceFilter`'s open-key slot (`20260815000040`) — a bound `$n::jsonb` carrying the
        /// serialized predicate list, or `NULL::jsonb`.
        ///
        /// **Rendered LAST, deliberately not beside `narrowings`, and the reason is a hazard not a
        /// preference.** It is a narrowing and it belongs with them by meaning; but `p_facets` is
        /// also `jsonb`, and two adjacent `jsonb` narrowings is the one transposition this
        /// positional call could make without a type error. The signature puts it last so the type
        /// mismatch that catches every other mis-ordering catches this one too.
        ///
        /// **The cast is never optional**, for the reason `slot` records: the widened fragment is
        /// reached by ARITY, and an untyped NULL in the thirteenth position cannot be resolved
        /// against a name that also has a twelve-parameter form.
        properties: String,
    },
    /// The survey core: a query embedding, a funnel width, and an anchor pair. The only core that
    /// takes BOTH `VISIBLE_IDS` (the resource gate) and `PRINCIPAL_BIND` (the region gate inside
    /// `wayfind_region_scores`).
    ///
    /// **`p_emb` is bound from the stage's intention**, same as the find-about acts. Survey
    /// requires an intention (enforced by the shape pass); a `None` here means the server's embed
    /// attempt failed, which is `EmbeddingUnavailable` — same refusal, same reason.
    ///
    /// **`regions_n` is bound from the `Regions` bound term**, clamped to the ceiling of 20 by
    /// `applied_terms`. It is a funnel width (how many regions to match), not a row limit — survey
    /// still declines `Limit` because `Limit` means rows and survey's rows are resources, not
    /// regions.
    Survey {
        core: &'a str,
        embedding: String,
        regions_n: &'a str,
        anchor_table: &'a str,
        anchor_id: &'a str,
    },
}

/// **The one place an ungated core is called, and the only place its authorization inputs are
/// supplied.**
///
/// `VISIBLE_IDS` and `PRINCIPAL_BIND` are fixed inside this function and are not parameters of
/// [`CoreCall`], so a caller cannot hand a core the wrong set. That is deliberate and structural.
/// The CI tripwire (`audit-ungated-fragments.sh`) can pin *where* a core is called but never *what
/// is passed*, and the realistic bug is not a rogue call site — it is an approved one passing an
/// upstream stage's ids where the visible set belongs. CI green, RBAC bypassed, every returned row
/// still plausible. There is no wrong set to pass because there is no argument for it.
///
/// The two arrays are genuinely confusable: a narrowed stage has both in scope, both are `uuid[]`,
/// and they sit adjacent in the signature. `p_visible_ids` is an authorization verdict whose NULL
/// admits nothing; `p_bound_ids` is a scope whose NULL is unbounded.
///
/// `c.doc_type` fills the fragment's `p_doc_type` slot — `NULL` where the plan declares none, which
/// is what "no doc-type narrowing" means to the fragment.
///
/// `[fixed — 2026-08-09]` It was a hardcoded `NULL`, so a declared `doc_type` was accepted, ignored,
/// and then ECHOED BACK in `StageResult.narrowed_by` as though it had been applied. A caller asking
/// for sessions about X received anything about X, with the response's own disclosure telling them
/// it had been filtered. Found in review — and it is the exact silent-substitution shape this
/// surface exists against, made worse by the echo, because the echo is the evidence a caller would
/// use to trust the answer. Every other narrowing slot the plan can declare is now refused at
/// validation rather than dropped here.
/// The selection core takes the same two authorization inputs in the same two roles — the verdict
/// first, the anchor reader last — so widening this to a second variant did not widen what a caller
/// can influence.
///
/// `[corrected — 2026-08-14, found in adversarial review]` This read *"Both arms write
/// `VISIBLE_IDS` and `PRINCIPAL_BIND` here and nowhere else."* **That sentence is false**, and a
/// reviewer grepping to confirm it finds two more sites. The security property is intact — neither
/// is a core-call ARGUMENT POSITION, which is the only thing a caller could influence:
///
///   * the `__temper_vis` CTE in `compile` *defines* the verdict — it is where the gate is
///     computed, not where it is handed to a core;
///   * `unusable_tally` *reads* it, to count ids an upstream stage produced that this principal
///     cannot see.
///
/// Stated precisely because the imprecise version had already been copied into
/// `audit-ungated-fragments.sh`'s reviewed baseline as that guard's verdict, and a security guard
/// whose stated evidence fails on a grep is a guard people stop believing.
fn emit_ungated_core_call(c: &CoreCall) -> String {
    match c {
        CoreCall::Find {
            core,
            intent_args,
            bound,
            doc_type,
            anchor_table,
            anchor_id,
            limit,
            offset,
        } => format!(
            "{core}({VISIBLE_IDS}, {intent_args}, {bound}, {anchor_table}, {anchor_id}, \
             {PRINCIPAL_BIND}, {doc_type}, {limit}, {offset})"
        ),
        CoreCall::Selection {
            core,
            narrowings,
            anchor_table,
            anchor_id,
            properties,
        } => format!(
            "{core}({VISIBLE_IDS}, {narrowings}, {anchor_table}, {anchor_id}, {PRINCIPAL_BIND}, \
             {properties})"
        ),
        // No `PRINCIPAL_BIND`: the walk reads no anchor, so it needs no `p_anchor_reader`. The
        // visible set is still the first argument, and is still written only here.
        CoreCall::Walk {
            core,
            seeds,
            depth,
            gamma,
            edge_kinds,
            labels,
            bound,
            limit,
            edge_properties,
            offset,
        } => format!(
            "{core}({VISIBLE_IDS}, {seeds}, {depth}, {gamma}, {edge_kinds}, {labels}, {bound}, \
             {limit}, {edge_properties}, {offset})"
        ),
        // The ONLY core that takes BOTH `VISIBLE_IDS` and `PRINCIPAL_BIND`: `wayfind_region_scores`
        // applies its own region visibility by principal, so the ungated core needs `$1` for the
        // region gate beside the hoisted set for the resource gate. The `PRINCIPAL_BIND` is the
        // compiler's `$1` (always bound first), not a second id set — the one-emitter security
        // property holds because no caller can influence either argument here.
        CoreCall::Survey {
            core,
            embedding,
            regions_n,
            anchor_table,
            anchor_id,
        } => format!(
            "{core}({VISIBLE_IDS}, {PRINCIPAL_BIND}, {embedding}, {regions_n}, {anchor_table}, \
             {anchor_id})"
        ),
    }
}

/// What one stage does with the sets it was handed, and therefore which slot each belongs in.
///
/// # It was an enum, and the widening retired the reason
///
/// `[widened — 2026-08-14]` This was `Unbounded | Bound(_) | Seed(_) | Anchor{..}`, and its comment
/// said: *an enum rather than a struct of optional strings, because "never both" was previously
/// prose over three sibling fields — and prose over sibling fields is what let `bounds_mode` be
/// ignored in the first place.*
///
/// **"Never both" is exactly what `inputs: Vec<StageInput>` retires.** A bounded walk carries seeds
/// AND a bound at once, so an enum can no longer express a well-formed stage, and keeping that
/// sentence beside a type that now admits both would be the drift it warns about, one level up.
///
/// **What must NOT be retired with it is the property underneath: the RELATION picks the slot.**
/// That is what the enum was really protecting, and it survives here structurally rather than by
/// prose — [`narrowing_for`] is the only constructor, it writes each field from
/// `StageInput::relation()`, and no other code assembles one. A seed cannot reach `p_bound_ids`
/// because nothing but the `Seed` relation ever writes [`Self::seed`]. The failure this guards
/// against is concrete and shipped once: routing a seed into `p_bound_ids` compiles a traversal
/// that can only return what was already in its own seed set — a stage that looks like it worked
/// and can never produce a neighbour.
///
/// A field left `None` means the slot is UNBOUNDED — `NULL::uuid[]`, never `'{}'`. The fragments
/// read those two differently and conflating them is the substitution delta 3 forbids.
#[derive(Default)]
struct StageNarrowing {
    /// Narrow to within this set: the `p_bound_ids uuid[]` slot.
    bound: Option<String>,
    /// Grow from this set: the `p_seed_ids uuid[]` slot.
    seed: Option<String>,
    /// A `(table, id)` anchor pair — how the fragments take a cogmap or context scope. Holds
    /// exactly one id.
    anchor: Option<(String, String)>,
}

impl StageNarrowing {
    /// The `p_bound_ids` expression.
    fn bound_expr(&self) -> &str {
        self.bound.as_deref().unwrap_or("NULL::uuid[]")
    }

    /// The `p_seed_ids` expression. Read by `follow-from`'s arm, the only act that grows from a set.
    fn seed_expr(&self) -> &str {
        self.seed.as_deref().unwrap_or("NULL::uuid[]")
    }

    /// Whether this stage was handed a set to GROW from.
    ///
    /// Read by the find acts, which cannot: an act declaring `accepts_seeds: []` that reaches the
    /// compiler holding a seed is a validator/compiler contradiction, and the loud answer is the
    /// safe one. Previously this lived inside `bound_expr`'s `Result`; it moved out because the
    /// two questions stopped being the same one when a stage could hold both.
    fn has_seed(&self) -> bool {
        self.seed.is_some()
    }

    fn anchor(&self) -> (&str, &str) {
        self.anchor
            .as_ref()
            .map_or(("NULL", "NULL"), |(t, i)| (t.as_str(), i.as_str()))
    }

    /// The set expression a placeholder body echoes, whichever slot it came from.
    ///
    /// Seed before bound before anchor — an arbitrary order for a body that deliberately does not
    /// exist, kept total so the placeholder still names *something* the reader can recognize.
    fn any_set_expr(&self) -> &str {
        self.seed
            .as_deref()
            .or(self.bound.as_deref())
            .or(self.anchor.as_ref().map(|(_, i)| i.as_str()))
            .unwrap_or("NULL::uuid[]")
    }
}

/// Route the stage's input to the slot its RELATION and KIND belong in.
///
/// Two independent questions, and this function got one of them wrong until the relation moved
/// onto the edge.
///
/// **The relation** — bound or seed — decides which of two `uuid[]` parameters the set fills.
/// `[fixed — 2026-08-08]` This read `bounds_mode` NOWHERE. Every upstream set went to the
/// narrowing slot unconditionally, on the strength of a comment asserting an upstream set "is
/// always the array slot". So a seed compiled as a bound, and `follow-from` — the only seeding act
/// — would have returned only what was already in its seed set: a traversal that looks like it
/// worked and can never reach a neighbour. It was latent because `follow-from` emitted the
/// placeholder, and is latent still for a different reason — `validate` refuses the only act that
/// accepts a seed, so no seed reaches this function through `compile`. It fires the moment
/// `search_graph_expand` is both reachable and bound. The relation is now a field of
/// [`StageInput`] rather than an `Option` beside it, so there is a value to read instead of one to
/// invent.
///
/// **The kind** decides array-versus-anchor. `IdSet.kind` was previously ignored too and every set
/// went to `p_bound_ids`: `find-exact` and `find-about-within` declare
/// `accepts_bounds: [Resource, Context, Cogmap]`, so a caller handing a Cogmap set would validate
/// cleanly and then have cogmap uuids compared against `r.id`, returning zero rows with a 200 and
/// no disclosure that the narrowing was nonsense.
///
/// A `None` input is UNBOUNDED, which is `NULL::uuid[]` and never `'{}'`.
///
/// It also returns the stage's **unusable tally** — a scalar SQL expression counting how many of the
/// handed-in ids this stage could not use. It is computed here rather than beside the tallies
/// because this is the only place that knows both where the set came from and which bind holds it,
/// and re-deriving either downstream would mean sniffing the emitted string.
///
/// Only a caller-supplied RESOURCE set can be non-zero — see [`NO_UNUSABLE`] for why an upstream
/// set is a factual zero and why an anchor is a named under-report.
fn narrowing_for(
    inv: &temper_core::types::query::ActInvocation,
    binds: &mut Vec<QueryBind>,
) -> Result<(StageNarrowing, String), PlanRefusal> {
    let mut narrowing = StageNarrowing::default();
    // One tally per caller-supplied resource set, SUMMED. `input_unusable` is one number per stage
    // — "how many of the ids you handed me could not be used" — and a stage handing over two sets
    // has one answer to that question, not two. Reporting only the first would under-report in
    // exactly the direction that reads as reassurance.
    let mut tallies: Vec<String> = Vec::new();

    for input in &inv.inputs {
        // Read per input. The relation is a property of the edge and is the same whether the set
        // came from the caller or from an upstream stage — which is why it is not re-derived in
        // each arm below.
        let seeding = input.relation() == StageRelation::Seed;
        // **The one place a relation becomes a slot.** Every write to `seed`/`bound` goes through
        // here, which is what makes "the relation picks the slot" a property of this function
        // rather than of everyone remembering.
        let assign = |n: &mut StageNarrowing, expr: String| {
            if seeding {
                n.seed = Some(expr);
            } else {
                n.bound = Some(expr);
            }
        };

        narrowing_one(inv, input, binds, &mut narrowing, &mut tallies, &assign)?;
    }

    let unusable = if tallies.is_empty() {
        NO_UNUSABLE.to_string()
    } else {
        tallies.join(" + ")
    };
    Ok((narrowing, unusable))
}

/// One input, routed. Split out of [`narrowing_for`] only so the loop body stays readable; it holds
/// no policy of its own beyond the arms it always had.
fn narrowing_one(
    inv: &temper_core::types::query::ActInvocation,
    input: &StageInput,
    binds: &mut Vec<QueryBind>,
    narrowing: &mut StageNarrowing,
    tallies: &mut Vec<String>,
    assign: &dyn Fn(&mut StageNarrowing, String),
) -> Result<(), PlanRefusal> {
    match input {
        // Ids only — no quantity from the upstream stage is ever in scope here, which is what keeps
        // `no-cross-act-ranking` structural rather than policed. An upstream set is always resource
        // ids (that is what these acts produce), so it is always an array slot; WHICH array slot is
        // the relation's business.
        StageInput::Upstream { stage, .. } => {
            assign(
                narrowing,
                format!(r#"ARRAY(SELECT id FROM "{}")"#, stage.as_str()),
            );
            Ok(())
        }
        StageInput::Caller { ids, .. } => match ids.kind {
            IdKind::Resource => {
                let idx = binds.len() + 1;
                binds.push(QueryBind::Uuids(ids.ids.clone()));
                assign(narrowing, format!("${idx}::uuid[]"));
                tallies.push(unusable_tally(idx));
                Ok(())
            }
            // The anchor slot holds exactly ONE id. Spec §9 names this as an open cardinality gap
            // — "an IdSet holds N ids; an anchor slot holds one" — and the honest response to it is
            // a refusal, never silently anchoring on the first element, which would answer a
            // different question than the one asked and look like a successful narrowing.
            IdKind::Context | IdKind::Cogmap => {
                let table = match ids.kind {
                    IdKind::Cogmap => "kb_cogmaps",
                    _ => "kb_contexts",
                };
                // **Unreachable through `validate`, and asserted anyway.** The cardinality is a
                // static property of the plan and is refused there now
                // (`RefusalReason::AnchorTakesOneId`), so a caller's mistake arrives in the 400
                // beside every other refusal rather than costing every innocent stage in the
                // composition. This arm survives as the same kind of last line as `bound_expr`'s:
                // the two decisions live in different crates and could drift, and if they ever do,
                // the loud answer is the safe one. Same reason on both sides so the drift cannot be
                // a difference of opinion about WHAT is wrong.
                let [id] = ids.ids.as_slice() else {
                    return Err(PlanRefusal {
                        stage: Some(inv.name.clone()),
                        reason: RefusalReason::AnchorTakesOneId,
                        detail: format!(
                            "a {table} bound is served by the anchor pair, which holds exactly one \
                             id; this stage supplied {}. Anchoring on one of them would answer a \
                             different question than the one asked",
                            ids.ids.len()
                        ),
                    });
                };
                let ai = binds.len() + 1;
                binds.push(QueryBind::Uuids(vec![*id]));
                // The anchor is its own slot and is NOT routed by relation — a cogmap or context
                // set is served by the `(table, id)` pair whichever relation carried it, which is
                // how it behaved before the widening too.
                narrowing.anchor =
                    Some((format!("'{table}'::varchar"), format!("(${ai}::uuid[])[1]")));
                Ok(())
            }
            // A region set reaching a find act is already refused by the validator against
            // `accepts_bounds`; unbounded here rather than a second, divergent opinion about it.
            _ => Ok(()),
        },
    }
}

/// The two `EdgeFilter` axes, bound — a closed DDL enum beside open free text.
///
/// They are not merged, and `20260805`'s §8 says why: on live data a kind and a label are different
/// vocabularies, and one slot taking both would have to guess which a caller meant.
///
/// **An empty vector is NULL, never `'{}'`** — the fragment reads a NULL axis as "no narrowing" and
/// an empty array would be the same thing spelled a second way. `EdgeFilter` derives `Default`, so a
/// caller who names the filter and fills neither axis gets the same walk as one who names no filter,
/// which is the honest reading of "narrow by nothing".
///
/// **The label axis silently excludes UNLABELLED edges.** `kb_edges.label` is nullable in the DDL
/// and populated on every edge in prod today, so the case is real and unobserved; `label = ANY(...)`
/// is NULL for it, so the neighbour it reaches drops out. Correct, and stated rather than left to be
/// discovered — see spec §4.3.
fn edge_filter_for(
    inv: &temper_core::types::query::ActInvocation,
    binds: &mut Vec<QueryBind>,
) -> (String, String, String) {
    let Some(f) = &inv.edge_filter else {
        return (
            "NULL::text[]".to_string(),
            "NULL::text[]".to_string(),
            "NULL::jsonb".to_string(),
        );
    };
    // The kind is a closed enum on BOTH sides — Rust's `EdgeKind` and Postgres's `edge_kind` — and
    // the fragment compares `e.edge_kind::text`, so the wire spelling is what crosses. Taken from
    // serde rather than from a hand-written match, which would be a second place for the two
    // vocabularies to disagree.
    let kinds = f
        .edge_kinds
        .iter()
        .filter_map(|k| serde_json::to_string(k).ok())
        .map(|s| s.trim_matches('"').to_string())
        .collect();
    let labels = f.labels.clone();
    // **Scoped, so the borrow ends before the third axis binds.** The two text axes are bound
    // first and the `$n` indices below follow them, matching the fragment's parameter order — the
    // call is positional, so a bind emitted out of order would name the wrong slot.
    let (kinds_expr, labels_expr) = {
        let mut bind_texts = |values: Vec<String>| {
            if values.is_empty() {
                return "NULL::text[]".to_string();
            }
            let idx = binds.len() + 1;
            binds.push(QueryBind::Texts(values));
            format!("${idx}::text[]")
        };
        (bind_texts(kinds), bind_texts(labels))
    };
    // The third axis, through the one emitter both containers share. Called HERE rather than
    // earlier because it binds, and `binds.len() + 1` is positional — see the scope note above.
    let properties = properties_slot(&f.properties, binds);
    (kinds_expr, labels_expr, properties)
}

/// How many of the ids bound at `$idx` the principal cannot use — **invisible, nonexistent and
/// malformed as ONE number**, counted against the relation the statement already computed.
///
/// Conflated on purpose. Splitting them, or naming a counter for the invisible case alone, turns
/// the trace into a single-probe existence oracle: pass one id, read the counter, learn whether that
/// id exists. A nonexistent id and an invisible one are both simply absent from the visible set,
/// which is what makes one subtraction the honest answer rather than a compromise.
///
/// **`COALESCE` is load-bearing and not defensive.** `array_agg` over zero rows yields NULL, so a
/// principal who can see nothing would give `u.id = ANY(NULL)` → NULL → `NOT NULL` → NULL, and
/// `count(*)` over a NULL predicate counts nothing: every id unusable would tally as zero unusable,
/// which is the one direction this number must never fail in. Against `'{}'` the comparison is
/// false and all of them are counted.
fn unusable_tally(idx: usize) -> String {
    format!(
        "(SELECT count(*) FROM unnest(${idx}::uuid[]) AS u(id) \
         WHERE NOT (u.id = ANY(COALESCE({VISIBLE_IDS}, '{{}}'::uuid[]))))::bigint"
    )
}

/// The declared paging terms, bound.
///
/// Previously emitted as literal `NULL` / `0`, i.e. UNBOUNDED — so a plan declaring `limit: 10`
/// compiled to the entire match set. That is the wide-then-hydrate cost `20260806000020` measured
/// at 1,883 rows for a request asking for ten, and it also left the declared
/// `bound_ceilings: Limit => 50` unenforced. Worse in a chain, where an unlimited upstream feeds
/// `ARRAY(SELECT id FROM …)` into a bounded stage and drives its exhaustive branch over everything.
fn paging_for(
    inv: &temper_core::types::query::ActInvocation,
    binds: &mut Vec<QueryBind>,
) -> (String, String) {
    // The APPLIED values, not the asked-for ones: a term above its act's published ceiling is
    // clamped, and the clamped value is what must run. Read from the one function the assembler
    // also reports from, so the statement and `terms_applied` cannot claim different page sizes.
    let applied = temper_core::types::query::declaration(&inv.act)
        .map(|d| temper_core::types::query::applied_terms(&inv.terms, &d))
        .unwrap_or_default();
    let mut bind_term = |t: BoundTerm, absent: &str| match applied.get(&t) {
        Some(v) => {
            let idx = binds.len() + 1;
            binds.push(QueryBind::Int(*v));
            format!("${idx}::int")
        }
        None => absent.to_string(),
    };
    // NULL limit is the twins' own "unbounded"; offset defaults to 0, matching their signature.
    (
        bind_term(BoundTerm::Limit, "NULL"),
        bind_term(BoundTerm::Offset, "0"),
    )
}

/// The find fragments' `p_doc_type` slot, which is now **always `NULL`** — and that is deliberate,
/// not a regression to the defect this function was written to fix.
///
/// `[retired — 2026-08-14]` It used to bind a single declared `doc_type` from the invocation's
/// resource filter. `doc_type` is no longer a modifier on a find act: narrowing by what a resource
/// IS is the `find-resources-with` act, and `validate` refuses the filter on every other act
/// (`capability.rs`) rather than ignoring it. So nothing can reach here with one to bind.
///
/// **A slot nothing binds is exactly the silhouette of the original bug**, which is why this is a
/// documented constant rather than a hardcoded `NULL` inlined at the call site. `[fixed —
/// 2026-08-09]` `p_doc_type` was a literal `NULL` here while `validate` ACCEPTED a `doc_type`, so a
/// caller asking for sessions about X got anything about X and the response echoed the filter back
/// as evidence. The difference now is the other half of that pair: the accept is gone. A `NULL`
/// beside a refusal is honest; a `NULL` beside an accept is the defect.
///
/// The parameter itself stays in the fragment signatures. Removing it means new functions — DROP +
/// CREATE — which is shape-breaking and would halt the deploy at `--additive-only`, buying nothing:
/// the argument is already the fragment's own "no doc-type narrowing" value.
fn doc_type_for() -> String {
    "NULL".to_string()
}

/// Which of the two owner spellings a plan supplied, and whether it needs a bind at all.
///
/// An enum rather than an `Option<QueryBind>` because `@me` is a THIRD case that is neither "no
/// owner filter" nor "a value to bind" — it is a reference to a bind that already exists.
/// Collapsing it into either of the other two is exactly what produced the defect this type was
/// added to fix.
enum OwnerSlot {
    /// No owner narrowing, or the handle spelling (which fills the other slot).
    Absent,
    /// A profile id the caller named literally.
    Bind(QueryBind),
    /// `@me` — the caller's own profile, resolved at execution through `PRINCIPAL_BIND`.
    Principal,
}

/// The selection core's eight narrowing arguments, in signature order, each bound or a typed `NULL`.
///
/// **A `NULL` here narrows NOTHING**, which is the opposite polarity from the `p_visible_ids` these
/// arguments sit beside. That asymmetry is why the visible set is not among them and cannot be:
/// this function's whole job is turning absent narrowings into permissive `NULL`s, and one slip
/// applying that rule to the verdict would open the corpus.
///
/// Order is the fragment's, and it is positional — the one real hazard in this function. It is
/// mitigated by the arguments being typed differently enough that a transposition does not
/// typecheck (`text[]`, `jsonb`, `text`, `uuid`), which is a property of the SIGNATURE rather than
/// of care here, and by `the_selection_narrowings_bind_in_signature_order` asserting the rendering
/// directly.
fn selection_narrowings_for(
    inv: &temper_core::types::query::ActInvocation,
    binds: &mut Vec<QueryBind>,
) -> String {
    let f = inv.resource_filter.clone().unwrap_or_default();

    // A bound `$n::type`, or a typed NULL when the caller declared nothing for this slot. The cast
    // is never optional: an untyped NULL is ambiguous against a DEFAULTed parameter list, which is
    // the same reason `20260808000030` casts `NULL::uuid[]` explicitly.
    let mut slot = |bind: Option<QueryBind>, ty: &str| -> String {
        match bind {
            Some(b) => {
                binds.push(b);
                format!("${}::{ty}", binds.len())
            }
            None => format!("NULL::{ty}"),
        }
    };

    let facets = (!f.facets.is_empty()).then(|| {
        QueryBind::Json(serde_json::Value::Array(
            f.facets
                .iter()
                .map(|p| serde_json::json!({ "key": p.key, "value": p.value }))
                .collect(),
        ))
    });
    // `owner` is ONE wire field and TWO fragment slots, because the incumbent resolves it two ways:
    // `@me` becomes the caller's profile id, anything else matches a handle
    // (`substrate_read.rs`'s `owner_self` / `owner_handle` pair).
    //
    // `[fixed — 2026-08-14, found in adversarial review]` **`@me` fell through to the HANDLE slot
    // and matched nothing.** Handles carry no sigil — `SELECT handle FROM kb_profiles` yields
    // `system`, not `@system` — so `p.handle = '@me'` is unsatisfiable, and the most likely owner
    // value anyone writes selected an empty set while `narrowed_by` echoed it back as an applied
    // narrowing. That is the silent question substitution this act exists to remove, landing in the
    // one act whose entire output is a narrowing.
    //
    // The comment here previously claimed this both kept the incumbent's convention AND that `@me`
    // was "deliberately NOT resolved" — two clauses that cannot both be true, over code that did
    // neither. The concern the second named is real and is now actually satisfied: `@me` resolves
    // to `PRINCIPAL_BIND`, which is `$1`, so it binds at EXECUTION. A UUID literal baked into the
    // plan would have the defect that clause feared; a reference to the principal bind does not.
    let (owner_profile, owner_handle) = match f.owner.as_deref() {
        Some("@me") => (OwnerSlot::Principal, None),
        Some(o) => match uuid::Uuid::parse_str(o) {
            Ok(id) => (OwnerSlot::Bind(QueryBind::Id(id)), None),
            Err(_) => (OwnerSlot::Absent, Some(QueryBind::Text(o.to_string()))),
        },
        None => (OwnerSlot::Absent, None),
    };

    [
        slot(
            (!f.doc_type.is_empty()).then(|| QueryBind::Texts(f.doc_type.clone())),
            "text[]",
        ),
        slot(
            (!f.tags.is_empty()).then(|| QueryBind::Texts(f.tags.clone())),
            "text[]",
        ),
        slot(facets, "jsonb"),
        slot(f.stage.clone().map(QueryBind::Text), "text"),
        slot(f.status.clone().map(QueryBind::Text), "text"),
        match owner_profile {
            // `$1` is already bound as the principal at index 1; referencing it costs no new bind
            // and resolves at execution rather than compile time.
            OwnerSlot::Principal => format!("{PRINCIPAL_BIND}::uuid"),
            OwnerSlot::Bind(b) => slot(Some(b), "uuid"),
            OwnerSlot::Absent => slot(None, "uuid"),
        },
        slot(owner_handle, "text"),
        slot(f.title_contains.clone().map(QueryBind::Text), "text"),
    ]
    .join(", ")
}

/// A property predicate list, bound to one `$n::jsonb` slot — **the one emitter both containers
/// use**, `ResourceFilter::properties` and `EdgeFilter::properties` alike.
///
/// **This is one function because the two slots are one thing.** They carry the same
/// `Vec<PropertyPredicate>`, `contains` means the same thing on both sides (`property_value @> v`,
/// the value whole), and both compile to a `$n::jsonb` the fragment parses the same way. Two
/// emitters agreed only because the second was written by reading the first, which is the agreement
/// that decays — and the cost of the decay is invisible, because both copies keep emitting
/// plausible SQL. A new [`PropertyOp`](temper_core::types::query::filter::PropertyOp) arm now
/// reaches both containers by construction rather than by a second edit.
///
/// **This builds no JSON of its own.** The fragment reads the operator at `q->'op'->>'op'` precisely
/// because that is where `PropertyOp` (internally tagged, in a field called `op`) already puts it.
/// Assembling a flatter object here would be a second spelling of the shape, free to drift with
/// nothing linking them — which is why this is deliberately **not** modelled on the facets slot,
/// which does hand-build its JSON.
///
/// An empty list binds `NULL` rather than `[]`. Both narrow nothing in the fragment, so this is a
/// statement-size choice rather than a semantic one — and it keeps "no predicates supplied"
/// indistinguishable in the SQL from "no filter at all", which is what it is.
///
/// **Call it at the position its bind occupies.** `binds.len() + 1` is positional and the emitted
/// call is positional, so hoisting the call away from where the slot appears would name the wrong
/// `$n`.
fn properties_slot(
    properties: &[temper_core::types::query::filter::PropertyPredicate],
    binds: &mut Vec<QueryBind>,
) -> String {
    if properties.is_empty() {
        return "NULL::jsonb".to_string();
    }
    let idx = binds.len() + 1;
    binds.push(QueryBind::Json(
        serde_json::to_value(properties).unwrap_or(serde_json::Value::Null),
    ));
    format!("${idx}::jsonb")
}

/// `ResourceFilter`'s open-key slot — the absent-container case, then [`properties_slot`].
///
/// The container is optional here and mandatory in [`edge_filter_for`], which is the whole of what
/// separates the two call sites; the emission itself is shared.
fn resource_properties_for(
    inv: &temper_core::types::query::ActInvocation,
    binds: &mut Vec<QueryBind>,
) -> String {
    let Some(f) = &inv.resource_filter else {
        return "NULL::jsonb".to_string();
    };
    properties_slot(&f.properties, binds)
}

/// The composition threaded no QUESTION. Static, and the validator refuses it first — this is the
/// compiler's own last line, kept because `compile` is public and does not require its caller to
/// have run `validate` on the same tick.
///
/// Not to be confused with a failed embedding, which is `RefusalReason::EmbeddingUnavailable` and
/// is the only refusal here that can be the SERVER's fault rather than the caller's.
fn missing_question(inv: &temper_core::types::query::ActInvocation, detail: &str) -> PlanRefusal {
    PlanRefusal {
        stage: Some(inv.name.clone()),
        reason: RefusalReason::MissingIntention,
        detail: detail.to_string(),
    }
}

/// The fragment this builder emits for an act, looked up through the act's DECLARED mechanic.
///
/// Two hops on purpose, and the FIRST hop's name moved on 2026-08-12 without the structure moving.
/// `served_by` names what the deployed `/api/search` door calls — `query_find_exact` since that
/// door gained a resource bound and `readback::search_exact` was repointed off the bound-less
/// incumbent. `CALLABLE_FRAGMENTS` maps that to what `/api/query` emits, which is the UNGATED core
/// (`__temper_ungated_find_exact`), because this compiler establishes the visibility verdict once
/// in the hoisted `__temper_vis` CTE and must not pay a wrapper's second gate per stage.
///
/// So the hop is from a GATED entry point to the ungated body beneath it, and that is what keeps it
/// from collapsing. Two things this comment used to say are recorded as dead rather than quietly
/// dropped: that collapsing the hops "would force the declaration to name the composable twin,
/// which is not the mechanic the deployed door serves" — the twin IS now what the deployed door
/// serves — and that the second hop lands on `query_find_exact`, which the table has never mapped
/// to. Neither was load-bearing; the two-hop structure rests on gated-vs-ungated, which is intact.
fn fragment_for(act: &temper_core::types::query::ActName) -> Option<&'static str> {
    let decl = search_family().into_iter().find(|d| &d.name == act)?;
    let served = decl.served_by?;
    temper_core::types::query::emitted_fragment_for(&served)
}

/// A set combinator over its inputs — **membership only, and the quantity must not enter it.**
///
/// `[fixed — 2026-08-09]` This selected `id, kind, quantity`, and `UNION`/`INTERSECT` compare WHOLE
/// ROWS. A resource found by two different acts carries two different scores — `fts_norm` from the
/// exact arm, `vec_norm` from the wide one — so it was two distinct rows: `intersect` across two
/// acts was ALWAYS empty, and `union` counted the same resource twice. Measured: a two-resource
/// corpus where stage `a` matched one and stage `b` matched both gave `a ∩ b` = 0 rows, reported as
/// `disposition: empty, extent: complete`, when the true answer was one resource.
///
/// It survived because one `Intention` is threaded to every stage, so two `find-exact` stages score
/// identically and a same-act combinator appears to work — and because no test below temper-core
/// has ever constructed a `StageNode::Combine` at all.
///
/// Projecting to `(id, kind)` is also what the stage contract already required: a quantity never
/// crosses a stage boundary, and a set operation is the clearest case of that rule. It stays
/// column-consistent for a combinator over another combinator, because every arm projects
/// explicitly rather than inheriting its input's shape.
fn emit_combine_body(cn: &temper_core::types::query::CombineNode) -> String {
    let op = match cn.op {
        temper_core::types::query::CombineOp::Union => "UNION",
        temper_core::types::query::CombineOp::Intersect => "INTERSECT",
        // Bare `EXCEPT`, never `EXCEPT ALL` — it deduplicates, exactly as its two neighbours do, so
        // all three agree that a stage's output is a set. `validate` pins this arm at exactly two
        // inputs, so the join below emits one `A EXCEPT B` and never a fold.
        temper_core::types::query::CombineOp::Difference => "EXCEPT",
    };
    let arms: Vec<String> = cn
        .inputs
        .iter()
        .map(|s| format!("  SELECT id, kind FROM \"{}\"", s.as_str()))
        .collect();
    arms.join(&format!("\n  {op}\n"))
}

/// The final select: one `hit` arm per RETURNED stage, then one `tally` arm per stage — every
/// stage, returned or not.
///
/// Each returned stage is its own arm, labelled by its stage name; nothing ranks one arm's quantity
/// against another's, and there is no arm two acts' rows share.
///
/// # Why the tallies ride in this statement rather than a second one
///
/// The trace covers every stage, including the intermediates whose rows nobody asked for — that is
/// what lets a reader decide whether stage 2 earned its place. A non-returned stage ships no rows,
/// so a count is the only thing that can distinguish `answered` from `empty` for it. Asking
/// separately would answer from a **different snapshot**, and a trace that disagrees with the rows
/// beside it is worse than no trace: it reads as disclosure and is not.
///
/// A tally carries **how many, never which**. Its id, kind, quantity and `via` columns are NULL by
/// construction, so an intermediate stage's membership stays the pipe's internal currency — and
/// `via` most of all, since it names the very edges a tally is refusing to disclose.
///
/// The two classes share one column list because they are one statement's result set. `row_class`
/// is what the executor switches on; it is a literal in the SQL rather than an inferred property of
/// a NULL id, because "this row has no id" and "this row is a tally" are different claims and
/// deriving one from the other would make an act that legitimately produced a NULL id unreadable.
fn final_select(v: &ValidatedComposition, tallies: &[StageTally]) -> String {
    let mut arms: Vec<String> = v
        .returns()
        .iter()
        .map(|r| {
            let s = r.stage.as_str();
            format!(
                "SELECT 'hit'::text AS row_class, '{s}'::text AS stage, id, kind, quantity, via, \
                 NULL::bigint AS produced, NULL::bigint AS unusable FROM \"{s}\""
            )
        })
        .collect();
    arms.extend(tallies.iter().map(|t| {
        let s = &t.stage;
        let unusable = &t.unusable;
        format!(
            "SELECT 'tally'::text AS row_class, '{s}'::text AS stage, NULL::uuid AS id, \
             NULL::text AS kind, NULL::double precision AS quantity, NULL::jsonb AS via, \
             (SELECT count(*) FROM \"{s}\")::bigint AS produced, {unusable} AS unusable"
        )
    }));
    if arms.is_empty() {
        // Unreachable through `validate`, which refuses an empty `stages` — a claim that was FALSE
        // when this comment was first written and is true now (`RefusalReason::Other("no-stages")`,
        // added 2026-08-09 after review found the gap). Kept because `compile` is public and a
        // zero-arm UNION is not valid SQL.
        return "SELECT NULL::text AS row_class, NULL::text AS stage, NULL::uuid AS id, \
                NULL::text AS kind, NULL::double precision AS quantity, NULL::jsonb AS via, \
                NULL::bigint AS produced, NULL::bigint AS unusable WHERE false"
            .to_string();
    }
    arms.join("\nUNION ALL\n")
}

fn act_name(act: &temper_core::types::query::ActName) -> String {
    serde_json::to_string(act)
        .ok()
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::query::{ActInvocation, ActName, IdSet, StageName};

    fn inv(inputs: Vec<StageInput>) -> ActInvocation {
        ActInvocation {
            name: StageName::parse("s").unwrap(),
            act: ActName::FollowFrom,
            // `follow-from` asks no question of its own — it walks from a set it is handed.
            intention: None,
            inputs,
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        }
    }

    fn upstream(relation: StageRelation) -> StageInput {
        StageInput::Upstream {
            relation,
            stage: StageName::parse("hits").unwrap(),
        }
    }

    /// The narrowing half of [`narrowing_for`]'s answer. The unusable-tally half is asserted at the
    /// emitted-SQL level (`tests/query_plan_compile.rs`), where the expression it produces can be
    /// read in the statement it has to be valid inside.
    fn narrowing(inputs: Vec<StageInput>, binds: &mut Vec<QueryBind>) -> StageNarrowing {
        narrowing_for(&inv(inputs), binds).unwrap().0
    }

    fn caller(relation: StageRelation) -> StageInput {
        StageInput::Caller {
            relation,
            ids: IdSet {
                kind: IdKind::Resource,
                provenance: None,
                ids: vec![Uuid::now_v7()],
            },
        }
    }

    /// **The defect this fix exists for, witnessed at the function that had it.**
    ///
    /// `narrowing_for` read `bounds_mode` NOWHERE: every upstream set went to the narrowing slot
    /// unconditionally. A seed therefore compiled as a bound, and `follow-from` — the only seeding
    /// act — would have returned nothing but what was already in its own seed set: a traversal
    /// that looks like it worked and can never reach a neighbour.
    ///
    /// Tested here rather than through `compile` deliberately, and the reason is a limit worth
    /// stating — one the reachability flip widened rather than closed. It used to be that
    /// `follow-from` emitted `__temper_unbound_act`, which has one slot, so both relations produced
    /// IDENTICAL emitted SQL and an end-to-end assertion would pass against the broken code. Now
    /// `follow-from` is the only act declaring `accepts_seeds` at all and `validate` refuses it, so
    /// **no composition reaching `compile` can carry a seed** and there is no emitted SQL to assert
    /// over in either direction. **This is the only level at which the fix is observable**, and
    /// there is no witness at the SQL level until `search_graph_expand` is reachable and bound to
    /// its `p_seed_ids` parameter — a named remainder, not a covered case.
    #[test]
    fn a_seed_routes_to_the_seed_slot_and_a_bound_to_the_bound_slot() {
        let mut binds = vec![];
        let seeded = narrowing(vec![upstream(StageRelation::Seed)], &mut binds);
        assert!(
            seeded.seed.is_some() && seeded.bound.is_none(),
            "an upstream set declared `seed` must not land in the narrowing slot"
        );
        let bounded = narrowing(vec![upstream(StageRelation::Bound)], &mut binds);
        assert!(bounded.bound.is_some() && bounded.seed.is_none());
    }

    /// **What the widening exists for**: one stage carrying a seed AND a bound, each routed by its
    /// own relation.
    ///
    /// `[added — 2026-08-14]` This was inexpressible until `inputs` became a list — a stage held one
    /// set and one relation, so a bounded walk could name where to start or where to stay, never
    /// both. The two expressions must be DIFFERENT and land in different fields; asserting only
    /// that each is populated would pass against a body that wrote the same set into both.
    #[test]
    fn a_stage_carries_a_seed_and_a_bound_at_once_each_in_its_own_slot() {
        let mut binds = vec![];
        let n = narrowing(
            vec![upstream(StageRelation::Seed), caller(StageRelation::Bound)],
            &mut binds,
        );
        assert_eq!(
            n.seed_expr(),
            r#"ARRAY(SELECT id FROM "hits")"#,
            "the upstream set carried the Seed relation, so it is what the walk grows FROM"
        );
        assert_eq!(
            n.bound_expr(),
            "$1::uuid[]",
            "the caller set carried the Bound relation, so it is what the walk stays WITHIN"
        );
        assert_ne!(
            n.seed_expr(),
            n.bound_expr(),
            "two slots, two sets — writing one set into both would be a walk bounded to its own \
             seeds, which is a different act"
        );
    }

    /// The order of the inputs does not decide the slots — the relation does.
    ///
    /// The list is a wire array and a caller may write it either way round. A body that assigned by
    /// POSITION would pass every test above and fail this one.
    #[test]
    fn the_slot_follows_the_relation_and_never_the_position_in_the_list() {
        let mut binds_a = vec![];
        let forward = narrowing(
            vec![upstream(StageRelation::Seed), caller(StageRelation::Bound)],
            &mut binds_a,
        );
        let mut binds_b = vec![];
        let reversed = narrowing(
            vec![caller(StageRelation::Bound), upstream(StageRelation::Seed)],
            &mut binds_b,
        );
        assert_eq!(forward.seed_expr(), reversed.seed_expr());
        assert_eq!(forward.bound_expr(), reversed.bound_expr());
    }

    /// The relation is read from the edge for a caller-supplied set too.
    ///
    /// The old code's comment justified itself with "an upstream set is always the array slot",
    /// which quietly implied the caller case was different. It is not: the relation and the source
    /// are independent, and reading one from the other is how they got coupled.
    #[test]
    fn the_relation_is_independent_of_where_the_set_came_from() {
        let mut binds = vec![];
        assert!(narrowing(vec![caller(StageRelation::Seed)], &mut binds)
            .seed
            .is_some());
        assert!(narrowing(vec![caller(StageRelation::Bound)], &mut binds)
            .bound
            .is_some());
    }

    /// An act with no input is UNBOUNDED, which is `NULL::uuid[]` and never `'{}'`.
    ///
    /// The fragments read the two differently — NULL is unbounded, empty returns zero rows — and
    /// conflating them turns a stage that found nothing into a global search.
    #[test]
    fn no_input_is_unbounded_and_that_is_not_an_empty_array() {
        let mut binds = vec![];
        let n = narrowing(vec![], &mut binds);
        assert!(n.seed.is_none() && n.bound.is_none() && n.anchor.is_none());
        assert_eq!(n.bound_expr(), "NULL::uuid[]");
        assert_ne!(n.bound_expr(), "'{}'");
        // Both slots, because the widening gave the seed one the same question to answer.
        assert_eq!(n.seed_expr(), "NULL::uuid[]");
        assert_ne!(n.seed_expr(), "'{}'");
    }

    /// A narrowing-only fragment handed a seed errors rather than silently narrowing.
    ///
    /// **Unreachable through `compile` today, and asserted anyway.** `ValidatedComposition` is
    /// parse-don't-validate and the validator refuses a seed against every act declaring
    /// `accepts_seeds: []`, which is all three find acts — so no public call path reaches this arm.
    /// It exists because the two decisions live in different crates and could drift, and because
    /// the failure it guards against is the silent one: narrowing when asked to reach is a
    /// confident wrong answer, while an error is a loud one.
    #[test]
    fn a_narrowing_fragment_handed_a_seed_errors_rather_than_quietly_narrowing() {
        let mut binds = vec![];
        assert!(
            narrowing(vec![upstream(StageRelation::Seed)], &mut binds).has_seed(),
            "`has_seed` is what the find arms read to refuse; it must be true of a seeded stage"
        );
        assert!(
            !narrowing(vec![caller(StageRelation::Bound)], &mut binds).has_seed(),
            "and false of a purely bounded one, or every find stage would refuse"
        );
    }
}
