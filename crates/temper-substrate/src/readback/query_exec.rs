//! Run a [`CompiledQuery`] and hand back its two row classes, typed.
//!
//! This is the third runtime-`sqlx` class in this module, and it inherits the second's reason: the
//! statement text is assembled at runtime because the DAG shape is not known at compile time, so
//! there is no `query!` macro to check it. What is NOT dynamic is the result shape — every compiled
//! statement answers in exactly the column list `query_plan`'s final select emits, which is why the
//! rows below are typed structs rather than a bag of `Value`s.
//!
//! **This layer runs the statement and nothing else.** It does not hydrate, does not decide a
//! disposition, and does not consult the plan. Those belong to the assembler in temper-services,
//! which has the composition and the declarations; keeping them out of here is what stops the
//! substrate from forming an opinion about what a stage MEANT.

use anyhow::Result;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use temper_core::types::query::PlanRefusal;

use super::query_plan::{CompiledQuery, QueryBind};

/// The literal every hit row carries in `row_class`.
const HIT: &str = "hit";

/// One row of a returned stage: membership plus the quantity its act ordered by.
///
/// The quantity is `Option` because the column is typed for the whole result set, and a tally row
/// carries NULL in it. A hit row from a real act always has one — every act that produces anything
/// declares an `orders_by`.
#[derive(Debug, Clone)]
pub struct HitRow {
    pub stage: String,
    pub id: Uuid,
    /// `resource` or `region` — the currency, as the stage contract carries it.
    pub kind: String,
    pub quantity: Option<f64>,
}

/// One stage's disclosure numbers, for EVERY stage — including the ones whose rows nobody asked
/// for, which is the only way they can appear in the trace at all.
#[derive(Debug, Clone)]
pub struct TallyRow {
    pub stage: String,
    /// How many rows the stage produced. A zero here is what distinguishes an `empty` stage from
    /// one that never ran — a distinction the hits alone cannot make, since both look like no rows.
    pub produced: i64,
    /// How many of the ids the stage was handed it could not use — invisible, nonexistent and
    /// malformed as ONE number. Always 0 where the input came from an upstream stage (already
    /// gated) or from an anchor pair (see `query_plan::NO_UNUSABLE`).
    pub unusable: i64,
}

/// Everything one compiled statement returned, split by row class — **plus the refusals the
/// compiler recorded**, which the rows themselves cannot express.
#[derive(Debug, Clone, Default)]
pub struct QueryRows {
    pub hits: Vec<HitRow>,
    pub tallies: Vec<TallyRow>,
    /// Carried through from [`CompiledQuery::refusals`], and **carried rather than left behind on
    /// purpose**.
    ///
    /// A refused stage's CTE is `WHERE false`, so its tally is `produced = 0, unusable = 0` — byte
    /// for byte what an honest empty looks like. Nothing in the row set distinguishes *"asked, no
    /// match"* from *"this act could not answer"*, and those are opposite things to tell a caller.
    ///
    /// This type is what an assembler holds, so leaving the refusals on the `CompiledQuery` would
    /// make rendering a refusal as `empty` the thing that happens when you simply do not think
    /// about it. Here it takes a deliberate omission instead.
    pub refusals: Vec<PlanRefusal>,
}

impl QueryRows {
    pub fn tally(&self, stage: &str) -> Option<&TallyRow> {
        self.tallies.iter().find(|t| t.stage == stage)
    }

    /// Why this stage refused, if it did. `None` means it ran — an empty result from here is an
    /// honest empty, and the two must never be collapsed.
    pub fn refusal(&self, stage: &str) -> Option<&PlanRefusal> {
        self.refusals
            .iter()
            .find(|r| r.stage.as_ref().is_some_and(|s| s.as_str() == stage))
    }

    /// One stage's hits, ordered by its own quantity, best first.
    ///
    /// **Sorted here, per stage, and never across stages.** A `UNION ALL` has no ordering
    /// guarantee, so the row order Postgres happens to return is not the order the act's
    /// `orders_by` declares — and a stage that claims an ordering it does not deliver is a quiet
    /// lie in the direction this whole surface exists to avoid. (The act's own `ORDER BY … LIMIT`
    /// still decides WHICH rows come back; this only restores the order they arrive in.)
    ///
    /// That it takes a stage name rather than sorting `hits` in place is the structural half of
    /// `no-cross-act-ranking`: there is no call that produces one ordered list two acts' rows share,
    /// so summing or interleaving them cannot happen by reaching for the obvious accessor.
    pub fn hits_for(&self, stage: &str) -> Vec<&HitRow> {
        let mut rows: Vec<&HitRow> = self.hits.iter().filter(|h| h.stage == stage).collect();
        // Descending: every quantity in this family is better-when-higher (`fts_norm`, `vec_norm`,
        // `graph_score`, `region_score`). A row with no quantity sorts last rather than first — it
        // is the absence of a score, not a perfect one.
        rows.sort_by(|a, b| {
            b.quantity
                .unwrap_or(f64::NEG_INFINITY)
                .total_cmp(&a.quantity.unwrap_or(f64::NEG_INFINITY))
        });
        rows
    }
}

/// Run the statement.
///
/// The binds go on in the order [`compile`](super::query_plan::compile) pushed them, which is the order the `$n`
/// placeholders were emitted in — the two are one sequence by construction, not two lists kept in
/// step.
///
/// An embedding is bound as its pgvector TEXT rendering against the emitted `$n::vector` cast, the
/// same treatment `search_wide` gives its own: pgvector's input is textual, and there is no Rust
/// float array that maps to the type directly.
pub async fn execute(pool: &PgPool, compiled: &CompiledQuery) -> Result<QueryRows> {
    let mut q = sqlx::query(&compiled.sql);
    for bind in &compiled.binds {
        q = match bind {
            QueryBind::Profile(p) => q.bind(p.uuid()),
            QueryBind::Uuids(ids) => q.bind(ids.clone()),
            QueryBind::Text(t) => q.bind(t.clone()),
            QueryBind::Int(i) => q.bind(*i),
            QueryBind::Embedding(e) => q.bind(super::format_pgvector(e)),
        };
    }

    let mut out = QueryRows {
        refusals: compiled.refusals.clone(),
        ..QueryRows::default()
    };
    for row in q.fetch_all(pool).await? {
        let stage: String = row.try_get("stage")?;
        let row_class: String = row.try_get("row_class")?;
        // Switched on the literal the compiler emitted, never inferred from a NULL id. "This row
        // has no id" and "this row is a tally" are different claims, and deriving one from the
        // other would make an act that legitimately produced a NULL id unreadable.
        if row_class == HIT {
            out.hits.push(HitRow {
                stage,
                id: row.try_get("id")?,
                kind: row.try_get("kind")?,
                quantity: row.try_get("quantity")?,
            });
        } else {
            out.tallies.push(TallyRow {
                stage,
                produced: row.try_get("produced")?,
                unusable: row.try_get("unusable")?,
            });
        }
    }
    Ok(out)
}
