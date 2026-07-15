use anyhow::Result;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Job A (spec §1 "Job A — embed", §6a): embed every current, non-folded chunk's authored content
/// and write the 768-dim vector back to `kb_chunks.embedding` (the column the seed leaves empty —
/// spec line 249). Content lives in `kb_chunk_content` keyed by chunk, per the content-block
/// primitive (`kb_content_blocks → kb_chunks → kb_chunk_content`); the chunks already exist in the
/// seed, so this fills their embeddings rather than creating rows. Cosine never enters formation —
/// these embeddings feed only the downstream SQL readouts (content_cohesion, telos_alignment, …).
pub async fn embed_chunks(pool: &PgPool) -> Result<()> {
    // Embed only chunks that lack an embedding — embeddings are deterministic for unchanged content, so
    // re-runs (run_eval invokes the binary several times) skip already-embedded chunks instead of paying
    // the full ONNX cost again. (Content-mutation re-embed is a content_hash check — deferred to the
    // scenario-DSL work; this eval's content is immutable once seeded.)
    let chunks = sqlx::query(
        "SELECT ch.id AS chunk_id, cc.content \
         FROM kb_chunks ch \
         JOIN kb_chunk_content cc ON cc.chunk_id = ch.id \
         JOIN kb_content_blocks b ON b.id = ch.block_id \
         WHERE ch.is_current AND NOT b.is_folded AND ch.embedding IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for row in chunks {
        let chunk_id: Uuid = row.get("chunk_id");
        let content: String = row.get("content");
        if content.trim().is_empty() {
            continue;
        }
        // submodule call path (no re-exports in temper-ingest lib.rs); bge-768, l2-normalized.
        let embeddings = temper_ingest::embed::embed_texts(&[content.as_str()])?;
        let emb = &embeddings[0];
        let vec_lit = format!(
            "[{}]",
            emb.iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        sqlx::query("UPDATE kb_chunks SET embedding = $1::vector WHERE id = $2")
            .bind(vec_lit)
            .bind(chunk_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// A chunk is **stale** when its vector was not produced by the model this build embeds with.
///
/// One clause, and it subsumes both jobs that used to need two mechanisms:
///
/// - the original async-embed backfill (a deferred chunk has BOTH `embedding` and `embedded_with`
///   NULL, and `NULL IS DISTINCT FROM <sha>` is TRUE — so it is still caught), and
/// - **re-embedding after a model change**, which previously had no mechanism at all.
///
/// It is why the re-embed needs no backfill script: rows written before `embedded_with` existed carry
/// NULL, so they are stale *by definition* the moment the column ships — and any future model change
/// re-stales the index automatically. Marking dirty is a `NULL`, not a data migration.
///
/// **There is deliberately NO `embedding IS NULL OR` disjunct**, and that is load-bearing. With it, a
/// chunk whose content is blank — nothing to embed, so it is stamped with the current model and left
/// with a NULL vector — would stay stale FOREVER: `remaining` would never reach zero, the drain would
/// re-enqueue that resource every single minute in perpetuity embedding nothing, and `stale_summary`
/// would never converge, destroying the operator's only progress signal. The disjunct is redundant
/// (see the deferred-chunk case above) and it is a wedge. `_insert_chunk` keeps the two columns
/// coherent (no vector ⇒ no provenance), so this clause alone is complete for every row the projector
/// writes.
///
/// Scoping by `is_current` is what makes create-then-update supersede naturally: a body revise makes
/// the new generation current and the old non-current, so a job — whenever it runs — only ever embeds
/// the resource's *live* chunks.
pub const STALE_CHUNK_PREDICATE: &str = "ch.is_current \
     AND NOT b.is_folded \
     AND ch.embedded_with IS DISTINCT FROM $2";

/// Default chunk allowance for ONE dispatch invocation — **not** per resource.
///
/// The distinction is the whole point. The drain runs inside a serverless function with a wall-clock
/// limit, and `dispatch_tick` claims `DEFAULT_EMBED_DISPATCH_CAP` (5) resources and embeds them
/// SERIALLY in a single invocation. A per-resource budget of 64 would therefore permit 5 x 64 = 320
/// chunks per invocation — and the server runs ONNX single-threaded (`INTRA_THREADS_DEFAULT = 1`),
/// with no `maxDuration` declared in vercel.json. That is not a bound; it is the same cliff, moved.
///
/// So the caller threads ONE allowance across the whole claimed batch (see `dispatch_tick`), and a
/// resource that exhausts it is re-enqueued to resume next tick. Progress stays monotonic regardless
/// of resource size — prod's largest holds **939** chunks — and the per-invocation ceiling is a number
/// you can actually reason about against a timeout.
///
/// Deliberately conservative, and tunable without a rebuild via `TEMPER_EMBED_CHUNK_BUDGET`: the right
/// value depends on the function's real wall-clock limit and the box's real ms/chunk, neither of which
/// is measured yet (task `019f5892`). Raise it once they are.
pub const EMBED_CHUNK_BUDGET: i64 = 64;

/// Env override for the per-invocation chunk allowance.
pub const EMBED_CHUNK_BUDGET_ENV: &str = "TEMPER_EMBED_CHUNK_BUDGET";

/// Resolve the per-invocation chunk allowance. A malformed or zero value falls back to the default
/// rather than silently disabling the bound.
pub fn resolve_chunk_budget() -> i64 {
    std::env::var(EMBED_CHUNK_BUDGET_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(EMBED_CHUNK_BUDGET)
}

/// One resource's drain progress: what this call embedded, and what it left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbedProgress {
    /// Chunks embedded by this call.
    pub embedded: u64,
    /// Stale chunks still outstanding for this resource. **Non-zero ⇒ the job is not done** and must
    /// stay queued: completing it here would strand every chunk past the budget.
    pub remaining: u64,
}

impl EmbedProgress {
    /// Nothing stale left — the caller may complete the job.
    pub fn is_complete(&self) -> bool {
        self.remaining == 0
    }
}

/// Embed up to [`EMBED_CHUNK_BUDGET`] stale chunks of ONE resource — the per-job body of the
/// async-embed worker (issue #299), widened to double as the re-embed drain.
///
/// Returns what it did and what remains; the caller completes the job only when
/// [`EmbedProgress::is_complete`]. Idempotent: a fully-fresh resource embeds 0 and reports 0 remaining.
///
/// Chunks are embedded in **one batched `embed_texts` call**, not one call each. The previous
/// per-chunk loop paid the full ORT dispatch overhead 939 times for a large resource; batching is the
/// difference between a drain that finishes inside the function's time budget and one that does not.
pub async fn embed_resource_chunks(
    pool: &PgPool,
    resource_id: Uuid,
    budget: i64,
) -> Result<EmbedProgress> {
    let model = temper_ingest::embed::EXPECTED_MODEL_SHA256;
    // A caller with nothing left to spend must still report what remains, so the job is re-enqueued
    // rather than completed as if it were done.
    if budget <= 0 {
        return Ok(EmbedProgress {
            embedded: 0,
            remaining: count_stale_chunks(pool, resource_id).await?,
        });
    }

    let sql = format!(
        "SELECT ch.id AS chunk_id, cc.content \
         FROM kb_chunks ch \
         JOIN kb_chunk_content cc ON cc.chunk_id = ch.id \
         JOIN kb_content_blocks b ON b.id = ch.block_id \
         WHERE ch.resource_id = $1 AND {STALE_CHUNK_PREDICATE} \
         ORDER BY ch.chunk_index \
         LIMIT $3"
    );
    let rows = sqlx::query(&sql)
        .bind(resource_id)
        .bind(model)
        .bind(budget)
        .fetch_all(pool)
        .await?;

    // An empty/whitespace chunk has nothing to embed but must not be re-selected forever — it would
    // pin `remaining` above zero and the job would never complete. Stamp it with the current model and
    // leave the vector NULL: provenance now says "this build considered it and there was nothing to
    // embed", and because the stale predicate keys on `embedded_with` ALONE (no `embedding IS NULL`
    // disjunct — see STALE_CHUNK_PREDICATE), it genuinely stops matching.
    let mut to_embed: Vec<(Uuid, String)> = Vec::with_capacity(rows.len());
    let mut blank: Vec<Uuid> = Vec::new();
    for row in rows {
        let chunk_id: Uuid = row.get("chunk_id");
        let content: String = row.get("content");
        if content.trim().is_empty() {
            blank.push(chunk_id);
        } else {
            to_embed.push((chunk_id, content));
        }
    }

    if !blank.is_empty() {
        sqlx::query("UPDATE kb_chunks SET embedded_with = $1 WHERE id = ANY($2)")
            .bind(model)
            .bind(&blank)
            .execute(pool)
            .await?;
    }

    let mut embedded = 0u64;
    if !to_embed.is_empty() {
        // ONNX inference is CPU-bound and blocking; run it on the blocking pool via `spawn_blocking`
        // rather than inline on the async executor. Inline, a 64-chunk single-threaded embed pins a
        // tokio worker for its full duration — starving every other task the runtime is driving on
        // that thread (the same reason `warm_embedder` already wraps its embed). The DB reads/writes
        // around it stay async; only the inference moves off-executor, so the strings are handed to
        // the closure by value and the vectors come back for the async UPDATE loop below.
        let (chunk_ids, texts): (Vec<Uuid>, Vec<String>) = to_embed.into_iter().unzip();
        let vectors = tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            temper_ingest::embed::embed_texts(&refs)
        })
        .await
        .map_err(|e| anyhow::anyhow!("embed inference task panicked: {e}"))??;
        if vectors.len() != chunk_ids.len() {
            anyhow::bail!(
                "embed_texts returned {} vectors for {} chunks",
                vectors.len(),
                chunk_ids.len()
            );
        }

        for (chunk_id, emb) in chunk_ids.iter().zip(&vectors) {
            let vec_lit = format!(
                "[{}]",
                emb.iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            // Vector and provenance are written together, always. Writing the vector without the stamp
            // would leave the chunk permanently stale and re-embedded on every tick forever.
            sqlx::query(
                "UPDATE kb_chunks SET embedding = $1::vector, embedded_with = $2 WHERE id = $3",
            )
            .bind(vec_lit)
            .bind(model)
            .bind(chunk_id)
            .execute(pool)
            .await?;
            embedded += 1;
        }
    }

    Ok(EmbedProgress {
        embedded,
        remaining: count_stale_chunks(pool, resource_id).await?,
    })
}

/// Stale chunks outstanding for one resource, under the same predicate the drain embeds by.
pub async fn count_stale_chunks(pool: &PgPool, resource_id: Uuid) -> Result<u64> {
    // Joins kb_chunk_content for the same reason the embed SELECT does — the two MUST select exactly
    // the same rows. If this counted a chunk the embed query cannot see (one with no content row), the
    // job's `remaining` would never reach zero and it would re-enqueue forever, embedding nothing.
    let sql = format!(
        "SELECT count(*) FROM kb_chunks ch \
         JOIN kb_chunk_content cc ON cc.chunk_id = ch.id \
         JOIN kb_content_blocks b ON b.id = ch.block_id \
         WHERE ch.resource_id = $1 AND {STALE_CHUNK_PREDICATE}"
    );
    let n: i64 = sqlx::query_scalar(&sql)
        .bind(resource_id)
        .bind(temper_ingest::embed::EXPECTED_MODEL_SHA256)
        .fetch_one(pool)
        .await?;
    Ok(n as u64)
}
