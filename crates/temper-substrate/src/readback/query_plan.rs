//! Compile a [`ValidatedComposition`] to ONE SQL statement.
//!
//! This is the second runtime-`sqlx` class in this module (the first is the `::vector` bind — see
//! the module note): the SQL SKELETON is assembled at runtime because the DAG shape is not known at
//! compile time, but **every caller value is a positional bind and never interpolated**, and the
//! only identifiers the builder ever emits are stage names, each already proven a safe SQL
//! identifier by [`temper_core::types::query::StageName`]'s parse-only constructor (beat A).
//!
//! **Beat C, Task 9 — skeleton only.** The per-act CTE bodies are PLACEHOLDERS that reference a
//! function (`__temper_unbound_act`) which does not exist in the schema, so a compiled statement
//! from this task cannot silently return wrong rows if executed — Postgres errors loudly. Task 10
//! replaces the placeholders with real calls to `search_graph_expand` / `wayfind_region_scores`.
//! There is deliberately no executor here yet; nothing runs a [`CompiledQuery`].

use temper_core::types::ids::ProfileId;
use temper_core::types::query::{
    search_family, Intention, PlanRefusal, RefusalReason, StageInput, StageNode,
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

/// The composable twins this builder emits for the find acts. Named here as constants so the match
/// in `emit_act_body` cannot drift from what `CALLABLE_FRAGMENTS` maps to.
const EMIT_FIND_EXACT: &str = "query_find_exact";
const EMIT_FIND_WIDE: &str = "query_find_wide";

/// The ANN candidate width handed to `query_find_wide`. Carried over from `/api/search`'s own draw
/// and matched by that function's `hnsw.ef_search` pin (200 >= 100) — a k above the pin would make
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

    // The visibility relation, materialized ONCE — decision 019fcd13: one query time, one
    // visibility computation, no per-stage recomputation. `MATERIALIZED` is an optimization fence,
    // not merely "compute once" — see the task notes on the hoist strategy.
    ctes.push("vis AS MATERIALIZED (\n  SELECT id FROM resources_visible_to($1)\n)".to_string());

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

    // The bound set, in the composition's one currency: ids. A `None` input is UNBOUNDED, which is
    // `NULL::uuid[]` and never `'{}'` — the twins read the two differently and conflating them is
    // exactly the substitution delta 3 forbids.
    let bound = match &inv.input {
        Some(StageInput::Caller { ids }) => {
            let idx = binds.len() + 1;
            binds.push(QueryBind::Uuids(ids.ids.clone()));
            format!("${idx}::uuid[]")
        }
        // Ids only — no quantity from the upstream stage is ever in scope here, which is what keeps
        // `no-cross-act-ranking` structural rather than policed.
        Some(StageInput::Upstream { stage }) => {
            format!("ARRAY(SELECT id FROM {})", stage.as_str())
        }
        None => "NULL::uuid[]".to_string(),
    };

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
            Ok(format!(
                "  -- act: {act} -> {EMIT_FIND_EXACT}\n  \
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 fts_norm::double precision AS quantity\n    \
                 FROM {EMIT_FIND_EXACT}($1, ${qi}, {bound}, NULL, NULL, NULL, NULL, 0)"
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
            Ok(format!(
                "  -- act: {act} -> {EMIT_FIND_WIDE}\n  \
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 vec_norm::double precision AS quantity\n    \
                 FROM {EMIT_FIND_WIDE}($1, ${ei}::vector, ${ki}::int, {bound}, \
                 NULL, NULL, NULL, NULL, 0)"
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
