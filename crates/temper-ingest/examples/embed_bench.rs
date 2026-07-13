//! Embedding benchmark: per-chunk cost, peak RSS, and cross-model vector equivalence.
//!
//! Built for the accelerator spike (`019f5891`, which rejected CoreML) and kept because it is the
//! acceptance gate for the quantized-model rollout: it is the thing that can prove the CLI's model
//! and the server's model produce the *same vectors*, which is the invariant that silently broke.
//!
//! The ORT session is process-global (`OnceLock`), so two models cannot be compared inside one
//! process. Hence: run it twice, once per model, each run dumping its vectors; the second run
//! diffs against the first.
//!
//! ```bash
//! # the model the SERVER runs (LFS-pinned, quantized)
//! TEMPER_ONNX_MODEL_PATH=crates/temper-ingest/models/bge-base-en-v1.5/model_quantized.onnx \
//!   cargo run --release -p temper-ingest --features embed --example embed_bench -- /tmp/quant.json
//!
//! # the model the CLI currently runs (fp32, fetched from HF `main`)
//! TEMPER_ONNX_MODEL_PATH=<fp32.onnx> \
//!   cargo run --release -p temper-ingest --features embed --example embed_bench -- \
//!   /tmp/fp32.json --baseline /tmp/quant.json
//! ```

use std::time::Instant;

use temper_ingest::chunk::chunk_markdown;
use temper_ingest::embed::{embed_texts, EMBEDDING_DIM};

/// Byte target for the synthetic body. Matches the e2e generator's 1.2 MB so the chunk count and
/// the per-chunk cost land on the same scale as the numbers already recorded in the spike.
const TARGET_BYTES: usize = 1_202_924;

/// Reproduces `generate_large_markdown` from `tests/e2e/tests/streaming_ingest_test.rs`. Copied
/// rather than shared: an example cannot depend on an e2e test target, and pinning the corpus here
/// keeps the benchmark reproducible even if the test's generator changes.
fn generate_large_markdown(target_bytes: usize) -> String {
    const FILLER: &str =
        "The quick brown fox jumps over the lazy dog, padding this section well past the \
         segmentation budget so the streaming ingest pipeline must split the document into \
         multiple blocks via the segmented begin/append/finalize path.\n";
    let mut body = String::from("# Big Document\n\n");
    let mut section = 0usize;
    while body.len() < target_bytes {
        body.push_str(&format!("## Section {section}\n\n"));
        for _ in 0..40 {
            body.push_str(FILLER);
        }
        body.push('\n');
        section += 1;
    }
    body
}

/// Peak resident set size in bytes, via `getrusage`. macOS reports `ru_maxrss` in bytes; Linux
/// reports kilobytes. The OS's number, not an allocation counter — a model swap moves real RSS.
fn peak_rss_bytes() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return 0;
    }
    let raw = usage.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        raw
    } else {
        raw * 1024
    }
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
    let mut args = std::env::args().skip(1);
    let out_path = args
        .next()
        .ok_or("usage: embed_bench <out.json> [--baseline <path>]")?;
    let baseline_path = match (args.next().as_deref(), args.next()) {
        (Some("--baseline"), Some(p)) => Some(p),
        _ => None,
    };

    // Which model is actually loaded is the entire subject of this benchmark, so print it rather
    // than leaving the reader to infer it from Cargo feature unification — the exact inference that
    // let the CLI ship the wrong model unnoticed.
    let model = std::env::var("TEMPER_ONNX_MODEL_PATH")
        .unwrap_or_else(|_| "<compiled-in / HF download>".to_owned());
    let threads =
        std::env::var("TEMPER_ONNX_INTRA_THREADS").unwrap_or_else(|_| "<unset>".to_owned());

    let body = generate_large_markdown(TARGET_BYTES);
    let chunks = chunk_markdown(&body);
    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();

    println!("model={model}");
    println!(
        "intra_threads={threads} body={} bytes chunks={}",
        body.len(),
        chunks.len()
    );

    // Warm the session (model load) so it is not billed to the measured run.
    let warm_start = Instant::now();
    embed_texts(&texts[..1])?;
    let warmup = warm_start.elapsed();

    let start = Instant::now();
    let vectors = embed_texts(&texts)?;
    let elapsed = start.elapsed();

    assert_eq!(vectors.len(), chunks.len(), "one vector per chunk");
    assert!(
        vectors.iter().all(|v| v.len() == EMBEDDING_DIM),
        "every vector is {EMBEDDING_DIM}-dim"
    );

    let per_chunk_ms = elapsed.as_secs_f64() * 1000.0 / chunks.len() as f64;
    println!(
        "session_warmup={:.2}s  embed={:.2}s  per_chunk={per_chunk_ms:.1}ms  peak_rss={:.2}GB",
        warmup.as_secs_f64(),
        elapsed.as_secs_f64(),
        peak_rss_bytes() as f64 / 1e9,
    );

    // The gate. Two models that disagree place the same text in two different neighbourhoods of the
    // index. Report the WORST cosine across all chunks, not the mean: a mean of 0.9999 hides one
    // catastrophically wrong chunk.
    if let Some(baseline_path) = baseline_path {
        let raw = std::fs::read_to_string(&baseline_path)?;
        let baseline: Vec<Vec<f32>> = serde_json::from_str(&raw)?;
        assert_eq!(
            baseline.len(),
            vectors.len(),
            "baseline chunk count differs — not comparable"
        );

        let mut worst = f64::MAX;
        let mut worst_idx = 0usize;
        let mut max_abs_delta = 0.0f64;
        for (i, (a, b)) in baseline.iter().zip(&vectors).enumerate() {
            let sim = cosine(a, b);
            if sim < worst {
                worst = sim;
                worst_idx = i;
            }
            for (x, y) in a.iter().zip(b) {
                max_abs_delta = max_abs_delta.max(f64::from((x - y).abs()));
            }
        }
        println!(
            "equivalence vs {baseline_path}: worst_cosine={worst:.9} (chunk {worst_idx})  \
             max_abs_component_delta={max_abs_delta:.2e}"
        );
    }

    std::fs::write(&out_path, serde_json::to_string(&vectors)?)?;
    println!("wrote {} vectors to {out_path}", vectors.len());
    Ok(())
}
