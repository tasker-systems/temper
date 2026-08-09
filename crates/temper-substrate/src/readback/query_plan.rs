//! Compile a [`ValidatedComposition`] to ONE SQL statement.
//!
//! This is the second runtime-`sqlx` class in this module (the first is the `::vector` bind — see
//! the module note): the SQL SKELETON is assembled at runtime because the DAG shape is not known at
//! compile time, but **every caller value is a positional bind and never interpolated**, and the
//! only identifiers the builder ever emits are stage names, each already proven a safe SQL
//! identifier by [`temper_core::types::query::StageName`]'s parse-only constructor (beat A).
//!
//! The three `find` acts emit real fragments. `follow-from` and `survey` still emit a PLACEHOLDER
//! referencing a function (`__temper_unbound_act`) that does not exist in the schema, so a compiled
//! statement containing one cannot silently return wrong rows if executed — Postgres errors loudly.
//! Their fragments take arguments no slot supplies (`p_depth`/`p_gamma`, `p_lens`), which is what
//! binding them waits on.
//!
//! [`query_exec`](super::query_exec) runs a [`CompiledQuery`] and hands back its two row classes.
//! What does NOT live in either module is the assembly of a `QueryResponse` — deciding a stage's
//! disposition, hydrating the returned arms, building the trace. That needs the composition and the
//! act declarations together, and keeping it out of the substrate is what stops this layer forming
//! an opinion about what a stage MEANT.

use temper_core::types::ids::ProfileId;
use temper_core::types::query::{
    search_family, BoundTerm, IdKind, Intention, PlanRefusal, RefusalReason, StageInput, StageNode,
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
    /// **This carries the per-stage refusals only, and `compile` can still fail whole.** The
    /// tempting sentence here is "static refusals never reach the compiler, they are `validate`'s",
    /// and it is FALSE — which is worth saying, because believing it is how the next refusal gets
    /// added to the wrong side.
    ///
    /// Two refusals still abort the entire composition by returning `Err`:
    ///
    /// * `UnsupportedBoundKind` from a multi-id anchor. `validate` accepts it — an `IdSet` holds N
    ///   ids and the anchor slot holds one, a cardinality gap spec §9 names as open — so this is a
    ///   caller error the compiler is the first to see. **It has the defect this field exists to
    ///   remove**: a healthy `find-exact` stage beside a two-context `find-about-within` loses both.
    ///   Making it per-stage is not the right repair; teaching `validate` to refuse it is, so the
    ///   caller gets it in the 400 with everything else. Named here rather than fixed in passing.
    /// * `UnsupportedSeedKind` from `StageNarrowing::bound_expr` — a compiler/validator
    ///   contradiction rather than anything a caller did, so failing loud is correct.
    ///
    /// What DOES ride here is the runtime refusal: a `find-about-*` stage whose embedding the server
    /// had to compute and could not. Carried rather than returned as an `Err` because a refusal is
    /// per stage — *"Every other stage runs… a composition holding both a `find-exact` and a
    /// `find-about-*` still returns the exact arm."* An `Err` refuses the exact arm too.
    ///
    /// A refused stage's CTE is an EMPTY set, so a stage bounded by it is bounded to nothing —
    /// `ARRAY(SELECT id FROM <it>)` is `'{}'`, which the fragments read as zero rows, and never the
    /// `NULL` they read as unbounded. Collapsing those two turns a failed stage into a global
    /// search.
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
}

/// The placeholder function name every act CTE body targets until Task 10. It intentionally does
/// NOT exist in the schema, so an accidentally-executed skeleton fails loudly rather than returning
/// a silently-empty or silently-wrong result.
const PLACEHOLDER_FN: &str = "__temper_unbound_act";

/// The ungated cores this builder emits for the find acts. Named here as constants so the match in
/// `emit_act_body` cannot drift from what `CALLABLE_FRAGMENTS` maps to.
///
/// These apply NO visibility gate — they are handed the verdict. That is the entire point: the
/// gated twins each compute `resources_visible_to` internally, and the planner does not dedupe those
/// across call sites, so an N-stage composition would pay N recursive team closures. Nothing here
/// may call them without going through [`emit_ungated_core_call`].
const EMIT_FIND_EXACT: &str = "__temper_ungated_find_exact";
const EMIT_FIND_WIDE: &str = "__temper_ungated_find_wide";

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
/// For an ANCHOR it is a **named under-report**. Whether a cogmap or context anchor was readable is
/// decided inside the fragment against `p_anchor_reader`, and finding out here would mean calling
/// `contexts_readable_by` — a second recursive team closure, which is the exact cost the `vis`
/// hoist exists to avoid. An unreadable anchor therefore comes back as an `empty` stage rather than
/// as one unusable id. That is the disposition the contract prescribes for it anyway (an id YOU
/// supplied that you cannot see is `empty`, never `withheld`, or the trace becomes a single-probe
/// existence oracle), so the loss is one counter, not the disclosure.
const NO_UNUSABLE: &str = "0::bigint";

/// The principal, always `$1` — `compile` pushes it first, before any per-stage bind. The cores read
/// it ONLY for cogmap-anchor readability, which is one boolean per call and a property of no row, so
/// it cannot ride in `VISIBLE_IDS`. It is not a visibility gate.
const PRINCIPAL_BIND: &str = "$1";

/// The ANN candidate width handed to the wide core. Carried over from `/api/search`'s own draw and
/// matched by that function's `hnsw.ef_search` pin (200 >= 100) — a k above the pin would make
/// `LIMIT p_k` unreachable and truncate the draw silently.
const ANN_DRAW_K: i32 = 100;

/// Compile a validated composition into one statement. `principal` is bound as `$1` and drives the
/// single visibility relation every stage joins.
///
/// `embedding` is the query vector. It is a parameter rather than a field of
/// `Composition.intention` because `Intention` is a WIRE type carrying `query: String` and
/// `embedded: bool` — the *fact* that an embedding was computed, never the vector. Putting a
/// 768-float array in the envelope would be a contract change nobody asked for.
///
/// **`None` means the vector could not be obtained, not that the caller declined to send one.**
/// `[amended — 2026-08-08, Pete]` Embedding on the caller's behalf is this surface's job: the CLI
/// links temper-ingest and computes vectors client-side, while the ruby gem, the TypeScript
/// package and MCP structurally cannot, so a caller-must-embed rule would deny `find-about-*` to
/// every non-CLI client. The executor calls `substrate_read::embed_query_if_missing` before it
/// reaches here, exactly as `/api/search` does.
///
/// So a `None` at this point has already survived that attempt, and a `find-about-*` stage
/// refuses with `EmbeddingUnavailable` — the contract's ONE runtime refusal. Still a refusal
/// rather than a silent NULL bind: the stage holds a well-formed question it cannot answer, and
/// searching on nothing returns a list that reads like an answer. That is why this returns a
/// `Result`: a refusal here is a disposition, not a panic.
pub fn compile(
    v: &ValidatedComposition,
    principal: ProfileId,
    embedding: Option<&[f32]>,
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

    let intention = v.composition().intention.as_ref();
    let mut tallies: Vec<StageTally> = Vec::new();
    let mut refusals: Vec<PlanRefusal> = Vec::new();
    for node in v.ordered() {
        let (name, body, unusable) = match node {
            StageNode::Act(inv) => {
                let emitted = emit_act_body(inv, intention, embedding, &mut binds, &mut refusals)?;
                (inv.name.as_str(), emitted.body, emitted.unusable)
            }
            // A combinator's inputs are upstream stages, so nothing it was handed can be unusable.
            StageNode::Combine(cn) => (
                cn.name.as_str(),
                emit_combine_body(cn),
                NO_UNUSABLE.to_string(),
            ),
        };
        ctes.push(format!("{name} AS (\n{body}\n)"));
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
         SELECT NULL::uuid AS id, NULL::text AS kind, NULL::double precision AS quantity \
         WHERE false"
    )
}

/// A placeholder act body in the `(id, kind, quantity)` stage-contract shape. IDs only cross a stage
/// boundary — a downstream stage references its upstream as `SELECT id FROM <stage>`, never a
/// quantity, which is what keeps `no-cross-act-ranking` structural (spec §4).
fn emit_act_body(
    inv: &temper_core::types::query::ActInvocation,
    intention: Option<&Intention>,
    embedding: Option<&[f32]>,
    binds: &mut Vec<QueryBind>,
    refusals: &mut Vec<PlanRefusal>,
) -> Result<EmittedAct, PlanRefusal> {
    let act = act_name(&inv.act);

    let (narrowing, unusable) = narrowing_for(inv, binds)?;
    let emitted = |body: String| EmittedAct {
        body,
        unusable: unusable.clone(),
    };
    let (anchor_table, anchor_id) = narrowing.anchor();
    let (anchor_table, anchor_id) = (anchor_table.to_string(), anchor_id.to_string());
    let (limit, offset) = paging_for(inv, binds);

    // The find acts narrow and never seed, so each takes the bound expression. A seed reaching one
    // is a validator/compiler disagreement rather than a caller error — `bound_expr` says so and
    // errors instead of quietly narrowing.
    let bound_for_find = |inv: &temper_core::types::query::ActInvocation| {
        narrowing
            .bound_expr()
            .map(str::to_string)
            .map_err(|detail| PlanRefusal {
                stage: Some(inv.name.clone()),
                reason: RefusalReason::UnsupportedSeedKind,
                detail: detail.to_string(),
            })
    };

    match fragment_for(&inv.act) {
        Some(EMIT_FIND_EXACT) => {
            let bound = bound_for_find(inv)?;
            let q = intention.map(|i| i.query.as_str()).ok_or_else(|| {
                missing_question(
                    inv,
                    "find-exact needs the intention's query text — it becomes `p_query`, and there \
                     is nowhere else to source it. The composition threaded no intention",
                )
            })?;
            let qi = binds.len() + 1;
            binds.push(QueryBind::Text(q.to_string()));
            let call = emit_ungated_core_call(&CoreCall {
                core: EMIT_FIND_EXACT,
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
                 fts_norm::double precision AS quantity\n    \
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
            let call = emit_ungated_core_call(&CoreCall {
                core: EMIT_FIND_WIDE,
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
                 vec_norm::double precision AS quantity\n    \
                 FROM {call}"
            )))
        }
        // `follow-from` and `survey` reach here: their mechanics are declared reachable, but their
        // fragments take arguments no slot supplies (`p_depth`/`p_gamma`, `p_lens`), so this
        // builder still emits the deliberately-absent placeholder rather than guessing a value.
        _ => Ok(emitted(format!(
            "  -- act: {act} (placeholder body; this builder emits no fragment for it yet)\n  \
             SELECT id, kind, quantity FROM {PLACEHOLDER_FN}({})",
            narrowing.any_set_expr(),
        ))),
    }
}

/// Everything an ungated-core call needs that is NOT an authorization input.
///
/// Note what is absent: there is no field for the visible-id set and none for the anchor reader.
/// That absence is the design — see [`emit_ungated_core_call`].
struct CoreCall<'a> {
    core: &'a str,
    /// The arm-specific arguments between the visible set and the narrowing slots: the bound query
    /// text for the exact arm, the embedding and draw width for the wide one.
    intent_args: String,
    bound: &'a str,
    anchor_table: &'a str,
    anchor_id: &'a str,
    limit: &'a str,
    offset: &'a str,
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
/// `NULL` in the `p_doc_type` slot: the typed doc-type filter is not yet routed from
/// `ActInvocation.resource_filter` into the fragment. Passing NULL is "no doc-type narrowing", which
/// is what a plan that declares none means.
fn emit_ungated_core_call(c: &CoreCall) -> String {
    format!(
        "{}({VISIBLE_IDS}, {}, {}, {}, {}, {PRINCIPAL_BIND}, NULL, {}, {})",
        c.core, c.intent_args, c.bound, c.anchor_table, c.anchor_id, c.limit, c.offset
    )
}

/// What one stage does with the set it was handed, and therefore which slot the set belongs in.
///
/// An enum rather than a struct of optional strings, because "never both" was previously prose
/// over three sibling fields — and prose over sibling fields is what let `bounds_mode` be ignored
/// in the first place. An act is handed ONE `IdSet`; the relation says what it is FOR and the
/// kind says which slot serves it.
enum StageNarrowing {
    /// No input. `NULL::uuid[]`, never `'{}'` — the fragments read the two differently and
    /// conflating them is exactly the substitution delta 3 forbids.
    Unbounded,
    /// Narrow to within this set: the `p_bound_ids uuid[]` slot.
    Bound(String),
    /// Grow from this set: the `p_seed_ids uuid[]` slot.
    ///
    /// **A different slot on a different fragment, which is the whole point of the fix.** Routing
    /// a seed into `p_bound_ids` compiles a traversal that can only return what was already in its
    /// own seed set — a stage that looks like it worked and can never produce a neighbour.
    Seed(String),
    /// A `(table, id)` anchor pair — how the fragments take a cogmap or context scope. Holds
    /// exactly one id.
    Anchor { table: String, id: String },
}

impl StageNarrowing {
    /// The `p_bound_ids` expression for a fragment that only narrows.
    ///
    /// A [`Self::Seed`] reaching here is a compiler-level contradiction, not a caller error: the
    /// validator refuses a seed against an act declaring `accepts_seeds: []`, and the three find
    /// acts all declare exactly that. It returns an error rather than silently narrowing, because
    /// silently narrowing is the defect this enum was introduced to remove — if the two ever
    /// disagree, the loud answer is the safe one.
    fn bound_expr(&self) -> Result<&str, &'static str> {
        match self {
            StageNarrowing::Bound(b) => Ok(b),
            StageNarrowing::Unbounded | StageNarrowing::Anchor { .. } => Ok("NULL::uuid[]"),
            StageNarrowing::Seed(_) => Err(
                "this act narrows within a set and cannot grow from one; the validator should \
                 have refused this stage as `unsupported_seed_kind` before compilation",
            ),
        }
    }

    fn anchor(&self) -> (&str, &str) {
        match self {
            StageNarrowing::Anchor { table, id } => (table, id),
            _ => ("NULL", "NULL"),
        }
    }

    /// The set expression a placeholder body echoes, whichever slot it came from.
    fn any_set_expr(&self) -> &str {
        match self {
            StageNarrowing::Bound(s) | StageNarrowing::Seed(s) => s,
            StageNarrowing::Anchor { id, .. } => id,
            StageNarrowing::Unbounded => "NULL::uuid[]",
        }
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
/// worked and can never reach a neighbour. It was latent only because `follow-from` still emits
/// the placeholder, and it would have fired the moment that fragment was bound. The relation is
/// now a field of [`StageInput`] rather than an `Option` beside it, so there is a value to read
/// instead of one to invent.
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
    let Some(input) = &inv.input else {
        return Ok((StageNarrowing::Unbounded, NO_UNUSABLE.to_string()));
    };
    // Read once, up front. The relation is a property of the edge and is the same whether the set
    // came from the caller or from an upstream stage — which is why it is not re-derived in each
    // arm below.
    let seeding = input.relation() == StageRelation::Seed;
    let by_relation = |expr: String| {
        if seeding {
            StageNarrowing::Seed(expr)
        } else {
            StageNarrowing::Bound(expr)
        }
    };

    match input {
        // Ids only — no quantity from the upstream stage is ever in scope here, which is what keeps
        // `no-cross-act-ranking` structural rather than policed. An upstream set is always resource
        // ids (that is what these acts produce), so it is always an array slot; WHICH array slot is
        // the relation's business.
        StageInput::Upstream { stage, .. } => Ok((
            by_relation(format!("ARRAY(SELECT id FROM {})", stage.as_str())),
            NO_UNUSABLE.to_string(),
        )),
        StageInput::Caller { ids, .. } => match ids.kind {
            IdKind::Resource => {
                let idx = binds.len() + 1;
                binds.push(QueryBind::Uuids(ids.ids.clone()));
                Ok((by_relation(format!("${idx}::uuid[]")), unusable_tally(idx)))
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
                let [id] = ids.ids.as_slice() else {
                    return Err(PlanRefusal {
                        stage: Some(inv.name.clone()),
                        reason: RefusalReason::UnsupportedBoundKind,
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
                Ok((
                    StageNarrowing::Anchor {
                        table: format!("'{table}'::varchar"),
                        id: format!("(${ai}::uuid[])[1]"),
                    },
                    NO_UNUSABLE.to_string(),
                ))
            }
            // A region set reaching a find act is already refused by the validator against
            // `accepts_bounds`; unbounded here rather than a second, divergent opinion about it.
            _ => Ok((StageNarrowing::Unbounded, NO_UNUSABLE.to_string())),
        },
    }
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
    let mut bind_term = |t: BoundTerm, absent: &str| match inv.terms.get(&t) {
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
/// Two hops on purpose. `served_by` keeps naming what `/api/search` calls (`search_exact`), because
/// that is what the declaration describes; `CALLABLE_FRAGMENTS` maps that to what `/api/query`
/// emits (`query_find_exact`). Collapsing them would force the declaration to name the composable
/// twin, which is not the mechanic the deployed door serves.
fn fragment_for(act: &temper_core::types::query::ActName) -> Option<&'static str> {
    let decl = search_family().into_iter().find(|d| &d.name == act)?;
    let served = decl.served_by?;
    temper_core::types::query::emitted_fragment_for(&served)
}

/// A set combinator over its inputs, ids only.
fn emit_combine_body(cn: &temper_core::types::query::CombineNode) -> String {
    let op = match cn.op {
        temper_core::types::query::CombineOp::Union => "UNION",
        temper_core::types::query::CombineOp::Intersect => "INTERSECT",
    };
    let arms: Vec<String> = cn
        .inputs
        .iter()
        .map(|s| format!("  SELECT id, kind, quantity FROM {}", s.as_str()))
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
/// A tally carries **how many, never which**. Its id, kind and quantity columns are NULL by
/// construction, so an intermediate stage's membership stays the pipe's internal currency.
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
                "SELECT 'hit'::text AS row_class, '{s}'::text AS stage, id, kind, quantity, \
                 NULL::bigint AS produced, NULL::bigint AS unusable FROM {s}"
            )
        })
        .collect();
    arms.extend(tallies.iter().map(|t| {
        let s = &t.stage;
        let unusable = &t.unusable;
        format!(
            "SELECT 'tally'::text AS row_class, '{s}'::text AS stage, NULL::uuid AS id, \
             NULL::text AS kind, NULL::double precision AS quantity, \
             (SELECT count(*) FROM {s})::bigint AS produced, {unusable} AS unusable"
        )
    }));
    if arms.is_empty() {
        // Unreachable through `validate`, which refuses an empty `stages`. Kept because `compile`
        // is public and a zero-arm UNION is not valid SQL.
        return "SELECT NULL::text AS row_class, NULL::text AS stage, NULL::uuid AS id, \
                NULL::text AS kind, NULL::double precision AS quantity, NULL::bigint AS produced, \
                NULL::bigint AS unusable WHERE false"
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

    fn inv(input: Option<StageInput>) -> ActInvocation {
        ActInvocation {
            name: StageName::parse("s").unwrap(),
            act: ActName::FollowFrom,
            input,
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        }
    }

    fn upstream(relation: StageRelation) -> Option<StageInput> {
        Some(StageInput::Upstream {
            relation,
            stage: StageName::parse("hits").unwrap(),
        })
    }

    /// The narrowing half of [`narrowing_for`]'s answer. The unusable-tally half is asserted at the
    /// emitted-SQL level (`tests/query_plan_compile.rs`), where the expression it produces can be
    /// read in the statement it has to be valid inside.
    fn narrowing(input: Option<StageInput>, binds: &mut Vec<QueryBind>) -> StageNarrowing {
        narrowing_for(&inv(input), binds).unwrap().0
    }

    fn caller(relation: StageRelation) -> Option<StageInput> {
        Some(StageInput::Caller {
            relation,
            ids: IdSet {
                kind: IdKind::Resource,
                provenance: None,
                ids: vec![Uuid::now_v7()],
            },
        })
    }

    /// **The defect this fix exists for, witnessed at the function that had it.**
    ///
    /// `narrowing_for` read `bounds_mode` NOWHERE: every upstream set went to the narrowing slot
    /// unconditionally. A seed therefore compiled as a bound, and `follow-from` — the only seeding
    /// act — would have returned nothing but what was already in its own seed set: a traversal
    /// that looks like it worked and can never reach a neighbour.
    ///
    /// Tested here rather than through `compile` deliberately, and the reason is a limit worth
    /// stating: `follow-from` still emits `__temper_unbound_act`, which has one slot, so the two
    /// relations produce IDENTICAL emitted SQL today. An end-to-end assertion would pass against
    /// the broken code. **This is the only level at which the fix is currently observable**, and
    /// there is no witness at the SQL level until `search_graph_expand` is bound to its
    /// `p_seed_ids` parameter — a named remainder, not a covered case.
    #[test]
    fn a_seed_routes_to_the_seed_slot_and_a_bound_to_the_bound_slot() {
        let mut binds = vec![];
        assert!(
            matches!(
                narrowing(upstream(StageRelation::Seed), &mut binds),
                StageNarrowing::Seed(_)
            ),
            "an upstream set declared `seed` must not land in the narrowing slot"
        );
        assert!(matches!(
            narrowing(upstream(StageRelation::Bound), &mut binds),
            StageNarrowing::Bound(_)
        ));
    }

    /// The relation is read from the edge for a caller-supplied set too.
    ///
    /// The old code's comment justified itself with "an upstream set is always the array slot",
    /// which quietly implied the caller case was different. It is not: the relation and the source
    /// are independent, and reading one from the other is how they got coupled.
    #[test]
    fn the_relation_is_independent_of_where_the_set_came_from() {
        let mut binds = vec![];
        assert!(matches!(
            narrowing(caller(StageRelation::Seed), &mut binds),
            StageNarrowing::Seed(_)
        ));
        assert!(matches!(
            narrowing(caller(StageRelation::Bound), &mut binds),
            StageNarrowing::Bound(_)
        ));
    }

    /// An act with no input is UNBOUNDED, which is `NULL::uuid[]` and never `'{}'`.
    ///
    /// The fragments read the two differently — NULL is unbounded, empty returns zero rows — and
    /// conflating them turns a stage that found nothing into a global search.
    #[test]
    fn no_input_is_unbounded_and_that_is_not_an_empty_array() {
        let mut binds = vec![];
        let n = narrowing(None, &mut binds);
        assert!(matches!(n, StageNarrowing::Unbounded));
        assert_eq!(n.bound_expr().unwrap(), "NULL::uuid[]");
        assert_ne!(n.bound_expr().unwrap(), "'{}'");
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
        assert!(
            StageNarrowing::Seed("ARRAY(SELECT id FROM hits)".to_string())
                .bound_expr()
                .is_err()
        );
        assert!(StageNarrowing::Bound("$2::uuid[]".to_string())
            .bound_expr()
            .is_ok());
    }
}
