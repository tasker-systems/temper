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
//! There is deliberately no executor here yet; nothing runs a [`CompiledQuery`].

use temper_core::types::ids::ProfileId;
use temper_core::types::query::{
    search_family, BoundTerm, IdKind, Intention, PlanRefusal, RefusalReason, StageInput, StageNode,
    ValidatedComposition,
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
/// `embedding` is the **caller-computed** query vector. It is a parameter rather than a field of
/// `Composition.intention` because `Intention` is a WIRE type carrying `query: String` and
/// `embedded: bool` — the *fact* that an embedding was computed, never the vector. Putting a
/// 768-float array in the envelope would be a contract change nobody asked for.
///
/// `None` is not a silent NULL bind. A `find-about-*` stage compiled without an embedding
/// **refuses**, because "I chose not to embed" and "I cannot embed" must stay two states rather
/// than collapsing into one ambiguous one — the same rule delta 3 states for an empty upstream.
/// That is why this returns a `Result`: a refusal here is a disposition, not a panic.
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
    for node in v.ordered() {
        let (name, body) = match node {
            StageNode::Act(inv) => (
                inv.name.as_str(),
                emit_act_body(inv, intention, embedding, &mut binds)?,
            ),
            StageNode::Combine(cn) => (cn.name.as_str(), emit_combine_body(cn)),
        };
        ctes.push(format!("{name} AS (\n{body}\n)"));
        cte_names.push((name.to_string(), name.to_string()));
    }

    let sql = format!("WITH {}\n{}", ctes.join(",\n"), final_select(v));
    Ok(CompiledQuery {
        sql,
        binds,
        cte_names,
    })
}

/// A placeholder act body in the `(id, kind, quantity)` stage-contract shape. IDs only cross a stage
/// boundary — a downstream stage references its upstream as `SELECT id FROM <stage>`, never a
/// quantity, which is what keeps `no-cross-act-ranking` structural (spec §4).
fn emit_act_body(
    inv: &temper_core::types::query::ActInvocation,
    intention: Option<&Intention>,
    embedding: Option<&[f32]>,
    binds: &mut Vec<QueryBind>,
) -> Result<String, PlanRefusal> {
    let act = act_name(&inv.act);

    let Narrowing {
        bound,
        anchor_table,
        anchor_id,
    } = narrowing_for(inv, binds)?;
    let (limit, offset) = paging_for(inv, binds);

    match fragment_for(&inv.act) {
        Some(EMIT_FIND_EXACT) => {
            let q = intention.map(|i| i.query.as_str()).ok_or_else(|| {
                refusal(
                    inv,
                    "find-exact needs the intention's query text; the composition threaded none",
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
            Ok(format!(
                "  -- act: {act} -> {EMIT_FIND_EXACT}\n  \
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 fts_norm::double precision AS quantity\n    \
                 FROM {call}"
            ))
        }
        Some(EMIT_FIND_WIDE) => {
            // The refusal delta 2 requires: distinct from failure and from honest-empty, and never
            // improvised into a NULL bind that would silently search on nothing.
            let emb = embedding.ok_or_else(|| {
                refusal(
                    inv,
                    "a find-about-* stage needs a query embedding; the caller supplied none, and \
                     the server does not embed on the caller's behalf — 'I chose not to embed' and \
                     'I cannot embed' are different states",
                )
            })?;
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
            Ok(format!(
                "  -- act: {act} -> {EMIT_FIND_WIDE}\n  \
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 vec_norm::double precision AS quantity\n    \
                 FROM {call}"
            ))
        }
        // `follow-from` and `survey` reach here: their mechanics are declared reachable, but their
        // fragments take arguments no slot supplies (`p_depth`/`p_gamma`, `p_lens`), so this
        // builder still emits the deliberately-absent placeholder rather than guessing a value.
        _ => Ok(format!(
            "  -- act: {act} (placeholder body; this builder emits no fragment for it yet)\n  \
             SELECT id, kind, quantity FROM {PLACEHOLDER_FN}({bound})",
        )),
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

/// How one stage narrows: an id-array bound, an anchor pair, or neither. Never both — an act is
/// handed ONE `IdSet`, and its `kind` decides which slot that set belongs in.
struct Narrowing {
    bound: String,
    anchor_table: String,
    anchor_id: String,
}

/// Route the stage's input to the slot its KIND belongs in.
///
/// `IdSet.kind` was previously ignored and every set went to `p_bound_ids uuid[]`. That is a
/// **confident empty**: `find-exact` and `find-about-within` both declare
/// `accepts_bounds: [Resource, Context, Cogmap]`, so a caller handing a Cogmap set would validate
/// cleanly and then have cogmap uuids compared against `r.id`, returning zero rows with a 200 and
/// no disclosure that the narrowing was nonsense. The declaration says those kinds are accepted
/// "through the anchor pair" — so the compiler has to actually emit the anchor pair.
///
/// A `None` input is UNBOUNDED, which is `NULL::uuid[]` and never `'{}'` — the twins read the two
/// differently and conflating them is exactly the substitution delta 3 forbids.
fn narrowing_for(
    inv: &temper_core::types::query::ActInvocation,
    binds: &mut Vec<QueryBind>,
) -> Result<Narrowing, PlanRefusal> {
    let unbounded = Narrowing {
        bound: "NULL::uuid[]".to_string(),
        anchor_table: "NULL".to_string(),
        anchor_id: "NULL".to_string(),
    };
    match &inv.input {
        None => Ok(unbounded),
        // Ids only — no quantity from the upstream stage is ever in scope here, which is what keeps
        // `no-cross-act-ranking` structural rather than policed. An upstream set is always resource
        // ids (that is what these acts produce), so it is always the array slot.
        Some(StageInput::Upstream { stage }) => Ok(Narrowing {
            bound: format!("ARRAY(SELECT id FROM {})", stage.as_str()),
            ..unbounded
        }),
        Some(StageInput::Caller { ids }) => match ids.kind {
            IdKind::Resource => {
                let idx = binds.len() + 1;
                binds.push(QueryBind::Uuids(ids.ids.clone()));
                Ok(Narrowing {
                    bound: format!("${idx}::uuid[]"),
                    ..unbounded
                })
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
                Ok(Narrowing {
                    bound: "NULL::uuid[]".to_string(),
                    anchor_table: format!("'{table}'::varchar"),
                    anchor_id: format!("(${ai}::uuid[])[1]"),
                })
            }
            // A region set reaching a find act is already refused by the validator against
            // `accepts_bounds`; unbounded here rather than a second, divergent opinion about it.
            _ => Ok(unbounded),
        },
    }
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

fn refusal(inv: &temper_core::types::query::ActInvocation, detail: &str) -> PlanRefusal {
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

/// The final per-arm select over the declared returned stages. Each returned stage is its own arm,
/// labelled by its stage name; nothing ranks one arm's quantity against another's.
fn final_select(v: &ValidatedComposition) -> String {
    let returns = v.returns();
    if returns.is_empty() {
        return "SELECT NULL::uuid AS id, NULL::text AS kind, NULL::double precision AS quantity, \
                NULL::text AS stage WHERE false"
            .to_string();
    }
    let arms: Vec<String> = returns
        .iter()
        .map(|r| {
            let s = r.stage.as_str();
            format!("SELECT id, kind, quantity, '{s}'::text AS stage FROM {s}")
        })
        .collect();
    arms.join("\nUNION ALL\n")
}

fn act_name(act: &temper_core::types::query::ActName) -> String {
    serde_json::to_string(act)
        .ok()
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
