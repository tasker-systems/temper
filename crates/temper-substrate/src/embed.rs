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

/// A chunk is **stale** when it has no vector, or when the vector it has was produced by a model that
/// is not the one this build embeds with.
///
/// This single predicate does two jobs that used to need two mechanisms:
///
/// - the original async-embed backfill (`embedding IS NULL` — a deferred chunk awaiting its vector), and
/// - **re-embedding after a model change**, which previously had no mechanism at all.
///
/// It is why the re-embed needs no backfill script: rows written before `embedded_with` existed carry
/// NULL, so they are stale *by definition* the moment the column ships — and any future model change
/// re-stales the index automatically. Marking dirty is a `NULL`, not a data migration.
///
/// Scoping by `is_current` is what makes create-then-update supersede naturally: a body revise makes
/// the new generation current and the old non-current, so a job — whenever it runs — only ever embeds
/// the resource's *live* chunks.
const STALE_CHUNK_PREDICATE: &str = "ch.is_current \
     AND NOT b.is_folded \
     AND (ch.embedding IS NULL OR ch.embedded_with IS DISTINCT FROM $2)";

/// How many chunks one call will embed before returning, leaving the rest for the next tick.
///
/// The drain runs inside a serverless function with a wall-clock limit, and it must not be possible
/// for one large resource to exceed it: prod's largest holds **939** chunks, and embedding those in a
/// single invocation would time out, retry, time out again, and land the job in `dead` — a resource
/// that can never heal, inside a system that reports itself as healthy. Bounding the work per call and
/// resuming on the next tick makes progress monotonic regardless of resource size.
pub const EMBED_CHUNK_BUDGET: i64 = 64;

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
pub async fn embed_resource_chunks(pool: &PgPool, resource_id: Uuid) -> Result<EmbedProgress> {
    let model = temper_ingest::embed::EXPECTED_MODEL_SHA256;

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
        .bind(EMBED_CHUNK_BUDGET)
        .fetch_all(pool)
        .await?;

    // An empty/whitespace chunk has nothing to embed but must not be re-selected forever — it would
    // pin `remaining` above zero and the job would never complete. Stamp it with the current model and
    // a NULL vector: provenance says "this build considered it", so the predicate stops matching it.
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
        let texts: Vec<&str> = to_embed.iter().map(|(_, c)| c.as_str()).collect();
        let vectors = temper_ingest::embed::embed_texts(&texts)?;
        if vectors.len() != to_embed.len() {
            anyhow::bail!(
                "embed_texts returned {} vectors for {} chunks",
                vectors.len(),
                to_embed.len()
            );
        }

        for ((chunk_id, _), emb) in to_embed.iter().zip(&vectors) {
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
    let sql = format!(
        "SELECT count(*) FROM kb_chunks ch \
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
