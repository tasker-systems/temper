//! Root-cause probe for issue #451: does ORT HANG or ERROR when handed a git-LFS pointer?
//!
//! Reproduces the enterprise condition. A Vercel build WITHOUT git-lfs checks out the ~134-byte LFS
//! pointer, not the 110 MB model, and `include_bytes!("…model_quantized.onnx")` bakes that pointer
//! into `MODEL_BYTES`. The bundled server path then hands it straight to `commit_from_memory` — which
//! does NOT re-verify the bytes (the sha gate only guards the file/override path; the compiled-in
//! bytes are "verified by construction", an assumption that git-lfs-off silently breaks).
//!
//! This feeds arbitrary bytes to `commit_from_memory` and times the outcome, to distinguish the two
//! hypotheses: a ~300s HANG (matching the `maxDuration` kills on the deploy) vs. a fast parse ERROR
//! (which would mean the pointer explains a failure but not the hang).
//!
//! ```bash
//! PROBE_BYTES=/repo/crates/temper-ingest/bench/lfs_pointer_sample.txt \
//!   cargo run --release -p temper-ingest --no-default-features --features embed-download \
//!   --example pointer_probe
//! ```

use std::time::Instant;

use ort::session::Session;

fn main() {
    let path =
        std::env::var("PROBE_BYTES").expect("set PROBE_BYTES=<file whose bytes to feed to ORT>");

    // dylib mode: reproduce the enterprise's FIRST failure — init the ORT runtime from a pointer
    // masquerading as libonnxruntime.so (both the .so and the model are LFS-tracked, so an
    // LFS-off build bakes a pointer for BOTH). This is what runs before the model is ever loaded.
    if std::env::var("PROBE_MODE").as_deref() == Ok("dylib") {
        println!("pointer_probe[dylib]: ort::init_from({path}) — a pointer-as-.so");
        let t = Instant::now();
        let result = ort::init_from(&path).map(|b| b.commit());
        let elapsed = t.elapsed().as_secs_f64();
        match result {
            Ok(_) => println!("RESULT: Ok (loaded as a shared library!?) in {elapsed:.2}s"),
            Err(e) => println!("RESULT: Err in {elapsed:.2}s — {e}"),
        }
        return;
    }

    let dylib = std::env::var("ORT_DYLIB_PATH").expect("set ORT_DYLIB_PATH (the bench bed does)");
    let committed = ort::init_from(dylib).expect("ort init_from").commit();
    assert!(committed, "ort runtime failed to commit");
    let bytes = std::fs::read(&path).expect("read PROBE_BYTES");
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(48)]).replace('\n', " ");
    println!(
        "pointer_probe: feeding {} bytes to commit_from_memory (head: {head:?})",
        bytes.len()
    );

    let t = Instant::now();
    let result = Session::builder()
        .expect("session builder")
        .commit_from_memory(&bytes);
    let elapsed = t.elapsed().as_secs_f64();
    match result {
        Ok(_) => println!("RESULT: Ok — parsed as ONNX in {elapsed:.2}s"),
        Err(e) => println!("RESULT: Err in {elapsed:.2}s — {e}"),
    }
}
