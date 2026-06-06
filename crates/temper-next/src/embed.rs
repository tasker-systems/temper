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
    let chunks = sqlx::query(
        "SELECT ch.id AS chunk_id, cc.content \
         FROM kb_chunks ch \
         JOIN kb_chunk_content cc ON cc.chunk_id = ch.id \
         JOIN kb_content_blocks b ON b.id = ch.block_id \
         WHERE ch.is_current AND NOT b.is_folded",
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
