//! Build script for temper-ingest.
//!
//! When the `embed` feature is enabled, `ort` (via `ort-sys`) links ONNX
//! Runtime. By default `ort-sys` would download a prebuilt static
//! `libonnxruntime.a` from pyke's CDN, but the Vercel Rust function build
//! sandbox has no outbound HTTPS at compile time, so we must point `ort-sys`
//! at a vendored copy of the static lib via `ORT_LIB_PATH`.
//!
//! This build script just declares reruns on the env vars that influence the
//! decision. The `ort-sys` build script also declares them, but stating them
//! here gives a stable rerun contract regardless of upstream changes and makes
//! the dependency explicit.
//!
//! See `crates/temper-ingest/LINKING.md` for the full rationale.

fn main() {
    println!("cargo:rerun-if-env-changed=ORT_LIB_PATH");
    println!("cargo:rerun-if-env-changed=ORT_LIB_LOCATION");
    println!("cargo:rerun-if-env-changed=ORT_SKIP_DOWNLOAD");
}
