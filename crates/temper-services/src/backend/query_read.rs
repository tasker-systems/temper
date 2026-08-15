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
//! the refusals come back from Postgres. One disclosure number is derived rather than measured,
//! and it is exact rather than approximate: `input_ids` for an upstream-fed stage is the upstream
//! stage's own produced tally — asking again would be asking the same statement the same question
//! twice. (`input_contributed` was a second derived number until ratification ⟨6⟩/9d removed the
//! field — see the tombstone on `StageResult`.)

use std::collections::{BTreeSet, HashMap};

use sqlx::PgPool;
use uuid::Uuid;

use crate::backend::substrate_read::{embed_query_text, QueryEmbed};
use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::query::{
    applied_terms, declaration, emitted_fragment_for, validate, validate_shape, ActName,
    ActRefusal, Composition, Extent, NarrowedBy, PlanRefusal, QueryResponse, ResourceHit,
    ReturnSpec, Scoring, StageDisposition, StageInput, StageNode, StageOutput, StageResult,
    StageTrace, ValidatedComposition, ViaEntry,
};
use temper_core::types::query::{BoundTerm, CompositionTrace, InputSource, StageInputTrace};
use temper_core::types::resource_view::{ResourceSection, ResourceView};
use temper_substrate::readback;
use temper_substrate::readback::query_exec::{execute, QueryRows};
use temper_substrate::readback::query_plan::{compile, EMIT_FIND_WIDE};

/// Run a validated composition and answer in the shape `POST /api/query` publishes.
///
/// **A caller's precomputed vector arrives on each stage's `intention.embedding`**, not as a
/// parameter here — spec ⟨7⟩. The CLI computes one because it links temper-ingest; the ruby gem,
/// the TypeScript package and MCP cannot, so the server embeds on their behalf, and **that step
/// runs BEFORE `validate`** (a `ValidatedComposition` is sealed, and filling vectors into it after
/// the fact would need a side-channel this contract refuses).
///
/// **This function does not perform that step — [`prepare`] does, and it is the only way to build
/// the argument.** Taking a `ValidatedComposition` is what makes that structural rather than
/// remembered: there is no way to reach here having skipped the embed, because there is no other
/// constructor.
pub async fn run_composition(
    pool: &PgPool,
    principal: ProfileId,
    v: &ValidatedComposition,
) -> ApiResult<QueryResponse> {
    // A refusal from here is a compiler/validator contradiction, never a caller error — `validate`
    // has already refused everything a caller can get wrong. Rendering it as a 500 rather than a
    // 400 says so: telling a caller to fix a plan that is correct would send them in circles.
    let compiled = compile(v, principal).map_err(|r: PlanRefusal| {
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

/// Turn a caller's composition into one this server will run: shape-gate it, embed on its behalf,
/// then seal it.
///
/// ```text
/// deserialize        serde — rejects a malformed body before anything here runs
///   → validate_shape cheap, pure, no DB and no declarations (⟨3⟩'s expressibility pass)
///   → embed          only the intentions that need a vector and did not carry one
///   → validate       the full pass, capability included — the seal
///   → compile        [`run_composition`]
/// ```
///
/// **The shape gate is a COST gate and nothing else.** `[decided — 2026-08-13, Pete]` — *"if a
/// composition is structurally invalid then we don't want to pay the onnx cost"*. It never decides
/// what the caller is told: a plan it refuses still falls through to [`validate()`], which is the
/// sole authority on refusals and returns **every** one of them rather than the first, so a plan
/// with both a shape fault and a capability fault is still repaired in one round trip.
///
/// So shape is evaluated twice — once here, once inside [`validate()`]. A pure function over a small
/// struct; named so it is not later "discovered" as a defect.
///
/// **Parse-don't-validate is our line, not an external law, and this is where we put the seal.**
/// `[2026-08-13]` The alternative was to leave the seal ahead of the embed and hand `compile` a
/// side-channel `BTreeMap<StageName, Vec<f32>>`. It was declined: that is two sources for one
/// fact — the shape spec ⟨7⟩ removed from `compile`'s signature, reintroduced one layer down.
pub async fn prepare(mut c: Composition) -> Result<ValidatedComposition, Vec<PlanRefusal>> {
    if validate_shape(&c).is_empty() {
        embed_missing_intentions(&mut c).await;
    }
    validate(&c)
}

/// The query text this node needs the SERVER to embed, trimmed — or `None`, for any of four
/// different reasons that must not be collapsed.
///
/// One definition, read twice by [`embed_missing_intentions`] (once to collect, once to write
/// back), because a predicate spelled at both ends of that function is a predicate that can
/// disagree with itself about which stages it just embedded for.
///
/// `None` means, in order: this act does not search by vector ([`wants_a_vector`]); it carries no
/// question at all; the caller already sent a vector; or the question is empty. The last two of
/// those are the properties `resolve_embedding` had and this had to keep:
///
///   * **Embed only when some stage would USE the vector.** A composition of `find-exact` stages
///     paying ONNX produces a value nothing binds, and a failure then refuses nothing, having spent
///     the budget.
///   * **An empty or whitespace-only query is NOT an embedding attempt.** `shape.rs`'s
///     `[widened — 2026-08-09]` note records what happens when it is: the caller is told
///     `embedding_unavailable` — a server fault, for a question they never asked. Through
///     [`prepare`] the shape pass has already refused that plan and no embed runs at all, so this
///     arm is the property held **structurally**; it is spelled here anyway because it is this
///     function's contract, not its caller's.
///
/// The text is TRIMMED, matching `substrate_read::embed_query_if_missing`. Two questions differing
/// only in surrounding whitespace are one question, and embedding them separately would be two
/// vectors for one string.
fn text_to_embed(node: &StageNode) -> Option<&str> {
    if !wants_a_vector(node) {
        return None;
    }
    let StageNode::Act(inv) = node else {
        return None;
    };
    let intention = inv.intention.as_ref()?;
    if intention.embedding.is_some() {
        return None;
    }
    let query = intention.query.trim();
    (!query.is_empty()).then_some(query)
}

/// Every DISTINCT question this composition needs embedded, in a stable order.
///
/// **Distinct query TEXT, not per stage** — the property this collection exists to hold, and it is
/// two properties rather than one. Two stages naming the same string must not pay ONNX twice; and
/// they must not be able to receive two *different* vectors for one question, which would make
/// paraphrase-stability unmeasurable in exactly the way the retired envelope placement was trying
/// to protect. A `BTreeSet` gives both, and gives a deterministic embed order for free.
fn texts_to_embed(c: &Composition) -> BTreeSet<String> {
    c.stages
        .iter()
        .filter_map(text_to_embed)
        .map(str::to_string)
        .collect()
}

/// Fill in the vectors the caller could not compute, in place, before the plan is sealed.
///
/// `[replaced — 2026-08-13]` `resolve_embedding` used to sit here. It resolved ONE vector for the
/// whole composition, from `Composition.intention` — the field spec ⟨7⟩ moved onto each stage — and
/// handed it to `compile` as a parameter. Neither end of that survives, so this writes INTO the
/// plan rather than beside it.
///
/// **The attempt is [`embed_query_text`], not a second one.** Its doc says why it was extracted:
/// the query has to be embedded by the same plain `embed_text` path the corpus was ingested with,
/// so it lands in the stored chunks' vector space. A second implementation here would be a second
/// answer to *"which space is this vector in"*, and `/api/query` scores would quietly stop being
/// comparable with `/api/search`'s.
///
/// **A failed attempt writes nothing and refuses nothing here.** The stage keeps its `None`, and
/// `compile` renders it as [`temper_core::types::query::RefusalReason::EmbeddingUnavailable`]
/// against that stage — the
/// contract's one runtime refusal, reported where a reader is already looking. That is the split
/// [`QueryEmbed`]'s own doc names: `/api/search` collapses the outcome into a `degraded` boolean
/// because its arms are fixed, and this surface cannot.
///
/// One question that no stage can use is therefore **not** a failed composition: its siblings run,
/// and the refusal is per stage.
async fn embed_missing_intentions(c: &mut Composition) {
    let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
    for query in texts_to_embed(c) {
        if let QueryEmbed::Embedded(vector) = embed_query_text(&query).await {
            vectors.insert(query, vector);
        }
    }
    if vectors.is_empty() {
        return;
    }

    for node in &mut c.stages {
        let Some(query) = text_to_embed(node).map(str::to_string) else {
            continue;
        };
        let Some(vector) = vectors.get(&query) else {
            continue;
        };
        if let StageNode::Act(inv) = node {
            if let Some(intention) = inv.intention.as_mut() {
                intention.embedding = Some(vector.clone());
            }
        }
    }
}

/// Whether this node's act searches by vector. Read off the declared mechanic rather than a
/// hardcoded act list, so a new act served by the wide arm is covered without an edit here.
///
/// **Two hops, and the second one is why this is not a string comparison against `served_by`.**
/// `[fixed — 2026-08-12]` This asked `served_by == "search_wide"`. `served_by` names what the
/// deployed `/api/search` door calls, and that moved to `query_find_wide` when the door gained a
/// resource bound — so this returned `false` for BOTH wide acts, [`text_to_embed`] found nothing to
/// embed, `compile` took its `None` arm, and every find-about stage refused
/// `EmbeddingUnavailable` for any caller that cannot precompute a vector. Which is the whole class
/// of caller this module's own header says the server embeds on behalf of.
///
/// The repair is not a newer literal. `served_by` is a name that is ALLOWED to move — it follows the
/// deployed door — so anything comparing it to a spelling here is a copy waiting to go stale a third
/// time. Going through [`emitted_fragment_for`] asks the question the answer actually depends on:
/// *does the compiler emit the wide core for this act?* Both hops are then single-sourced —
/// `CALLABLE_FRAGMENTS` owns the mapping, [`EMIT_FIND_WIDE`] owns the core's name — and this
/// function holds no name of its own.
fn wants_a_vector(node: &StageNode) -> bool {
    match node {
        StageNode::Act(inv) => declaration(&inv.act)
            .and_then(|d| d.served_by)
            .and_then(|mechanic| emitted_fragment_for(&mechanic))
            .is_some_and(|fragment| fragment == EMIT_FIND_WIDE),
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
        trace: CompositionTrace { stages },
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
    /// Present iff the stage refused — the ONE construction both the result and the trace carry
    /// (the pair rule), built from the per-stage refusal `compile` recorded.
    refusal: Option<ActRefusal>,
    input_ids: i64,
    input_unusable: i64,
    /// One entry per input. `[widened — 2026-08-14]` was a `relation`/`input_source` pair
    /// describing *the* input; a stage carrying a seed AND a bound has two, and a single relation
    /// filled from whichever came first is half the truth with no marker saying so.
    inputs: Vec<StageInputTrace>,
}

fn stage_numbers(node: &StageNode, rows: &QueryRows) -> StageNumbers {
    let name = node.name().as_str();
    let tally = rows.tally(name);
    let produced = tally.map(|t| t.produced).unwrap_or(0);

    let (inputs, input_ids): (Vec<StageInputTrace>, i64) = match node {
        StageNode::Act(inv) => {
            let traced: Vec<StageInputTrace> = inv
                .inputs
                .iter()
                .map(|i| match i {
                    StageInput::Caller { relation, ids } => StageInputTrace {
                        relation: *relation,
                        source: InputSource::Caller,
                        ids: ids.ids.len() as i64,
                    },
                    StageInput::Upstream { relation, stage } => StageInputTrace {
                        relation: *relation,
                        source: InputSource::Upstream {
                            stage: stage.clone(),
                        },
                        // The upstream stage's OWN tally — the count of what it produced is
                        // exactly the count of what this stage was handed. There is no second
                        // question to ask.
                        ids: rows.tally(stage.as_str()).map(|t| t.produced).unwrap_or(0),
                    },
                })
                .collect();
            let total = traced.iter().map(|t| t.ids).sum();
            (traced, total)
        }
        // A combinator's input is its own inputs' outputs; it declares no relation of its own — so
        // it contributes a total and no per-input entries, exactly as it carried no relation before.
        StageNode::Combine(cn) => (
            Vec::new(),
            cn.inputs
                .iter()
                .filter_map(|s| rows.tally(s.as_str()))
                .map(|t| t.produced)
                .sum(),
        ),
    };

    // The ONE construction of the refusal both carriers share — built here so `StageResult.refusal`
    // and `StageTrace.refusal` cannot disagree, the same way this struct already shares the input
    // numbers. `compile` recorded the per-stage refusal; this is where it becomes the wire's.
    let refusal = rows.refusal(name).map(|r| ActRefusal {
        reason: r.reason.clone(),
        detail: r.detail.clone(),
    });

    StageNumbers {
        // **A refusal outranks the row count, and that ordering is the whole point.** A refused
        // stage's CTE is `WHERE false`, so its tally is `produced = 0` — byte-identical to an honest
        // empty. Reading the tally first would render `embedding_unavailable` as *"asked, nothing
        // matched"*: a rephrase-and-retry suggestion for a question that was never asked.
        disposition: if refusal.is_some() {
            StageDisposition::Refused
        } else if produced > 0 {
            StageDisposition::Answered
        } else {
            StageDisposition::Empty
        },
        refusal,
        input_ids,
        input_unusable: tally.map(|t| t.unusable).unwrap_or(0),
        inputs,
    }
}

// `input_contributed` used to be derived here — the act-declaration gate, the anchor-input null,
// and the bound-equals-produced arm. Removed by ratification ⟨6⟩/9d `[2026-08-09, Pete]` with the
// field it fed: redundant where filled (the bound arm restated the produced count) and null where
// interesting. Returns with the field when a walk carries its origin.

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
                // **The one place raw `jsonb` becomes the typed contract.** A malformed payload
                // yields an EMPTY list rather than a partial one — `serde_json` fails the whole
                // array if any entry is wrong, and half a provenance trail is worse than none,
                // because the caller cannot tell which half is missing. That is a deliberate
                // trade and its cost is stated: an unparseable `via` is indistinguishable here
                // from a walk that reached nothing, and the fragment is the only writer, so the
                // case means the two have drifted rather than that a caller did anything.
                via: h
                    .via
                    .clone()
                    .and_then(|v| serde_json::from_value::<Vec<ViaEntry>>(v).ok())
                    .unwrap_or_default(),
            })
        })
        .collect();

    let produced = produced_for(&act, hits);
    StageResult {
        act,
        disposition: n.disposition,
        refusal: n.refusal,
        orders_by: decl.as_ref().and_then(|d| d.orders_by.clone()),
        produced,
        extent: extent_of(node, rows, &terms),
        // Carried only by acts that can produce one WITHOUT a second query, and none can: the
        // fragments return a page, not a count. Absent rather than guessed from the page size.
        total: None,
        terms_applied: terms,
        narrowed_by: narrowed_by(node),
        input_ids: n.input_ids,
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
        refusal: n.refusal,
        inputs: n.inputs,
        input_ids: n.input_ids,
        input_unusable: n.input_unusable,
        narrowed_by: narrowed_by(node),
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
    // **Unreachable through `validate`, and kept anyway** — the same status
    // `query_plan.rs`'s `_` arm carries, said here too because a fact documented in one file and
    // silent in another teaches the next reader that only one of them is unreachable.
    // `[since 2026-08-12]` `survey` left `CALLABLE_FRAGMENTS`, so `validate` refuses it as
    // `NotSeparablyReachable` and no `ValidatedComposition` can carry a `survey` stage to this
    // function. It stays because the arm is a fact about the ACT — a funnel produces its candidate
    // set rather than selecting from one — that a lens slot restores rather than invents, and
    // because the fallback below would otherwise answer `complete` over a corpus survey never
    // counted.
    //
    // **NOTHING TESTS THIS ARM, and that is a second fact rather than the same one restated.**
    // `[declared — 2026-08-12, re-review]` The paragraph above justified itself by saying that a
    // fact documented in one file and silent in another teaches the next reader that only one of
    // them is unreachable — and then reproduced exactly that asymmetry one fact over, stating the
    // unreachability and not the coverage. `a_refused_stage_reports_an_indeterminate_extent_rather_
    // than_complete` is the only test in this file that reaches `Extent::Indeterminate`, and it
    // drives a `find-about-anywhere` stage carrying an `EmbeddingUnavailable` refusal — so it
    // exercises the REFUSAL arm above, never this one. `ActName::Survey` appears in this file
    // exactly once, here. Same status as `query_plan.rs`'s `_` arm, now said the same way.
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
/// `[widened — 2026-08-14]` **This reported `doc_type` and nothing else, because `doc_type` was the
/// only narrowing anything applied.** The other six were refused at validation, so a stage carrying
/// one never ran and had nothing to disclose. `find-resources-with` applies all EIGHT
/// `[widened again — 2026-08-15, the open-key slot]`, and a
/// disclosure that named one of them would be worse than none: a caller reading
/// `narrowed_by: [doc_type]` on a stage that also narrowed by stage, owner and three tags would
/// conclude those had not been applied.
///
/// **Where this surfaces is the point.** A selection stage is refused in `returns`
/// (`StageNotReturnable`), so its `StageResult` never exists — but `stage_trace` builds a
/// `StageTrace` for EVERY stage, returned or not, and that is where a composition's intermediate
/// work is legible. Without this, the one act whose entire output is a narrowing would be the one
/// act whose narrowing the trace could not describe.
///
/// Counts stay absent rather than zero: no fragment computes what it dropped, and requiring it would
/// reintroduce the second query `Extent` exists to avoid.
fn narrowed_by(node: &StageNode) -> Vec<NarrowedBy> {
    let StageNode::Act(inv) = node else {
        return vec![];
    };
    let entry = |key: String, value: String| NarrowedBy {
        key,
        value,
        admitted: None,
        excluded: None,
    };

    // **The edge filter is echoed too, and it did not used to be** `[fixed — 2026-08-14, found in
    // review]`. This function returned early unless a `resource_filter` was present, so a walk
    // narrowed to `edge_kinds: [contains]` came back with a smaller result set and nothing in the
    // response saying which narrowing produced it.
    //
    // That is the exact MIRROR of the defect the "narrowings this door declares but does not apply"
    // block in `capability.rs` was written for: there a filter was echoed and not applied, here it
    // was applied and not echoed. Both leave a caller unable to reconcile the rows with the
    // question, and the second is the one that survives every refusal test — the answer is correct,
    // only the disclosure is missing.
    //
    // It became reachable in this same change: the unconditional `edge_filter` refusal retired when
    // `follow-from` gained a fragment that binds `p_edge_kinds`/`p_labels`, so before that no stage
    // carrying one ever ran.
    let mut out: Vec<NarrowedBy> = inv
        .edge_filter
        .iter()
        .flat_map(|e| {
            e.edge_kinds
                .iter()
                // The WIRE spelling, via serde — `format!("{k:?}").to_lowercase()` yields
                // `leadsto` where the contract says `leads_to`, and a disclosure that echoes a
                // value the caller cannot have sent is worse than a missing one.
                .map(|k| {
                    entry(
                        "edge_kind".to_string(),
                        serde_json::to_string(k)
                            .map(|s| s.trim_matches('"').to_string())
                            .unwrap_or_default(),
                    )
                })
                .chain(
                    e.labels
                        .iter()
                        .map(|l| entry("edge_label".to_string(), l.clone())),
                )
                // **The third axis is echoed for the same reason the first two are**
                // `[2026-08-15]`. An edge property predicate excludes HOPS, so a walk carrying one
                // returns a smaller set of nodes; leaving it out would recreate the
                // applied-but-not-echoed defect this block was written for, one axis over — and
                // recreate it silently, because every refusal test stays green when the answer is
                // correct and only the disclosure is missing.
                //
                // The value is the KEY, not the whole predicate. A caller reading the trace needs
                // to reconcile the rows with the question they asked, and the operator is in the
                // request they still hold; rendering `values` here would put caller-supplied JSON
                // of arbitrary size into every response's disclosure.
                .chain(
                    e.properties
                        .iter()
                        .map(|p| entry("edge_property".to_string(), p.key.clone())),
                )
        })
        .collect();

    let Some(f) = &inv.resource_filter else {
        return out;
    };

    // One entry PER VALUE for the repeated fields, which is the shape the incumbent `doc_type` loop
    // already had. A comma-joined single entry would be shorter and would make `a,b` — one tag
    // containing a comma — indistinguishable from two tags.
    out.extend(
        f.doc_type
            .iter()
            .map(|v| entry("doc_type".to_string(), v.clone()))
            .chain(f.tags.iter().map(|v| entry("tags".to_string(), v.clone())))
            // The facet's own key rides in the disclosure key, so `facet:domain = search` says which
            // facet was narrowed on rather than just that one was.
            .chain(
                f.facets
                    .iter()
                    .map(|p| entry(format!("facet:{}", p.key), p.value.clone())),
            )
            // **The open-key slot is echoed for the same reason every other field is**
            // `[2026-08-15]`. It narrows the returned SET, so leaving it out recreates the
            // applied-but-not-echoed defect this block was written for — and recreates it
            // silently, because every refusal test stays green when the answer is correct and only
            // the disclosure is missing. It is the field most exposed to that, since it is the one
            // whose key the caller invents: a reader who cannot see `derived_from` in the trace
            // has no way to tell a predicate that ran from one that was dropped.
            //
            // The value is the KEY, not the whole predicate — the same call the edge sibling
            // makes, for the same two reasons. The operator is in the request the caller still
            // holds, and rendering `values` would put caller-supplied JSON of arbitrary size into
            // every response's disclosure.
            .chain(
                f.properties
                    .iter()
                    .map(|p| entry("property".to_string(), p.key.clone())),
            ),
    );
    for (key, value) in [
        ("stage", f.stage.as_ref()),
        ("status", f.status.as_ref()),
        ("owner", f.owner.as_ref()),
        ("title_contains", f.title_contains.as_ref()),
    ] {
        if let Some(v) = value {
            out.push(entry(key.to_string(), v.clone()));
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
        ActInvocation, Intention, OutcomeDeclaration, RefusalReason, ResourceFilter, ReturnSpec,
        StageName, StageRelation,
    };
    use temper_substrate::readback::query_exec::{HitRow, TallyRow};

    fn name(s: &str) -> StageName {
        StageName::parse(s).unwrap()
    }

    /// A minimal legal act node, carrying a question because a find act is refused without one.
    ///
    /// **The query text is arbitrary here and is allowed to be**, which is worth saying out loud
    /// given how this module got its `intention: None`. Every test below hand-builds its
    /// [`QueryRows`] and asserts on `assemble`'s derivations; none compiles SQL and none consults a
    /// corpus, so nothing can read this string. A test where the text IS load-bearing belongs in
    /// `query_run_composition_test.rs`, against a real database that can disagree with it.
    ///
    /// `[2026-08-13]` The per-stage move left this helper at `intention: None`, and eleven of this
    /// module's twelve tests panicked in `plan`'s `validate` — caught by running `--lib`, which the
    /// hand-off's verified baseline did not include. Compile-clean, clippy-clean, and red.
    /// **An APPLIED edge filter is echoed, and the echo uses the wire spelling.**
    ///
    /// `[added — 2026-08-14, with the fix it witnesses]` `narrowed_by` returned early unless a
    /// `resource_filter` was present, so a walk narrowed by `edge_kinds` came back smaller with
    /// nothing recording why. The mirror of the defect `capability.rs`'s "declared but not applied"
    /// block exists for — and the harder one to notice, because the ANSWER is correct and only the
    /// disclosure is missing, so every refusal test stays green.
    ///
    /// The spelling half is asserted separately because it fails silently in the other direction: a
    /// `{:?}` of `EdgeKind::LeadsTo` is `LeadsTo`, and lowercased it is `leadsto` — a value no
    /// caller could have sent, echoed back as though they had.
    #[test]
    fn an_applied_edge_filter_is_echoed_in_the_narrowing_disclosure() {
        use temper_core::types::graph::EdgeKind;
        use temper_core::types::query::{EdgeFilter, PropertyOp, PropertyPredicate};

        let mut node = act_node("near", ActName::FollowFrom, None);
        if let StageNode::Act(a) = &mut node {
            a.edge_filter = Some(EdgeFilter {
                edge_kinds: vec![EdgeKind::LeadsTo, EdgeKind::Contains],
                labels: vec!["cites".to_string()],
                properties: vec![PropertyPredicate {
                    key: "confidence".to_string(),
                    op: PropertyOp::HasKey,
                }],
            });
        }

        let disclosed = narrowed_by(&node);
        let pairs: Vec<(&str, &str)> = disclosed
            .iter()
            .map(|n| (n.key.as_str(), n.value.as_str()))
            .collect();

        assert!(
            pairs.contains(&("edge_kind", "leads_to")),
            "the WIRE spelling, not the Rust variant name; got {pairs:?}"
        );
        assert!(
            pairs.contains(&("edge_kind", "contains")),
            "one entry per value, not a comma-joined one; got {pairs:?}"
        );
        assert!(
            pairs.contains(&("edge_label", "cites")),
            "the label axis is disclosed too; got {pairs:?}"
        );
        // The third axis, and it is disclosed by KEY rather than by whole predicate: the caller
        // still holds the operator, and rendering `values` would put arbitrary caller JSON into
        // every response.
        assert!(
            pairs.contains(&("edge_property", "confidence")),
            "an applied edge property predicate is echoed, or the walk returns fewer nodes with \
             nothing saying why; got {pairs:?}"
        );
        assert_eq!(
            disclosed.len(),
            4,
            "two kinds, one label and one property key, and nothing invented; got {pairs:?}"
        );
    }

    /// A stage with neither filter discloses nothing — an empty list, not an entry saying so.
    #[test]
    fn a_stage_that_narrowed_by_nothing_discloses_an_empty_list() {
        let node = act_node("hits", ActName::FindExact, None);
        assert!(narrowed_by(&node).is_empty());
    }

    fn act_node(n: &str, act: ActName, input: Option<StageInput>) -> StageNode {
        StageNode::Act(ActInvocation {
            name: name(n),
            act,
            intention: Some(Intention {
                query: "a question this test never reads".to_string(),
                embedding: None,
            }),
            inputs: input.into_iter().collect(),
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
            stages,
        };
        validate(&c).expect("plan is valid")
    }

    /// A find-about stage asking a specific question — the act the server embeds for.
    fn find_about(n: &str, query: &str) -> StageNode {
        let mut node = act_node(n, ActName::FindAboutAnywhere, None);
        if let StageNode::Act(a) = &mut node {
            a.intention = Some(Intention {
                query: query.to_string(),
                embedding: None,
            });
        }
        node
    }

    fn composition(stages: Vec<StageNode>) -> Composition {
        let returns = stages
            .iter()
            .map(|n| ReturnSpec {
                stage: n.name().clone(),
                with: vec![],
            })
            .collect();
        Composition {
            outcome: OutcomeDeclaration { returns },
            stages,
        }
    }

    /// **A `find-exact` stage is never embedded for**, because nothing would bind the vector.
    ///
    /// Paying ONNX for a composition that binds no vector spends the budget for a value nothing
    /// reads — and worse, a failure then *refuses nothing*, having spent it. This is the cheap,
    /// model-free witness for that property; no test that actually runs the embedder can show that
    /// a call did **not** happen.
    #[test]
    fn a_find_exact_stage_is_never_embedded_for_because_nothing_would_bind_the_vector() {
        assert_eq!(
            text_to_embed(&act_node("hits", ActName::FindExact, None)),
            None
        );
        assert!(texts_to_embed(&composition(vec![act_node(
            "hits",
            ActName::FindExact,
            None
        )]))
        .is_empty());
    }

    /// A caller who computed its own vector is not made to pay for a second one.
    #[test]
    fn a_vector_the_caller_supplied_is_never_recomputed() {
        let mut node = find_about("hits", "kestrel");
        if let StageNode::Act(a) = &mut node {
            a.intention.as_mut().unwrap().embedding = Some(vec![0.5; 768]);
        }
        assert_eq!(text_to_embed(&node), None);
    }

    /// **An empty question is not an embedding attempt** — `[widened — 2026-08-09]`, `shape.rs`.
    ///
    /// Embedding it and failing tells the caller `embedding_unavailable`: a server fault, for a
    /// question they never asked. Through [`prepare`] the shape pass refuses this plan and no embed
    /// runs at all; this asserts the function's own contract, so the property does not depend on
    /// every future caller remembering to gate.
    #[test]
    fn a_whitespace_only_question_is_not_an_embedding_attempt() {
        assert_eq!(text_to_embed(&find_about("hits", "   \t ")), None);
        assert_eq!(text_to_embed(&find_about("hits", "")), None);
    }

    /// **Once per distinct question, not once per stage.** Two stages naming the same string must
    /// not pay ONNX twice — and must not be able to receive two DIFFERENT vectors for one question,
    /// which is the property the retired envelope placement was protecting and the one thing
    /// per-stage intentions could plausibly have cost.
    #[test]
    fn two_stages_asking_the_same_question_are_embedded_once() {
        let texts = texts_to_embed(&composition(vec![
            find_about("a", "kestrel"),
            find_about("b", "kestrel"),
        ]));
        assert_eq!(texts.len(), 1, "one question, one embed: {texts:?}");
    }

    /// The whole point of the move: two stages, two questions, two vectors.
    #[test]
    fn two_stages_asking_different_questions_are_embedded_separately() {
        let texts = texts_to_embed(&composition(vec![
            find_about("a", "kestrel"),
            find_about("b", "sourdough"),
        ]));
        assert_eq!(texts.len(), 2, "got: {texts:?}");
    }

    /// Whitespace is not part of a question. Matches `substrate_read::embed_query_if_missing`,
    /// which trims before it embeds — two vectors for one string would otherwise be reachable by
    /// nothing more than a stray space.
    #[test]
    fn questions_differing_only_in_surrounding_whitespace_are_one_question() {
        let texts = texts_to_embed(&composition(vec![
            find_about("a", "kestrel"),
            find_about("b", "  kestrel\n"),
        ]));
        assert_eq!(texts, BTreeSet::from(["kestrel".to_string()]));
    }

    /// **The shape gate never decides what the caller is told.** A plan that is both inexpressible
    /// AND asks for something this server has not built comes back with BOTH refusals, because
    /// [`validate()`] runs whatever the gate concluded — the gate's only job is to keep an
    /// inexpressible plan from paying ONNX.
    #[tokio::test]
    async fn a_shape_refusal_does_not_cost_the_caller_the_rest_of_its_refusals() {
        // `find-about-within` with no intention: a shape refusal (`MissingIntention`). Returning
        // an unknown stage: a capability refusal, raised by `validate_returns`.
        let mut node = act_node("hits", ActName::FindAboutWithin, None);
        if let StageNode::Act(a) = &mut node {
            a.intention = None;
        }
        let c = Composition {
            outcome: OutcomeDeclaration {
                returns: vec![ReturnSpec {
                    stage: name("hits"),
                    with: vec![ResourceSection::Body],
                }],
            },
            stages: vec![node],
        };

        let refusals = prepare(c).await.expect_err("the plan is refused");
        assert!(
            refusals
                .iter()
                .any(|r| r.reason == RefusalReason::MissingIntention),
            "the shape refusal: {refusals:?}"
        );
        assert!(
            refusals.len() > 1,
            "every refusal in one round trip, not just the one the gate saw: {refusals:?}"
        );
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
            via: None,
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
        let mut node = act_node("hits", ActName::FindExact, None);
        if let StageNode::Act(a) = &mut node {
            a.terms = asked.clone();
        }
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

        // The pair rule (ADJ-3): the reason reaches the reader on BOTH carriers, identically —
        // the trace is the only refusal record for an intermediate stage, and the result must not
        // say less than the trace for a returned one.
        let result_refusal = r.returned[&name("wide")]
            .refusal
            .as_ref()
            .expect("a refused result carries its reason");
        let trace_refusal = r.trace.stages[0]
            .refusal
            .as_ref()
            .expect("a refused trace entry carries its reason");
        assert_eq!(result_refusal.reason, RefusalReason::EmbeddingUnavailable);
        assert_eq!(
            result_refusal, trace_refusal,
            "one construction, two carriers"
        );
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
        // And the reason does not travel either: the answered stage's `refusal` is absent on both
        // carriers, so `Some` here would be one stage wearing its neighbour's refusal.
        assert!(r.returned[&name("exact")].refusal.is_none());
        assert!(r
            .trace
            .stages
            .iter()
            .find(|s| s.stage == name("exact"))
            .expect("the exact stage is traced")
            .refusal
            .is_none());
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
        let mut node = act_node("hits", ActName::FindExact, None);
        if let StageNode::Act(a) = &mut node {
            a.terms = std::collections::BTreeMap::from([(BoundTerm::Limit, 2)]);
        }
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

    // Five tests about `input_contributed` stood here — the bound-equals-produced derivation, the
    // cannot-report null, the declaration gate, the anchor-input null, and its resource-bound
    // contrast. Deleted with the field (ratification ⟨6⟩/9d, 2026-08-09); the refusal-pair
    // assertions above are the field's replacement disclosure, and the derivation returns with the
    // field when a walk carries its origin.

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

    /// `[moved to the selection act — 2026-08-14]` This built the filter on a `find-exact` stage,
    /// which no longer validates: a resource filter on any act but `find-resources-with` is refused
    /// rather than applied. The property is unchanged and so is the assertion — an echoed narrowing
    /// carries no count, because no fragment computes what it dropped and a zero would be a number
    /// nobody measured.
    ///
    /// It also now covers the WHOLE filter rather than `doc_type` alone, which is the half that was
    /// untestable before: with six of the seven fields refused, a `narrowed_by` that reported only
    /// `doc_type` was indistinguishable from one that reported everything it was given.
    ///
    /// `[2026-08-15]` The filter is EIGHT fields now, and the eighth is carried here for the reason
    /// this test exists: a field applied by the fragment and absent from the echo passes every
    /// refusal test, because the answer is correct and only the disclosure is missing.
    #[test]
    fn a_filter_is_echoed_back_without_counts_it_never_measured() {
        let mut node = act_node("sel", ActName::FindResourcesWith, None);
        if let StageNode::Act(a) = &mut node {
            a.resource_filter = Some(ResourceFilter {
                doc_type: vec!["session".to_string(), "task".to_string()],
                tags: vec!["ci".to_string()],
                stage: Some("in-progress".to_string()),
                // The open-key slot `[2026-08-15]`. It is the field most exposed to the
                // applied-but-not-echoed defect, because its key is one the CALLER invents: a
                // reader who cannot find `derived_from` in the trace cannot tell a predicate that
                // ran from one that was dropped.
                properties: vec![temper_core::types::query::PropertyPredicate {
                    key: "derived_from".to_string(),
                    op: temper_core::types::query::PropertyOp::HasKey,
                }],
                ..Default::default()
            });
        }
        // **The selection is not RETURNED, and cannot be** — it orders nothing, so its rows have no
        // quantity to score them and `returns` refuses it as `stage_not_returnable`. Its echo lives
        // in the TRACE, which carries every stage regardless. That is not a workaround for this
        // test; it is the whole reason `narrowed_by` had to keep working for non-returned stages,
        // since the one act whose entire output is a narrowing is the one act whose narrowing would
        // otherwise be undescribable.
        let sink = act_node(
            "hits",
            ActName::FindExact,
            Some(StageInput::Upstream {
                relation: StageRelation::Bound,
                stage: name("sel"),
            }),
        );
        let v = plan(vec![node, sink], vec!["hits"]);
        let rows = QueryRows {
            hits: vec![],
            tallies: vec![tally("sel", 0, 0), tally("hits", 0, 0)],
            refusals: vec![],
        };
        let r = assemble(&v, &rows, &Hydrated::default());
        let n = &r
            .trace
            .stages
            .iter()
            .find(|t| t.stage == name("sel"))
            .expect("the selection appears in the trace though it returns nothing")
            .narrowed_by;
        // One entry PER VALUE: two doc types, one tag, one stage, one open-key predicate.
        assert_eq!(n.len(), 5, "got: {n:?}");
        assert_eq!(
            n.iter().filter(|e| e.key == "doc_type").count(),
            2,
            "a multi-value field echoes once per value, so `a,b` cannot be read as one value \
             containing a comma"
        );
        assert!(n.iter().any(|e| e.key == "tags" && e.value == "ci"));
        assert!(n
            .iter()
            .any(|e| e.key == "stage" && e.value == "in-progress"));
        // The KEY is the value, matching the edge sibling: the operator is in the request the
        // caller still holds, and echoing `values` would put caller-supplied JSON of arbitrary
        // size into every response's disclosure.
        assert!(n
            .iter()
            .any(|e| e.key == "property" && e.value == "derived_from"));
        assert!(
            n.iter()
                .all(|e| e.admitted.is_none() && e.excluded.is_none()),
            "absent, never a zero it did not measure"
        );
    }

    /// **The acts the compiler emits the wide core for must want a vector — and when they stop, the
    /// symptom is a plausible refusal rather than an error.**
    ///
    /// `wants_a_vector` is what decides whether the server embeds on the caller's behalf. Answer
    /// `false` for a find-about stage and nothing fails loudly: [`text_to_embed`] finds nothing to
    /// embed, `compile` takes its no-embedding arm, and the stage refuses `EmbeddingUnavailable` —
    /// which is indistinguishable from outside from a genuine ONNX failure. That is how a hardcoded
    /// `"search_wide"` here survived the `served_by` repoint of 2026-08-12 with no test going red:
    /// there was no find-about case in `query_run_composition_test.rs`, and `/api/query` had no
    /// route, so the door would have opened already broken.
    ///
    /// `[closed — 2026-08-13]` `query_run_composition_test.rs::server_side_embedding` now drives a
    /// find-about stage through `prepare → compile → execute` against a real corpus, so the same
    /// drift reddens an integration test as well as this family assertion. **That does not retire
    /// this one**: the integration test covers the two acts that exist, and this covers whichever
    /// acts the family declares, which is the half that a NEW act added later falls into.
    ///
    /// **Derived from the family, not written as an act list**, so an act added later that the
    /// compiler emits the wide core for is covered with no edit here — and so the count assertion,
    /// not a per-act one, is what catches the drift. An EMPTY derivation is the exact defect: it
    /// means `served_by` no longer maps through `CALLABLE_FRAGMENTS`.
    #[test]
    fn every_act_the_compiler_emits_the_wide_core_for_wants_a_vector() {
        use temper_core::types::query::search_family;

        let wide: Vec<ActName> = search_family()
            .into_iter()
            .filter(|d| {
                d.served_by
                    .as_deref()
                    .and_then(emitted_fragment_for)
                    .is_some_and(|f| f == EMIT_FIND_WIDE)
            })
            .map(|d| d.name)
            .collect();

        assert_eq!(
            wide.len(),
            2,
            "`find-about-anywhere` and `find-about-within` are both served by the wide core; the \
             derivation found {wide:?}. Zero means `served_by` and `CALLABLE_FRAGMENTS` have drifted \
             apart and every find-about stage is about to refuse EmbeddingUnavailable"
        );

        for act in wide {
            assert!(
                wants_a_vector(&act_node("s", act.clone(), None)),
                "{act:?} is served by the wide core and must want a vector"
            );
        }

        assert!(
            !wants_a_vector(&act_node("s", ActName::FindExact, None)),
            "the exact arm binds no vector — embedding for it pays ONNX inference to produce a \
             value nothing binds, and a failure would then refuse a stage that never needed one"
        );
    }
}
