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
    /// How a walk reached this row — the raw `via` column, `None` for every act that is not one.
    ///
    /// **Raw `jsonb` at this layer, typed at the assembler.** This crate has no dependency on
    /// `temper-core`'s wire types and should not grow one to carry a column through; the typed
    /// `ViaEntry` is where a caller reads it.
    ///
    /// **A malformed payload becomes an EMPTY list there, not an error** — see the note at that
    /// conversion for why, and for the cost it accepts. `[corrected — 2026-08-14, found in review]`
    /// This doc used to claim the opposite ("the parse failing is an error rather than a silent
    /// empty"), which was the behaviour I would have preferred and never the behaviour that
    /// shipped. A doc that describes a stricter contract than the code is worse than none: it is
    /// read as a guarantee.
    pub via: Option<serde_json::Value>,
    /// The region a `survey` row came from — the raw `region` column, `None` for every act that is
    /// not a survey.
    ///
    /// **A bare `Uuid` rather than a typed newtype**, for the reason stated one field up: this
    /// crate has no dependency on `temper-core`'s wire types and must not grow one to carry a
    /// column through. `HitRow::id` is already a bare `Uuid`; there is no region newtype in the
    /// codebase to reach for anyway.
    pub region: Option<Uuid>,
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
            QueryBind::Texts(t) => q.bind(t.clone()),
            QueryBind::Id(u) => q.bind(*u),
            QueryBind::Json(j) => q.bind(j.clone()),
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
                via: row.try_get("via")?,
                region: row.try_get("region")?,
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

/// `hits_for` is pure, so its two claims are decidable without a database — and both were
/// unwitnessed at every layer until now. Mutation-probed: dropping the stage filter and reversing
/// the comparator each survived the whole suite.
#[cfg(test)]
mod tests {
    use super::*;

    /// Ids are literal rather than generated: this crate mints UUIDv7 only, and nothing here turns
    /// on the id's shape — only on which rows come back and in what order.
    fn hit(stage: &str, id: u128, q: Option<f64>) -> HitRow {
        HitRow {
            stage: stage.to_string(),
            id: Uuid::from_u128(id),
            kind: "resource".to_string(),
            via: None,
            region: None,
            quantity: q,
        }
    }

    /// **A stage's hits are its own.** The structural half of `no-cross-act-ranking`: there is no
    /// call that produces one ordered list two acts' rows share, so summing or interleaving them
    /// cannot happen by reaching for the obvious accessor.
    ///
    /// The two stages hold DISJOINT rows on purpose. A fixture where both stages match the same
    /// resource cannot express this property — the filter and `true` return the same set — which is
    /// how the accessor stayed unwitnessed while a two-arm test passed over it.
    #[test]
    fn a_stages_hits_are_its_own_and_never_another_stages() {
        let rows = QueryRows {
            hits: vec![
                hit("left", 1, Some(0.9)),
                hit("right", 2, Some(0.8)),
                hit("left", 3, Some(0.7)),
            ],
            ..QueryRows::default()
        };

        let left = rows.hits_for("left");
        let right = rows.hits_for("right");
        assert_eq!(left.len(), 2, "got: {left:?}");
        assert_eq!(right.len(), 1, "got: {right:?}");
        assert!(
            left.iter().all(|h| h.stage == "left"),
            "an arm carrying another arm's row is a list two acts share: {left:?}"
        );
        assert!(right.iter().all(|h| h.stage == "right"), "got: {right:?}");
        // Named so the whole-set escape is closed: no accessor hands back every stage's rows.
        assert_eq!(
            left.len() + right.len(),
            rows.hits.len(),
            "the arms partition the row set — they do not each see all of it"
        );
    }

    /// Best first, and **a missing quantity sorts last rather than first** — it is the absence of a
    /// score, not a perfect one. `UNION ALL` has no ordering guarantee, so this is the only thing
    /// that makes a stage's declared `orders_by` true of what a caller receives.
    #[test]
    fn a_stages_hits_come_back_best_first_and_an_absent_quantity_sorts_last() {
        let rows = QueryRows {
            hits: vec![
                hit("a", 1, Some(0.2)),
                hit("a", 2, None),
                hit("a", 3, Some(0.9)),
                hit("a", 4, Some(0.5)),
            ],
            ..QueryRows::default()
        };

        let got: Vec<Option<f64>> = rows.hits_for("a").iter().map(|h| h.quantity).collect();
        assert_eq!(
            got,
            vec![Some(0.9), Some(0.5), Some(0.2), None],
            "descending, with the unscored row last"
        );
    }
}
