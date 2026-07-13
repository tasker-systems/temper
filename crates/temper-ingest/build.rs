//! Emit the **expected sha256 of the embedding model** as a compile-time constant.
//!
//! This is the pin. The CLI and the server must embed with the *same* model — when they did not,
//! the CLI silently shipped fp32 while the server ran the quantized model, and the semantic index
//! filled with vectors from two models that nothing recorded and nothing could tell apart.
//!
//! The constant is derived from the model **as committed**, so it cannot drift from the artifact:
//!
//! - **git-lfs pointer** (a checkout without `git lfs pull` — e.g. CI that does not set `lfs: true`):
//!   the pointer is plain text and *already carries the sha256* as its `oid`. Parse it.
//! - **smudged file** (a normal LFS checkout): it is the real model. Hash it.
//!
//! Both paths yield the same hash, so nobody has to remember to update a hardcoded constant when the
//! model changes — the failure mode that rots.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// The vendored model. LFS-tracked (see `.gitattributes`); its LFS oid **is** its sha256.
const MODEL_REL_PATH: &str = "models/bge-base-en-v1.5/model_quantized.onnx";

/// First line of a git-lfs pointer file. Pointers are small plain text; the model is 110 MB of
/// binary — so this prefix is an unambiguous discriminator.
const LFS_POINTER_MAGIC: &str = "version https://git-lfs.github.com/spec/v1";

fn main() {
    println!("cargo:rerun-if-changed={MODEL_REL_PATH}");
    println!("cargo:rerun-if-changed=build.rs");

    let path = Path::new(MODEL_REL_PATH);
    let (sha, size) = match resolve_model_digest(path) {
        Ok(v) => v,
        Err(e) => {
            // A hard error, not a fallback. A build that cannot state which model it expects cannot
            // verify the one it loads — and an unverifiable model is the whole bug.
            panic!("cannot determine expected model sha256 from {MODEL_REL_PATH}: {e}");
        }
    };

    println!("cargo:rustc-env=TEMPER_EXPECTED_MODEL_SHA256={sha}");
    println!("cargo:rustc-env=TEMPER_EXPECTED_MODEL_SIZE={size}");
}

/// Returns `(sha256_hex, size_bytes)` for the model, whether it is an LFS pointer or the real file.
fn resolve_model_digest(path: &Path) -> Result<(String, u64), String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;

    // Read enough to recognise a pointer without slurping 110 MB of model into memory.
    let mut head = [0u8; 256];
    let n = file.read(&mut head).map_err(|e| format!("read: {e}"))?;
    let head_str = String::from_utf8_lossy(&head[..n]);

    if head_str.starts_with(LFS_POINTER_MAGIC) {
        return parse_lfs_pointer(&head_str);
    }

    // Not a pointer: the real model. Hash it in chunks — a 110 MB read_to_end in a build script is
    // a needless memory spike on every cold build.
    let mut file = std::fs::File::open(path).map_err(|e| format!("reopen: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hex(&hasher.finalize()), size))
}

/// A pointer looks like:
/// ```text
/// version https://git-lfs.github.com/spec/v1
/// oid sha256:c9729cc84cbd…
/// size 110083337
/// ```
fn parse_lfs_pointer(text: &str) -> Result<(String, u64), String> {
    let mut oid = None;
    let mut size = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("oid sha256:") {
            oid = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("size ") {
            size = rest.trim().parse::<u64>().ok();
        }
    }
    match (oid, size) {
        (Some(o), Some(s)) => Ok((o, s)),
        _ => Err("lfs pointer missing oid/size".to_owned()),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}
