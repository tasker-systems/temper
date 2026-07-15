//! Cold-load probe for issue #451 — reproduce the serverless embed hang under a constrained CPU.
//!
//! This mirrors `warm_embedder()` (the `/api/embed/warm` cron path): the FIRST `embed_text` forces
//! the one-time `load_model()` — ORT runtime init → ORT session build → tokenizer load → first
//! inference — which is exactly the cold path that 504s at `maxDuration` on the enterprise deploy.
//! A tracing subscriber is installed so the per-phase cold-load markers wired into `embed.rs`
//! (`embed cold-load: entering …` / `… done`) print, localizing WHICH phase is slow (or hangs)
//! rather than only the aggregate.
//!
//! Run inside the throttled Docker bed (`crates/temper-ingest/bench/`), which pins the CPU to a
//! Vercel-like ~1.5 vCPU. The lib reads `TEMPER_ONNX_INTRA_THREADS` (server default 1) and
//! `TEMPER_ONNX_MODEL_PATH` (the model to load), so a run sweeps threads without a rebuild.
//!
//! ```bash
//! TEMPER_ONNX_INTRA_THREADS=1 \
//! TEMPER_ONNX_MODEL_PATH=crates/temper-ingest/models/bge-base-en-v1.5/model_quantized.onnx \
//!   cargo run --release -p temper-ingest --no-default-features --features embed-download \
//!   --example coldload_probe
//! ```

use std::time::Instant;

use temper_ingest::embed::embed_text;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Match the server's subscriber shape (JSON, info) so the phase markers surface the same way
    // they would in Vercel logs — this probe reads the same signal the deploy would emit.
    tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::INFO)
        .init();

    let threads =
        std::env::var("TEMPER_ONNX_INTRA_THREADS").unwrap_or_else(|_| "<unset → 1>".to_owned());
    let model =
        std::env::var("TEMPER_ONNX_MODEL_PATH").unwrap_or_else(|_| "<compiled-in>".to_owned());
    println!("coldload_probe: intra_threads={threads} model={model}");

    // COLD: the first embed pays the whole model load + first inference. This is the call that
    // hangs on the deploy; the phase markers above will show where the time goes (or stops).
    let cold = Instant::now();
    let v = embed_text("warm")?;
    println!(
        "COLD embed_text(\"warm\"): {:.2}s  (dims={})",
        cold.elapsed().as_secs_f64(),
        v.len()
    );

    // WARM: subsequent embeds reuse the cached session — the cheap steady-state the warm cron is
    // meant to keep the process in. A large gap between COLD and WARM localizes the cost to the load.
    for i in 0..3 {
        let t = Instant::now();
        embed_text("the quick brown fox jumps over the lazy dog")?;
        println!(
            "WARM embed_text #{i}: {:.1}ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }

    Ok(())
}
