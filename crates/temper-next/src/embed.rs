use anyhow::Result;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Job A (spec §6a): chunk + embed every non-folded block's content, write kb_chunks rows with
/// 768-dim embeddings. Block content source: Plan 3 authors block text; this reads it from a
/// `block_text(block_id, body)` source table seeded by Plan 3 T1 (reconcile before running — the
/// table does not exist when Plan 2 lands, so `tests/embed_job.rs` stays `#[ignore]`d until then).
pub async fn embed_all_blocks(pool: &PgPool) -> Result<()> {
    let blocks = sqlx::query(
        "SELECT b.id AS block_id, b.resource_id, bt.body \
         FROM kb_content_blocks b JOIN block_text bt ON bt.block_id = b.id \
         WHERE NOT b.is_folded",
    )
    .fetch_all(pool)
    .await?;
    for row in blocks {
        let block_id: Uuid = row.get("block_id");
        let resource_id: Uuid = row.get("resource_id");
        let body: String = row.get("body");
        // submodule call paths (no re-exports in temper-ingest lib.rs).
        let chunks = temper_ingest::chunk::chunk_markdown(&body);
        let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        if texts.is_empty() {
            continue;
        }
        let embeddings = temper_ingest::embed::embed_texts(&texts)?; // 768-dim, l2-normalized
        for (i, emb) in embeddings.iter().enumerate() {
            let vec_lit = format!(
                "[{}]",
                emb.iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            sqlx::query(
                "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, embedding, is_current) \
                 VALUES ($1,$2,$3,$4,$5::vector,true) \
                 ON CONFLICT (block_id, chunk_index, version) DO UPDATE SET embedding = EXCLUDED.embedding",
            )
            .bind(block_id)
            .bind(resource_id)
            .bind(i as i32)
            // ChunkData carries a real SHA-256 content_hash; use it (1:1 with embeddings).
            .bind(chunks[i].content_hash.clone())
            .bind(vec_lit)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}
