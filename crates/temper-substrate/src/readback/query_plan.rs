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
use temper_core::types::query::{StageInput, StageNode, ValidatedComposition};
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

/// Compile a validated composition into one statement. `principal` is bound as `$1` and drives the
/// single visibility relation every stage joins.
pub fn compile(v: &ValidatedComposition, principal: ProfileId) -> CompiledQuery {
    let mut binds: Vec<QueryBind> = vec![QueryBind::Profile(principal)];
    let mut cte_names: Vec<(String, String)> = Vec::new();
    let mut ctes: Vec<String> = Vec::new();

    // The visibility relation, materialized ONCE — decision 019fcd13: one query time, one
    // visibility computation, no per-stage recomputation. `MATERIALIZED` is an optimization fence,
    // not merely "compute once" — see the task notes on the hoist strategy.
    ctes.push("vis AS MATERIALIZED (\n  SELECT id FROM resources_visible_to($1)\n)".to_string());

    for node in v.ordered() {
        let (name, body) = match node {
            StageNode::Act(inv) => (inv.name.as_str(), emit_act_body(inv, &mut binds)),
            StageNode::Combine(cn) => (cn.name.as_str(), emit_combine_body(cn)),
        };
        ctes.push(format!("{name} AS (\n{body}\n)"));
        cte_names.push((name.to_string(), name.to_string()));
    }

    let sql = format!("WITH {}\n{}", ctes.join(",\n"), final_select(v));
    CompiledQuery {
        sql,
        binds,
        cte_names,
    }
}

/// A placeholder act body in the `(id, kind, quantity)` stage-contract shape. IDs only cross a stage
/// boundary — a downstream stage references its upstream as `SELECT id FROM <stage>`, never a
/// quantity, which is what keeps `no-cross-act-ranking` structural (spec §4).
fn emit_act_body(
    inv: &temper_core::types::query::ActInvocation,
    binds: &mut Vec<QueryBind>,
) -> String {
    let act = act_name(&inv.act);
    let arg = match &inv.input {
        Some(StageInput::Caller { ids }) => {
            let idx = binds.len() + 1;
            binds.push(QueryBind::Uuids(ids.ids.clone()));
            format!("${idx}")
        }
        // Ids only — no quantity from the upstream stage is ever in scope here.
        Some(StageInput::Upstream { stage }) => {
            format!("ARRAY(SELECT id FROM {})", stage.as_str())
        }
        None => String::new(),
    };
    format!(
        "  -- act: {act} (placeholder body; Task 10 binds the deployed fragment)\n  \
         SELECT id, kind, quantity FROM {PLACEHOLDER_FN}({arg})",
    )
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
