//! `/api/query`'s read path: embed, compile, run, hydrate, assemble.
//!
//! The layering this module completes. `temper-core` decides what a plan may be (`validate`);
//! `temper-substrate` turns a validated plan into one statement and runs it (`query_plan`,
//! `query_exec`) **without forming an opinion about what a stage meant**; and this module is where
//! the opinion belongs, because it is the only layer holding the composition and the act
//! declarations at the same time.
//!
//! # What is derived here rather than asked of the database
//!
//! Most of a `StageResult` is not a row. `act`, `orders_by`, `relation`, `input_source` and
//! `terms_applied` are properties of the PLAN and its declarations; only the rows, the counts and
//! the refusals come back from Postgres. Two of the disclosure numbers are derived rather than
//! measured, and each is exact rather than approximate:
//!
//! * `input_ids` for an upstream-fed stage is the upstream stage's own produced tally. Asking again
//!   would be asking the same statement the same question twice.
//! * `input_contributed` for a **bound** is the produced count, because a bound narrows *within* the
//!   set it was handed — output ⊆ input by construction, so *"how many of your ids are in the
//!   output"* is just *"how many came out"*.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::backend::substrate_read::{embed_query_text, QueryEmbed};
use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::query::{
    applied_terms, declaration, ActName, Disclosure, Extent, NarrowedBy, PlanRefusal,
    QueryResponse, ResourceHit, ReturnSpec, Scoring, StageDisposition, StageInput, StageNode,
    StageOutput, StageRelation, StageResult, StageTrace, ValidatedComposition,
};
use temper_core::types::query::{BoundTerm, CompositionTrace, InputSource};
use temper_core::types::resource_view::{ResourceSection, ResourceView};
use temper_substrate::readback;
use temper_substrate::readback::query_exec::{execute, QueryRows};
use temper_substrate::readback::query_plan::compile;

/// Run a validated composition and answer in the shape `POST /api/query` publishes.
///
/// `caller_embedding` is a vector the caller precomputed — the CLI can, because it links
/// temper-ingest. **Its absence is not a refusal.** The ruby gem, the TypeScript package and MCP
/// carry no embedding ability at all, so the server embeds on their behalf, exactly as
/// `/api/search` does; only a FAILED attempt refuses, and it refuses the stages that needed a
/// vector rather than the composition.
pub async fn run_composition(
    pool: &PgPool,
    principal: ProfileId,
    v: &ValidatedComposition,
    caller_embedding: Option<Vec<f32>>,
) -> ApiResult<QueryResponse> {
    let embedding = resolve_embedding(v, caller_embedding).await;
    // A refusal from here is a compiler/validator contradiction, never a caller error — `validate`
    // has already refused everything a caller can get wrong. Rendering it as a 500 rather than a
    // 400 says so: telling a caller to fix a plan that is correct would send them in circles.
    let compiled = compile(v, principal, embedding.as_deref()).map_err(|r: PlanRefusal| {
        ApiError::Internal(format!(
            "compiler refused a validated plan ({:?}): {}",
            r.reason, r.detail
        ))
    })?;
    let rows = execute(pool, &compiled).await.map_err(opaque)?;
    let hydrated = hydrate(pool, principal, v, &rows).await?;
    Ok(assemble(v, &rows, &hydrated))
}

/// Log the real failure; hand the caller nothing but the fact of it.
///
/// `[fixed — 2026-08-09]` These three sites formatted the underlying error into
/// `ApiError::Internal`, whose message goes STRAIGHT INTO THE RESPONSE BODY — so a composition
/// naming `follow-from` answered the caller with
/// `function __temper_unbound_act(uuid[]) does not exist`, and a mis-sized caller embedding would
/// answer with pgvector's dimension complaint. This crate's own `From<sqlx::Error> for ApiError`
/// redacts to "An internal error occurred" and logs the detail; these bypassed that convention.
/// Found in review. Not an RBAC hole — but internal function names and argument types are exactly
/// what this codebase refuses to echo everywhere else.
fn opaque(e: anyhow::Error) -> ApiError {
    tracing::error!(error = %e, "query read failed");
    ApiError::Internal("An internal error occurred".to_string())
}

/// The vector, or the honest absence of one.
///
/// Embedding is attempted only when some stage would USE it. Embedding for a composition of
/// `find-exact` stages would pay ONNX inference to produce a value nothing binds — and worse, a
/// failure would then refuse nothing while having cost the budget.
async fn resolve_embedding(
    v: &ValidatedComposition,
    caller_embedding: Option<Vec<f32>>,
) -> Option<Vec<f32>> {
    if caller_embedding.is_some() {
        return caller_embedding;
    }
    if !v.ordered().iter().any(wants_a_vector) {
        return None;
    }
    let query = v.composition().intention.as_ref().map(|i| i.query.trim())?;
    if query.is_empty() {
        return None;
    }
    match embed_query_text(query).await {
        QueryEmbed::Embedded(e) => Some(e),
        QueryEmbed::Unavailable => None,
    }
}

/// Whether this node's act searches by vector. Read off the declared mechanic rather than a
/// hardcoded act list, so a new act served by the wide arm is covered without an edit here.
fn wants_a_vector(node: &StageNode) -> bool {
    match node {
        StageNode::Act(inv) => declaration(&inv.act)
            .and_then(|d| d.served_by)
            .is_some_and(|m| m == "search_wide"),
        StageNode::Combine(_) => false,
    }
}

/// Hydrate the RETURNED stages' rows, one batched read for the whole response.
///
/// Batched across every returned arm at once rather than per arm, because `hit_identities` costs one
/// statement per CALL and the arms are independent — two arms would otherwise be two round trips for
/// no reason. The arms stay separate in the response; only the read is shared.
///
/// **A returned stage that is not `Resources` is not hydrated here**, and today that is only
/// `survey`, whose fragment the compiler cannot yet emit — so a composition naming it fails loudly
/// at Postgres long before this. Region hydration is a declared hole, not a silent one.
///
/// The open tier comes back BESIDE the views rather than merged into them, because `with` is
/// per-arm. `[fixed — 2026-08-09]` Merging it into the shared views filled `open_meta` on every arm
/// as soon as ANY arm asked, so an arm that asked for nothing received `Some({})` — "this resource
/// has no open metadata" where the truth is "you did not ask". That is the exact conflation the
/// sibling test's own doc forbids, and it passed only because that test used a single-arm
/// composition. Found in review.
async fn hydrate(
    pool: &PgPool,
    principal: ProfileId,
    v: &ValidatedComposition,
    rows: &QueryRows,
) -> ApiResult<Hydrated> {
    let mut ids: Vec<ResourceId> = Vec::new();

    for spec in v.returns() {
        for hit in rows.hits_for(spec.stage.as_str()) {
            if hit.kind == "resource" {
                ids.push(ResourceId::from(hit.id));
            }
        }
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(Hydrated::default());
    }

    let views = readback::hit_identities(pool, principal, &ids)
        .await
        .map_err(|e| opaque(anyhow::anyhow!(e)))?;

    // ONE statement for the whole response if ANY arm asked — the read is shared, the APPLICATION
    // is not. Its predecessor on the list surface ran a per-view read and a 13-row page cost 28
    // statements, measured; the loop is the defect, not the section.
    let open_meta = if v
        .returns()
        .iter()
        .any(|r| r.with.contains(&ResourceSection::OpenMeta))
    {
        readback::meta_batch(pool, principal, &ids)
            .await
            .map_err(|e| opaque(anyhow::anyhow!(e)))?
            .into_iter()
            .map(|(id, rb)| (id.uuid(), serde_json::Value::Object(rb.open)))
            .collect()
    } else {
        HashMap::new()
    };

    Ok(Hydrated {
        views: views.into_iter().map(|v| (v.id.uuid(), v)).collect(),
        open_meta,
    })
}

/// What the hydrating reads returned, kept SEPARATE so each arm applies only what it asked for.
#[derive(Default)]
struct Hydrated {
    views: HashMap<Uuid, ResourceView>,
    /// Read once for the whole response; applied only to arms whose `with` names `open-meta`.
    /// An id absent here after the read means the resource genuinely has no open properties, which
    /// is an EMPTY tier (`{}`) rather than a missing one.
    open_meta: HashMap<Uuid, serde_json::Value>,
}

/// Build the response from the plan, the rows and the hydrated views. **Pure** — every database
/// question has already been asked, which is what makes the derivations below testable without one.
fn assemble(v: &ValidatedComposition, rows: &QueryRows, hydrated: &Hydrated) -> QueryResponse {
    let by_name: HashMap<&str, &StageNode> =
        v.ordered().iter().map(|n| (n.name().as_str(), n)).collect();

    let returned = v
        .returns()
        .iter()
        .filter_map(|spec| {
            let node = by_name.get(spec.stage.as_str())?;
            Some((spec.stage.clone(), stage_result(node, spec, rows, hydrated)))
        })
        .collect();

    let stages = v
        .ordered()
        .iter()
        .filter_map(|node| stage_trace(node, rows))
        .collect();

    QueryResponse {
        returned,
        trace: CompositionTrace {
            meta_detail: v.composition().meta_detail,
            stages,
        },
    }
}

/// The numbers every stage discloses, whether or not its rows come back.
///
/// One computation for both the result and the trace, because the contract requires them to be
/// identical — the trace covers every stage and the results cover only the returned ones, so these
/// are the ONLY numbers a reader gets for an intermediate stage. Two computations would eventually
/// disagree, and a caller reading a returned stage's `input_ids` in one place and the trace's in
/// another would have no way to tell which was right.
struct StageNumbers {
    disposition: StageDisposition,
    input_ids: i64,
    input_unusable: i64,
    input_contributed: Option<i64>,
    relation: Option<StageRelation>,
    input_source: Option<InputSource>,
}

fn stage_numbers(node: &StageNode, rows: &QueryRows) -> StageNumbers {
    let name = node.name().as_str();
    let tally = rows.tally(name);
    let produced = tally.map(|t| t.produced).unwrap_or(0);

    let (relation, input_source, input_ids) = match node {
        StageNode::Act(inv) => match &inv.input {
            Some(StageInput::Caller { relation, ids }) => (
                Some(*relation),
                Some(InputSource::Caller),
                ids.ids.len() as i64,
            ),
            Some(StageInput::Upstream { relation, stage }) => (
                Some(*relation),
                Some(InputSource::Upstream {
                    stage: stage.clone(),
                }),
                // The upstream stage's OWN tally — the count of what it produced is exactly the
                // count of what this stage was handed. There is no second question to ask.
                rows.tally(stage.as_str()).map(|t| t.produced).unwrap_or(0),
            ),
            None => (None, None, 0),
        },
        // A combinator's input is its own inputs' outputs; it declares no relation of its own.
        StageNode::Combine(cn) => (
            None,
            None,
            cn.inputs
                .iter()
                .filter_map(|s| rows.tally(s.as_str()))
                .map(|t| t.produced)
                .sum(),
        ),
    };

    StageNumbers {
        // **A refusal outranks the row count, and that ordering is the whole point.** A refused
        // stage's CTE is `WHERE false`, so its tally is `produced = 0` — byte-identical to an honest
        // empty. Reading the tally first would render `embedding_unavailable` as *"asked, nothing
        // matched"*: a rephrase-and-retry suggestion for a question that was never asked.
        disposition: if rows.refusal(name).is_some() {
            StageDisposition::Refused
        } else if produced > 0 {
            StageDisposition::Answered
        } else {
            StageDisposition::Empty
        },
        input_ids,
        input_unusable: tally.map(|t| t.unusable).unwrap_or(0),
        input_contributed: input_contributed(node, relation.as_ref(), produced),
        relation,
        input_source,
    }
}

/// How many of the handed-in ids actually contributed — **or `None`, which never means zero**.
///
/// Three answers, in this order:
///
/// 1. The act does not declare [`Disclosure::InputContribution`] → `None`. `follow-from` is the
///    case: its walk builds a path array that knows which seed reached which node and then discards
///    it, so seed provenance is gone before the function returns. The obvious fallback is worse than
///    null — `hop > 0` means a seed never appears in its own output, so the `bound` reading would
///    report 0 contributed out of 10 on a stage that just returned forty good neighbours.
/// 2. The input is an ANCHOR (a cogmap or context id) → `None`. An anchor id can never appear in a
///    resource output, so the literal answer is a permanent 0 on a stage that may have returned
///    fifty rows. `[decided — 2026-08-09, Pete]` Kept as null rather than reported as zero; the cost
///    is that "knowable in advance from `discloses`" now has an input-shape caveat, and whether the
///    declaration model should express that is an OPEN question — the real one being what signal a
///    caller can actually act on.
/// 3. A `bound` over resource ids → the produced count, exactly. A bound narrows *within* the set it
///    was handed, so output ⊆ input by construction and "how many of yours are in the output" is
///    "how many came out". Not an estimate.
fn input_contributed(
    node: &StageNode,
    relation: Option<&StageRelation>,
    produced: i64,
) -> Option<i64> {
    let StageNode::Act(inv) = node else {
        return None;
    };
    let decl = declaration(&inv.act)?;
    if !decl.discloses.contains(&Disclosure::InputContribution) {
        return None;
    }
    match &inv.input {
        // 0 of 0 is honest, and is why `find-about-anywhere` declares the disclosure despite
        // accepting no input at all: omitting it would claim "cannot", the word reserved for an act
        // whose mechanic destroys the answer.
        None => Some(0),
        Some(StageInput::Caller { ids, .. })
            if matches!(
                ids.kind,
                temper_core::types::query::IdKind::Cogmap
                    | temper_core::types::query::IdKind::Context
            ) =>
        {
            None
        }
        Some(_) => match relation {
            Some(StageRelation::Bound) => Some(produced),
            // A seed's contribution is not derivable from counts — reaching beyond the set means
            // the output is not a subset of it, so nothing here can say how many seeds led
            // somewhere. Unreachable today (no seeding act declares the disclosure) and answered
            // honestly rather than plausibly.
            _ => None,
        },
    }
}

fn stage_result(
    node: &StageNode,
    spec: &ReturnSpec,
    rows: &QueryRows,
    hydrated: &Hydrated,
) -> StageResult {
    let name = spec.stage.as_str();
    let wants_open_meta = spec.with.contains(&ResourceSection::OpenMeta);
    let n = stage_numbers(node, rows);
    let act = act_of(node);
    let decl = declaration(&act);
    let terms = match node {
        StageNode::Act(inv) => decl
            .as_ref()
            .map(|d| applied_terms(&inv.terms, d))
            .unwrap_or_default(),
        StageNode::Combine(_) => Default::default(),
    };

    let hits: Vec<ResourceHit> = rows
        .hits_for(name)
        .into_iter()
        // An id absent from the batch stopped being visible between the two statements — a dropped
        // row, not a fault. Same convention as every other set read in this crate.
        .filter_map(|h| {
            let mut resource = hydrated.views.get(&h.id)?.clone();
            // Applied per ARM, not per response: absent means NOT REQUESTED and `{}` means requested
            // and empty, and both survive the wire. An arm that asked and whose resource has no open
            // properties still gets `{}` — the read returning no row for it is an empty tier.
            if wants_open_meta {
                // **The `unwrap_or_else` arm is defensive and is NOT the source of the `{}`.**
                // `meta_batch` groups the property ROWS it read, so a resource reaches this map
                // whenever it has any property at all — and `create_resource` always writes the
                // managed identity keys, so the open tier arrives as an already-empty `{}` and the
                // default never runs. It is reachable only for a visible resource with zero
                // property rows of any tier, which no write path produces.
                // `[measured — 2026-08-09]` Mutating the default to `Null` survives the whole
                // suite; mutating the WHOLE expression is caught by both open-meta tests. So this
                // is an equivalent mutant, not a coverage gap — do not "close" it with a fixture
                // that cannot reach the branch.
                resource.open_meta = Some(
                    hydrated
                        .open_meta
                        .get(&h.id)
                        .cloned()
                        .unwrap_or_else(|| serde_json::Value::Object(Default::default())),
                );
            }
            Some(ResourceHit {
                resource,
                scoring: Scoring {
                    score_kind: decl.as_ref().and_then(|d| d.score_kind())?,
                    score: h.quantity.unwrap_or_default() as f32,
                },
                // Declared as fillable only by the wide arm, and nothing fills it yet: the
                // fragments collapse to a per-resource score and the argmin that would recover the
                // closest chunk is not emitted. A named remainder — `discloses` says which acts
                // COULD, and this is where the "could" is still not "does".
                located_at: None,
            })
        })
        .collect();

    let produced = produced_for(&act, hits);
    StageResult {
        act,
        disposition: n.disposition,
        orders_by: decl.as_ref().and_then(|d| d.orders_by.clone()),
        produced,
        extent: extent_of(node, rows, &terms),
        // Carried only by acts that can produce one WITHOUT a second query, and none can: the
        // fragments return a page, not a count. Absent rather than guessed from the page size.
        total: None,
        terms_applied: terms,
        narrowed_by: narrowed_by(node),
        input_ids: n.input_ids,
        input_contributed: n.input_contributed,
        input_unusable: n.input_unusable,
    }
}

/// The output variant the act DECLARES, so the response cannot contradict `ValidationOutcome`'s
/// promise about it.
///
/// `[fixed — 2026-08-09]` This was `StageOutput::Resources` unconditionally, so a region-producing
/// act would have answered `resources` with an empty list — the currency tag saying one thing while
/// the declaration promised another, which is precisely what tagging by currency exists to prevent.
/// Unreachable today because `survey` compiles to the absent placeholder and Postgres errors first,
/// which is why review found it by reading rather than by running.
///
/// **Region hydration is still a hole, and this makes it a LOUD one**: a region stage answers
/// `regions` with no rows rather than `resources` with no rows, so the emptiness is attributable to
/// the missing hydration instead of looking like a currency mismatch.
fn produced_for(act: &ActName, hits: Vec<ResourceHit>) -> StageOutput {
    match declaration(act).and_then(|d| d.produces) {
        Some(temper_core::types::query::IdKind::Region) => StageOutput::Regions { hits: vec![] },
        _ => StageOutput::Resources { hits },
    }
}

fn stage_trace(node: &StageNode, rows: &QueryRows) -> Option<StageTrace> {
    let n = stage_numbers(node, rows);
    Some(StageTrace {
        stage: node.name().clone(),
        act: act_of(node),
        disposition: n.disposition,
        relation: n.relation,
        input_source: n.input_source,
        input_ids: n.input_ids,
        input_contributed: n.input_contributed,
        input_unusable: n.input_unusable,
        narrowed_by: narrowed_by(node),
        // Set only where the per-resource meta budget bit. **Nothing truncates, and nothing HONOURS
        // `meta_detail` either** — it is echoed into the trace and read nowhere else, so `full` and
        // `none` behave exactly like `surviving`. `[corrected — 2026-08-09]` This comment claimed it
        // was "threaded and honoured"; only the first half was true. Absent is still the truthful
        // value for this field — a zeroed pair would claim nothing was dropped, which is a claim no
        // budget was ever consulted to make.
        meta_truncated: None,
    })
}

/// Is there more than you got back?
///
/// `Partial` iff a `limit` was applied and the stage produced exactly that many. **It over-reports
/// at the boundary and never under-reports**, which is the direction that matters: a result set of
/// exactly `limit` rows with nothing beyond it is reported `partial`, and the caller pages once more
/// to find nothing. Claiming `complete` on a truncated set would be a false claim about the corpus.
///
/// `survey` is `Indeterminate`: its funnel width does not select from a set, it PRODUCES one, so
/// there is no "rest" to have more of.
fn extent_of(
    node: &StageNode,
    rows: &QueryRows,
    terms: &std::collections::BTreeMap<BoundTerm, i64>,
) -> Extent {
    // **A refused stage never consulted the corpus, so it cannot report completeness over it.**
    // `[fixed — 2026-08-09]` It fell through to `complete` — no rows, no limit matched — which is
    // the same false claim as `complete` over a truncated set, and this function's own doc already
    // refuses that one. `disposition` disambiguates it only for a reader who checks both fields,
    // and the whole ordering rule elsewhere in this file exists so nobody has to. Found in review.
    if rows.refusal(node.name().as_str()).is_some() {
        return Extent::Indeterminate {
            reason: "the stage refused, so nothing was asked of the corpus and there is no \
                     remainder to report"
                .to_string(),
        };
    }
    if act_of(node) == ActName::Survey {
        return Extent::Indeterminate {
            reason:
                "a region funnel produces its candidate set rather than selecting from one, so \
                     there is no remainder to report"
                    .to_string(),
        };
    }
    let produced = rows
        .tally(node.name().as_str())
        .map(|t| t.produced)
        .unwrap_or(0);
    match terms.get(&BoundTerm::Limit) {
        Some(limit) if produced >= *limit => Extent::Partial,
        _ => Extent::Complete,
    }
}

/// The filters this stage applied, echoed back.
///
/// Counts are absent, never zero: `admitted`/`excluded` ride only where an act computes them for
/// free, and none does — the fragments apply their filters inside a single scan and do not count
/// what they dropped. Requiring them would reintroduce the second query `Extent` exists to avoid.
fn narrowed_by(node: &StageNode) -> Vec<NarrowedBy> {
    let StageNode::Act(inv) = node else {
        return vec![];
    };
    let mut out = Vec::new();
    if let Some(f) = &inv.resource_filter {
        for dt in &f.doc_type {
            out.push(NarrowedBy {
                key: "doc_type".to_string(),
                value: dt.clone(),
                admitted: None,
                excluded: None,
            });
        }
    }
    out
}

fn act_of(node: &StageNode) -> ActName {
    match node {
        StageNode::Act(inv) => inv.act.clone(),
        // A combinator runs no act. `Admit` is the anti-act and is the honest stand-in: it names
        // "nothing was asked of the corpus here", which is exactly what a set union is.
        StageNode::Combine(_) => ActName::Admit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::query::{
        validate, ActInvocation, Composition, IdKind, IdSet, Intention, OutcomeDeclaration,
        RefusalReason, ResourceFilter, ReturnSpec, StageName,
    };
    use temper_substrate::readback::query_exec::{HitRow, TallyRow};

    fn name(s: &str) -> StageName {
        StageName::parse(s).unwrap()
    }

    fn act_node(n: &str, act: ActName, input: Option<StageInput>) -> StageNode {
        StageNode::Act(ActInvocation {
            name: name(n),
            act,
            input,
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    }

    fn plan(stages: Vec<StageNode>, returns: Vec<&str>) -> ValidatedComposition {
        let c = Composition {
            outcome: OutcomeDeclaration {
                returns: returns
                    .into_iter()
                    .map(|s| ReturnSpec {
                        stage: name(s),
                        with: vec![],
                    })
                    .collect(),
            },
            intention: Some(Intention {
                query: "composable".to_string(),
                embedded: false,
            }),
            meta_detail: Default::default(),
            bounds: Default::default(),
            stages,
        };
        validate(&c).expect("plan is valid")
    }

    fn tally(stage: &str, produced: i64, unusable: i64) -> TallyRow {
        TallyRow {
            stage: stage.to_string(),
            produced,
            unusable,
        }
    }

    fn hit(stage: &str, id: Uuid, q: f64) -> HitRow {
        HitRow {
            stage: stage.to_string(),
            id,
            kind: "resource".to_string(),
            quantity: Some(q),
        }
    }

    /// **`terms_applied` reports the APPLIED page, and the applied page is the clamped one.**
    ///
    /// [`applied_terms`] exists so the statement and the response cannot claim different page sizes
    /// — `paging_for` reads it to BIND and this module reads it to REPORT. Only the binding consumer
    /// had a test: the assembler could have echoed the request unclamped, or reported nothing at
    /// all, and every test stayed green. Mutation-probed both ways.
    ///
    /// `find-exact` publishes `limit: 50` and no `offset` ceiling, so one term is clamped and the
    /// other passes through — which is what distinguishes "clamps" from "returns a constant".
    #[test]
    fn the_page_a_stage_reports_is_the_clamped_one_the_statement_actually_ran() {
        let asked =
            std::collections::BTreeMap::from([(BoundTerm::Limit, 999), (BoundTerm::Offset, 5)]);
        let node = StageNode::Act(ActInvocation {
            name: name("hits"),
            act: ActName::FindExact,
            input: None,
            terms: asked.clone(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        });
        let v = plan(vec![node], vec!["hits"]);
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("hits", 0, 0)],
            refusals: vec![],
        };

        let r = assemble(&v, &rows, &Hydrated::default());
        let applied = &r.returned[&name("hits")].terms_applied;

        assert_eq!(
            applied.get(&BoundTerm::Limit),
            Some(&50),
            "the ceiling find-exact publishes, not the 999 the caller asked for"
        );
        assert_eq!(
            applied.get(&BoundTerm::Offset),
            Some(&5),
            "a term below any ceiling passes through unchanged"
        );
        assert_ne!(
            *applied, asked,
            "reporting the request back would make `terms_applied` an echo rather than a disclosure"
        );
    }

    #[test]
    fn every_returned_stage_is_keyed_by_its_own_name_and_every_stage_is_traced() {
        // The trace covers stages nobody asked to have returned — that is what lets a reader decide
        // whether stage 1 earned its place in the pipe.
        let v = plan(
            vec![
                act_node("hits", ActName::FindExact, None),
                act_node(
                    "narrowed",
                    ActName::FindExact,
                    Some(StageInput::Upstream {
                        relation: StageRelation::Bound,
                        stage: name("hits"),
                    }),
                ),
            ],
            vec!["narrowed"],
        );
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("hits", 3, 0), tally("narrowed", 0, 0)],
            refusals: vec![],
        };

        let r = assemble(&v, &rows, &Hydrated::default());

        assert_eq!(r.returned.len(), 1, "exactly what `returns` asked for");
        assert!(r.returned.contains_key(&name("narrowed")));
        assert_eq!(r.trace.stages.len(), 2, "every stage, returned or not");
    }

    #[test]
    fn a_refused_stage_is_refused_and_never_reported_as_an_honest_empty() {
        // **The collapse this whole path is built to prevent.** A refused stage's CTE is
        // `WHERE false`, so its tally is `produced = 0` — identical to a stage that asked and found
        // nothing. Only the refusal distinguishes them, and rendering `refused` as `empty` tells a
        // caller to rephrase a question that was never asked.
        let v = plan(
            vec![act_node("wide", ActName::FindAboutAnywhere, None)],
            vec!["wide"],
        );
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("wide", 0, 0)],
            refusals: vec![PlanRefusal {
                stage: Some(name("wide")),
                reason: RefusalReason::EmbeddingUnavailable,
                detail: "the server could not compute one".to_string(),
            }],
        };

        let r = assemble(&v, &rows, &Hydrated::default());

        assert_eq!(
            r.returned[&name("wide")].disposition,
            StageDisposition::Refused
        );
        assert_eq!(r.trace.stages[0].disposition, StageDisposition::Refused);
    }

    /// **A refusal is PER STAGE**, and one composition holding both is the only fixture that can
    /// say so.
    ///
    /// Every refusal test above is single-stage, where "this stage refused" and "the composition
    /// refused" are indistinguishable — so `refusal()` ignoring the stage name it was handed
    /// survived, and one stage's `embedding_unavailable` marking EVERY stage refused was
    /// unwitnessed at the layer that RENDERS it. The compiler asserts the same property on the
    /// emitted text; nothing asserted it on the response.
    ///
    /// The healthy stage is what makes the test bite: without it there is no arm whose disposition
    /// a leaking refusal could wrongly change.
    #[test]
    fn a_stage_refusing_does_not_refuse_the_healthy_stage_beside_it() {
        let v = plan(
            vec![
                act_node("wide", ActName::FindAboutAnywhere, None),
                act_node("exact", ActName::FindExact, None),
            ],
            vec!["wide", "exact"],
        );
        let id = Uuid::from_u128(1);
        let rows = QueryRows {
            hits: vec![hit("exact", id, 0.7)],
            tallies: vec![tally("wide", 0, 0), tally("exact", 1, 0)],
            refusals: vec![PlanRefusal {
                stage: Some(name("wide")),
                reason: RefusalReason::EmbeddingUnavailable,
                detail: "the server could not compute one".to_string(),
            }],
        };

        let r = assemble(&v, &rows, &Hydrated::default());

        assert_eq!(
            r.returned[&name("wide")].disposition,
            StageDisposition::Refused,
            "the stage that could not be served"
        );
        assert_eq!(
            r.returned[&name("exact")].disposition,
            StageDisposition::Answered,
            "a refusal must not travel to a stage that ran — the composition did not refuse, one \
             stage did"
        );
        // And the same distinction in the trace, which is where a reader looks for it.
        let refused: Vec<&StageDisposition> = r
            .trace
            .stages
            .iter()
            .filter(|s| s.disposition == StageDisposition::Refused)
            .map(|s| &s.disposition)
            .collect();
        assert_eq!(refused.len(), 1, "exactly one stage refused: {:?}", r.trace);
    }

    /// **`Extent::Partial` — produced by no test until now**, though `extent` is how a caller decides
    /// whether to page at all.
    ///
    /// `Partial` iff a limit was applied and the stage produced exactly that many. It over-reports
    /// at the boundary and never under-reports, which is the direction that matters: claiming
    /// `complete` over a truncated set would be a false claim about the corpus.
    #[test]
    fn a_stage_that_filled_its_page_reports_partial_rather_than_claiming_completeness() {
        let node = StageNode::Act(ActInvocation {
            name: name("hits"),
            act: ActName::FindExact,
            input: None,
            terms: std::collections::BTreeMap::from([(BoundTerm::Limit, 2)]),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        });
        let v = plan(vec![node], vec!["hits"]);

        let full = QueryRows {
            hits: vec![],
            tallies: vec![tally("hits", 2, 0)],
            refusals: vec![],
        };
        assert_eq!(
            assemble(&v, &full, &Hydrated::default()).returned[&name("hits")].extent,
            Extent::Partial,
            "a page filled to its limit may have more behind it"
        );

        // The other side of the same boundary, so the test measures the RULE and not a constant.
        let short = QueryRows {
            hits: vec![],
            tallies: vec![tally("hits", 1, 0)],
            refusals: vec![],
        };
        assert_eq!(
            assemble(&v, &short, &Hydrated::default()).returned[&name("hits")].extent,
            Extent::Complete,
            "a page the limit did not fill has nothing behind it"
        );
    }

    #[test]
    fn a_stage_that_asked_and_matched_nothing_is_empty_rather_than_refused() {
        // The other half of the pair. Same zero rows, no refusal — `empty` means "asked, no match",
        // and rephrasing helps this one.
        let v = plan(
            vec![act_node("hits", ActName::FindExact, None)],
            vec!["hits"],
        );
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("hits", 0, 0)],
            refusals: vec![],
        };
        let r = assemble(&v, &rows, &Hydrated::default());
        assert_eq!(
            r.returned[&name("hits")].disposition,
            StageDisposition::Empty
        );
    }

    #[test]
    fn the_ids_a_stage_was_handed_are_the_upstream_stages_own_tally() {
        // Derived, not measured — and exact. The count of what the upstream produced IS the count of
        // what this stage was handed; asking the database again would ask one statement the same
        // question twice.
        let v = plan(
            vec![
                act_node("hits", ActName::FindExact, None),
                act_node(
                    "narrowed",
                    ActName::FindExact,
                    Some(StageInput::Upstream {
                        relation: StageRelation::Bound,
                        stage: name("hits"),
                    }),
                ),
            ],
            vec!["narrowed"],
        );
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("hits", 12, 0), tally("narrowed", 4, 0)],
            refusals: vec![],
        };

        let r = assemble(&v, &rows, &Hydrated::default());
        assert_eq!(r.returned[&name("narrowed")].input_ids, 12);
        let traced = r
            .trace
            .stages
            .iter()
            .find(|s| s.stage == name("narrowed"))
            .unwrap();
        assert_eq!(
            traced.input_ids, 12,
            "the trace and the result must not disagree about one number"
        );
    }

    #[test]
    fn a_bounds_contribution_is_the_produced_count_because_output_is_a_subset_of_input() {
        let v = plan(
            vec![
                act_node("hits", ActName::FindExact, None),
                act_node(
                    "narrowed",
                    ActName::FindExact,
                    Some(StageInput::Upstream {
                        relation: StageRelation::Bound,
                        stage: name("hits"),
                    }),
                ),
            ],
            vec!["narrowed"],
        );
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("hits", 12, 0), tally("narrowed", 4, 0)],
            refusals: vec![],
        };
        let r = assemble(&v, &rows, &Hydrated::default());
        assert_eq!(
            r.returned[&name("narrowed")].input_contributed,
            Some(4),
            "a bound narrows WITHIN its input, so what came out is what contributed"
        );
    }

    #[test]
    fn an_act_that_cannot_report_a_contribution_says_null_and_never_zero() {
        // `follow-from` declares no `InputContribution`: its walk discards seed provenance. Zero
        // here would be plausible, quiet and false — a seed never appears in its own output, so the
        // bound reading would print 0 on a stage that returned forty neighbours.
        let v = plan(
            vec![act_node(
                "near",
                ActName::FollowFrom,
                Some(StageInput::Caller {
                    relation: StageRelation::Seed,
                    ids: IdSet {
                        kind: IdKind::Resource,
                        provenance: None,
                        ids: vec![Uuid::now_v7(), Uuid::now_v7()],
                    },
                }),
            )],
            vec!["near"],
        );
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("near", 40, 0)],
            refusals: vec![],
        };
        let r = assemble(&v, &rows, &Hydrated::default());
        // One assertion, not two: `assert_ne!(…, Some(0))` after `assert_eq!(…, None)` cannot
        // fail — it is implied by the line above it, and an assertion that cannot fail reads as
        // protection that is not there. The name carries the "never zero" claim; the comment
        // carries why it matters.
        assert_eq!(r.returned[&name("near")].input_contributed, None);
    }

    /// A minimal hydrated view for a given id. Only `id` is load-bearing below — the assembler
    /// keys on it and copies the rest through.
    fn view(id: Uuid) -> ResourceView {
        ResourceView {
            id: ResourceId::from(id),
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
            created: chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).expect("epoch"),
            updated: chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).expect("epoch"),
            body_hash: None,
            ingest_state: None,
            body_storage: None,
            managed_meta: Default::default(),
            open_meta: None,
            content: None,
        }
    }

    #[test]
    fn a_hit_whose_row_vanished_between_the_two_statements_is_dropped_never_substituted() {
        // `hit_identities` is a second statement; a resource can stop being visible between them.
        // The convention every set read in this crate follows is that the row is simply absent.
        //
        // **Two hits, one hydrated**, because the single-hit version of this test was passing
        // vacuously: with an empty hydration map there is nothing to substitute, so "dropped" and
        // "handed the wrong view" are indistinguishable. Found by probing — the substitution probe
        // did not fail it. The row that survives must be the one that was actually hydrated.
        let present = Uuid::now_v7();
        let vanished = Uuid::now_v7();
        let v = plan(
            vec![act_node("hits", ActName::FindExact, None)],
            vec!["hits"],
        );
        let rows = QueryRows {
            hits: vec![hit("hits", vanished, 0.9), hit("hits", present, 0.5)],
            tallies: vec![tally("hits", 2, 0)],
            refusals: vec![],
        };
        let hydrated = Hydrated {
            views: HashMap::from([(present, view(present))]),
            open_meta: HashMap::new(),
        };

        let r = assemble(&v, &rows, &hydrated);
        match &r.returned[&name("hits")].produced {
            StageOutput::Resources { hits } => {
                assert_eq!(hits.len(), 1, "the vanished row is dropped");
                assert_eq!(
                    hits[0].resource.id.uuid(),
                    present,
                    "and the surviving hit carries ITS OWN view, never a neighbour's"
                );
                assert_eq!(
                    hits[0].scoring.score, 0.5,
                    "with its own score — pairing a view with another row's quantity would be \
                     a confidently wrong answer"
                );
            }
            other => panic!("expected resources, got {other:?}"),
        }
    }

    /// **The `discloses` GATE, tested where it can actually be reached.**
    ///
    /// Review found both `None`-returning clauses of `input_contributed` uncovered: deleting either
    /// left all eight tests green. The act-level one is unreachable through `assemble` — every act
    /// that declares no `InputContribution` also accepts no bound, so the relation arm swallows it
    /// first — so it is exercised against the function directly, the same level `narrowing_for`'s
    /// tests use in the compiler.
    #[test]
    fn an_act_that_declares_no_contribution_is_gated_before_the_relation_is_consulted() {
        // `follow-from` declares none. Hand it a BOUND anyway — a shape `validate` refuses, which is
        // exactly why the guard cannot be reached from above and must be asserted here.
        let node = act_node(
            "n",
            ActName::FollowFrom,
            Some(StageInput::Upstream {
                relation: StageRelation::Bound,
                stage: name("up"),
            }),
        );
        assert_eq!(
            input_contributed(&node, Some(&StageRelation::Bound), 40),
            None,
            "the declaration decides before the relation does; 40 would be a plausible lie"
        );
    }

    /// **An ANCHOR input reports null, not zero** — the clause the commit message dedicates a
    /// decision to, and which no test reached until review said so.
    #[test]
    fn an_anchor_input_reports_null_because_its_ids_can_never_be_in_a_resource_output() {
        let node = act_node(
            "n",
            ActName::FindAboutWithin,
            Some(StageInput::Caller {
                relation: StageRelation::Bound,
                ids: IdSet {
                    kind: IdKind::Cogmap,
                    provenance: None,
                    ids: vec![Uuid::now_v7()],
                },
            }),
        );
        // `find-about-within` DOES declare the disclosure, so the act-level gate passes and this is
        // the anchor arm alone. Without it the answer would be `Some(50)` — "50 of your 1 id
        // contributed" — which is the plausible-but-false number the decision exists to prevent.
        assert_eq!(
            input_contributed(&node, Some(&StageRelation::Bound), 50),
            None
        );
    }

    /// A resource bound on the same act DOES report, so the test above is about the anchor and not
    /// about the act.
    #[test]
    fn a_resource_bound_on_the_same_act_still_reports_its_contribution() {
        let node = act_node(
            "n",
            ActName::FindAboutWithin,
            Some(StageInput::Upstream {
                relation: StageRelation::Bound,
                stage: name("up"),
            }),
        );
        assert_eq!(
            input_contributed(&node, Some(&StageRelation::Bound), 50),
            Some(50)
        );
    }

    /// **A refused stage cannot claim `complete`.** It never consulted the corpus, so it has nothing
    /// to be complete about — the same false claim as `complete` over a truncated set.
    #[test]
    fn a_refused_stage_reports_an_indeterminate_extent_rather_than_complete() {
        let v = plan(
            vec![act_node("wide", ActName::FindAboutAnywhere, None)],
            vec!["wide"],
        );
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("wide", 0, 0)],
            refusals: vec![PlanRefusal {
                stage: Some(name("wide")),
                reason: RefusalReason::EmbeddingUnavailable,
                detail: "no vector".to_string(),
            }],
        };
        let r = assemble(&v, &rows, &Hydrated::default());
        assert!(
            matches!(
                r.returned[&name("wide")].extent,
                temper_core::types::query::Extent::Indeterminate { .. }
            ),
            "got: {:?}",
            r.returned[&name("wide")].extent
        );
    }

    /// A stage that ran and matched nothing IS complete — an honest zero is a complete answer.
    #[test]
    fn a_stage_that_ran_and_matched_nothing_is_complete_because_it_did_consult_the_corpus() {
        let v = plan(
            vec![act_node("hits", ActName::FindExact, None)],
            vec!["hits"],
        );
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("hits", 0, 0)],
            refusals: vec![],
        };
        let r = assemble(&v, &rows, &Hydrated::default());
        assert!(matches!(
            r.returned[&name("hits")].extent,
            temper_core::types::query::Extent::Complete
        ));
    }

    #[test]
    fn a_filter_is_echoed_back_without_counts_it_never_measured() {
        let mut node = act_node("hits", ActName::FindExact, None);
        if let StageNode::Act(a) = &mut node {
            a.resource_filter = Some(ResourceFilter {
                doc_type: vec!["session".to_string()],
                ..Default::default()
            });
        }
        let v = plan(vec![node], vec!["hits"]);
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("hits", 0, 0)],
            refusals: vec![],
        };
        let r = assemble(&v, &rows, &Hydrated::default());
        let n = &r.returned[&name("hits")].narrowed_by;
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].key, "doc_type");
        assert_eq!(
            n[0].admitted, None,
            "absent, never a zero it did not measure"
        );
    }
}
