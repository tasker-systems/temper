//! Recover the lost provenance of stored embeddings: which model produced each vector?
//!
//! Nothing in `kb_chunks` records the model that produced an embedding. That gap is what let the
//! CLI (fp32) and the server (quantized) silently populate the same index with vectors from two
//! different models. This tool recovers the answer after the fact.
//!
//! **How it works.** Re-embed a chunk's *stored content* with a known model and cosine it against
//! its *stored vector*. If the same model produced it, the cosine is ~1.0. If the other model did,
//! the cosine sits at the cross-model similarity (~0.991 for quantized-vs-fp32 on bge-base). The
//! two outcomes are cleanly separated, so a single pass with ONE model classifies every chunk —
//! there is no need to embed with both.
//!
//! Input is the JSON produced by:
//!
//! ```sql
//! select json_agg(t) from (
//!   select c.id::text as id, cc.content, c.embedding::text as emb,
//!          coalesce(r.origin_uri,'') as origin
//!   from kb_chunks c
//!   join kb_chunk_content cc on cc.chunk_id = c.id
//!   join kb_resources r on r.id = c.resource_id
//!   where c.is_current and c.embedding is not null
//!   order by random() limit 80
//! ) t;
//! ```
//!
//! ```bash
//! TEMPER_ONNX_MODEL_PATH=crates/temper-ingest/models/bge-base-en-v1.5/model_quantized.onnx \
//!   cargo run --release -p temper-ingest --features embed --example classify_chunks -- sample.json
//! ```

use serde::Deserialize;
use temper_ingest::embed::embed_texts;

/// The `--match-threshold`: above this, the loaded model is judged to be the one that produced the
/// stored vector. Sits well above the measured cross-model similarity (~0.991) and well below the
/// same-model floor (~1.0), so the classification is not sensitive to where exactly it is placed.
const MATCH_THRESHOLD: f64 = 0.9995;

#[derive(Deserialize)]
struct Row {
    id: String,
    content: String,
    emb: String,
    origin: String,
}

/// pgvector renders as `[0.1,0.2,...]`.
fn parse_pgvector(raw: &str) -> Result<Vec<f32>, String> {
    raw.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().parse::<f32>().map_err(|e| e.to_string()))
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let na: f64 = a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: classify_chunks <sample.json>")?;
    let model = std::env::var("TEMPER_ONNX_MODEL_PATH")
        .unwrap_or_else(|_| "<compiled-in / HF download>".to_owned());

    let rows: Vec<Row> = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    let texts: Vec<&str> = rows.iter().map(|r| r.content.as_str()).collect();

    println!("reference model: {model}");
    println!("chunks: {}\n", rows.len());

    let recomputed = embed_texts(&texts)?;

    let mut matched = 0usize;
    let mut mismatched = 0usize;
    let mut mismatch_sims: Vec<f64> = Vec::new();
    let mut match_sims: Vec<f64> = Vec::new();
    let mut mismatched_mcp = 0usize;

    for (row, fresh) in rows.iter().zip(&recomputed) {
        let stored = parse_pgvector(&row.emb)?;
        let sim = cosine(&stored, fresh);
        if sim >= MATCH_THRESHOLD {
            matched += 1;
            match_sims.push(sim);
        } else {
            mismatched += 1;
            mismatch_sims.push(sim);
            if row.origin.starts_with("mcp://") {
                mismatched_mcp += 1;
            }
            if mismatch_sims.len() <= 3 {
                println!("  mismatch example: chunk {} cos={sim:.6}", row.id);
            }
        }
    }

    let pct = |n: usize| 100.0 * n as f64 / rows.len() as f64;
    let mean = |v: &[f64]| {
        if v.is_empty() {
            f64::NAN
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };

    println!("\n── provenance of stored vectors ──");
    println!(
        "  produced by THIS model : {matched:3} ({:.1}%)  mean cos {:.6}",
        pct(matched),
        mean(&match_sims)
    );
    println!(
        "  produced by ANOTHER    : {mismatched:3} ({:.1}%)  mean cos {:.6}",
        pct(mismatched),
        mean(&mismatch_sims)
    );
    println!("  (of the mismatches, {mismatched_mcp} came from an mcp:// resource)");
    Ok(())
}
