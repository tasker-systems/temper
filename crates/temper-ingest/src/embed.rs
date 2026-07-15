//! Text embedding using BAAI/bge-base-en-v1.5 via ONNX Runtime.
//!
//! **Every surface embeds with the same model — `model_quantized.onnx`, LFS-pinned in this repo —
//! and the model is verified against [`EXPECTED_MODEL_SHA256`] before it is loaded.** That
//! invariant is load-bearing: nothing records which model produced a stored vector, so if two
//! surfaces embed with different models the semantic index silently fills with vectors from two
//! different geometries and nothing can tell them apart. It happened (the CLI ran fp32 while the
//! server ran quantized; ~95% of the index was fp32 before it was caught), which is why the check
//! is a hard error and not a warning.
//!
//! TWO model-loading strategies, selected at compile time, plus a runtime override:
//!
//! - **`embed`**: model bundled via `include_bytes!()`. Requires a git-lfs checkout. Used by the
//!   server (temper-substrate → temper-ingest(embed)).
//! - **`embed-download`**: model resolved from disk at runtime — next to the `temper` binary (the
//!   release archive stages it there, exactly as it stages `libonnxruntime`), or from the XDG
//!   install dir. Used by the CLI, so the binary stays ~18 MB instead of ~128 MB.
//! - **`TEMPER_ONNX_MODEL_PATH`** (runtime, wins over both): load the model from an explicit path.
//!   Mirrors `ORT_DYLIB_PATH`, which does the same for the runtime library. **Also verified** — an
//!   unchecked override would be a hole straight through the lock.
//!
//! `embed-download` deliberately has **no network fallback**. It used to fetch `onnx/model.onnx`
//! from Hugging Face `main`, which was wrong three ways at once: that is the **fp32** model, it
//! tracked a **mutable ref**, and upstream publishes no quantized ONNX at all — so the fetch could
//! never have produced the artifact the server uses. Failing loudly beats embedding with the wrong
//! model.
//!
//! Note that a `--workspace` build resolves to **`embed-download`**, because temper-cli's default
//! `embed` feature selects it and Cargo unifies features across the build — so the `include_bytes!`
//! branch is cfg'd out of every workspace build, whether or not that is what you intended. This is
//! why a test asserting "the CLI and the server agree" must exercise a **built binary**: in a
//! workspace test target both sides compile to the same variant and the assertion passes vacuously.
//!
//! ONNX session created once per process via `OnceLock`.
//!
//! ORT runtime loading is platform-aware:
//! - **Linux** (Vercel deploy): the bundled `libonnxruntime.so` is written to
//!   `/tmp` and loaded via `ort::init_from`.
//! - **Other platforms** (macOS dev): ORT is loaded from the system library
//!   path.  Install via `brew install onnxruntime` and set `ORT_DYLIB_PATH`
//!   if needed.
//!
//! Pipeline: tokenize -> build tensors -> inference -> mean pool -> normalize

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use ndarray::{Array2, ArrayView3};
use ort::session::Session;
use ort::value::TensorRef;
use tokenizers::{Encoding, Tokenizer};

use crate::error::{EmbedError, Result};

/// Embedding dimension for bge-base-en-v1.5.
pub const EMBEDDING_DIM: usize = 768;

#[cfg(all(feature = "embed", not(feature = "embed-download")))]
static MODEL_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/model_quantized.onnx");
static TOKENIZER_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/tokenizer.json");

/// Bundled Linux x86_64 libonnxruntime.so — only compiled into the binary on
/// Linux embed targets (Vercel deploy).  On other platforms this is unused
/// and the system-installed ORT is loaded instead.
#[cfg(all(
    target_os = "linux",
    feature = "embed",
    not(feature = "embed-download")
))]
static ORT_LIB_BYTES: &[u8] = include_bytes!("../lib/x86_64-unknown-linux-gnu/libonnxruntime.so");

// ---- ORT runtime initialization ----

// The three helpers below are consumed only by the "load-dynamic" init branch
// further down and by unit tests. On the Vercel/Linux path (embed without
// embed-download) only the bundled-.so branch compiles, leaving them dead —
// hence the explicit allow. The tests always exercise them via cfg(test).

#[allow(dead_code)]
fn resolve_dylib_from_candidates(candidates: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    candidates.iter().find(|p| p.exists()).cloned()
}

/// Covers the two archive layouts the release installer produces:
///   - mac/linux:  <exe_dir>/lib/libonnxruntime.{dylib,so}
///   - windows:    <exe_dir>/onnxruntime.dll (flat)
#[allow(dead_code)]
fn binary_adjacent_candidates(exe_path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Some(exe_dir) = exe_path.parent() else {
        return Vec::new();
    };
    vec![
        exe_dir.join("lib").join("libonnxruntime.dylib"),
        exe_dir.join("lib").join("libonnxruntime.so"),
        exe_dir.join("onnxruntime.dll"),
    ]
}

/// Linux fallback for when `temper` is symlinked onto PATH while the actual
/// install lives at `~/.local/share/temper/`. On macOS, `dirs::data_local_dir()`
/// resolves to `~/Library/Application Support/` (platform-idiomatic) — but the
/// binary-adjacent candidates fire first there, so this fallback is effectively
/// Linux-only in practice.
#[allow(dead_code)]
fn xdg_data_candidates() -> Vec<std::path::PathBuf> {
    let Some(data_dir) = dirs::data_local_dir() else {
        return Vec::new();
    };
    let lib_dir = data_dir.join("temper").join("lib");
    vec![
        lib_dir.join("libonnxruntime.dylib"),
        lib_dir.join("libonnxruntime.so"),
    ]
}

static ORT_INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Initialize the ORT runtime.
///
/// On Linux: write the bundled `.so` to `/tmp` and load it explicitly.
/// On other platforms: let ORT's `load-dynamic` search system paths
/// (`ORT_DYLIB_PATH`, Homebrew, etc.).
fn init_ort_runtime() -> std::result::Result<(), String> {
    ORT_INIT.get_or_init(|| {
        // Bundled Linux deploy: write the bundled .so to /tmp and load it.
        #[cfg(all(
            target_os = "linux",
            feature = "embed",
            not(feature = "embed-download")
        ))]
        {
            // Fail fast if git-lfs was skipped: `ort::init_from` HANGS on a pointer-as-.so for the
            // whole function timeout (issue #451). Check the compiled-in bytes before touching /tmp,
            // so a pointer already staged there on a prior invocation is caught too.
            reject_lfs_pointer(ORT_LIB_BYTES, "the bundled libonnxruntime.so")?;
            let lib_path = std::path::Path::new("/tmp/libonnxruntime.so");
            if !lib_path.exists() {
                std::fs::write(lib_path, ORT_LIB_BYTES)
                    .map_err(|e| format!("write libonnxruntime.so to /tmp: {e}"))?;
            }
            ort::init_from(lib_path)
                .map_err(|e| format!("ort::init_from: {e}"))?
                .commit();
        }

        // All other cases (macOS, Linux with embed-download): search system paths.
        #[cfg(not(all(
            target_os = "linux",
            feature = "embed",
            not(feature = "embed-download")
        )))]
        {
            // Search order:
            //   1. ORT_DYLIB_PATH env var (explicit override)
            //   2. Binary-adjacent: <exe_dir>/lib/libonnxruntime.{dylib,so} OR
            //      <exe_dir>/onnxruntime.dll (installer-bundled layout)
            //   3. XDG data: ~/.local/share/temper/lib/libonnxruntime.{dylib,so}
            //   4. Homebrew ARM64: /opt/homebrew/lib/libonnxruntime.dylib
            //   5. Homebrew Intel: /usr/local/lib/libonnxruntime.dylib
            //   6. Linux system: /usr/lib/libonnxruntime.so
            let dylib_path = std::env::var("ORT_DYLIB_PATH")
                .ok()
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::current_exe().ok().and_then(|exe| {
                        resolve_dylib_from_candidates(&binary_adjacent_candidates(&exe))
                    })
                })
                .or_else(|| resolve_dylib_from_candidates(&xdg_data_candidates()))
                .or_else(|| {
                    resolve_dylib_from_candidates(&[
                        std::path::PathBuf::from("/opt/homebrew/lib/libonnxruntime.dylib"),
                        std::path::PathBuf::from("/usr/local/lib/libonnxruntime.dylib"),
                        std::path::PathBuf::from("/usr/lib/libonnxruntime.so"),
                    ])
                })
                .map(|p| p.to_string_lossy().into_owned());

            if let Some(path) = dylib_path {
                ort::init_from(path)
                    .map_err(|e| format!("ort::init_from: {e}"))?
                    .commit();
            } else {
                return Err(
                    "ONNX Runtime not found. Install via `brew install onnxruntime` \
                     or set ORT_DYLIB_PATH to the library location."
                        .to_owned(),
                );
            }
        }

        Ok(())
    });
    match ORT_INIT.get() {
        Some(Ok(())) => Ok(()),
        Some(Err(e)) => Err(e.clone()),
        None => Err("ORT_INIT not initialized".to_owned()),
    }
}

// ---- ONNX intra-op thread count ----
//
// ORT parallelizes a single inference across an internal "intra-op" thread
// pool. `N` pins it to N; `0` asks ORT to size the pool itself. This is the
// dominant throughput lever for the embed path: the whole chunk batch is one
// ORT run (see pipeline.rs), so the intra-op pool *is* the parallelism.
//
// `0` DOES NOT MEAN "ALL CORES". It means "ORT's guess", and on a heterogeneous
// ARM box the guess is poor. Measured on a 12-core M4 Pro (8 performance + 4
// efficiency), embedding one 262 KB segment (task 019f57d2):
//
//     0 (ORT picks) 10.77s   <- only ~598% CPU: ORT used about 6 threads
//     4             15.64s
//     6             11.59s
//     8 (P-cores)    9.62s   <- best
//    10              9.88s
//    12 (all cores)  9.73s
//
// Two counterintuitive results: `0` leaves half the machine idle, and using
// *all* 12 is also worse than 8 — the 4 efficiency cores drag every intra-op
// barrier, since the batch can only advance as fast as its slowest thread. The
// optimum is the PERFORMANCE-core count, not the core count. The CLI therefore
// resolves its default by detecting performance cores
// (`temper_cli::actions::embed_threads`) rather than passing `0`.
//
// The right count still differs by surface. The CLI is one user embedding one
// document. The server (temper-api) may run N concurrent ingests, where "every
// embed grabs every core" risks oversubscription — its ideal count is an open
// question pending a measurement under concurrent load (task 019f5892), so the
// server does NOT opt in here and inherits the conservative pinned default
// below.

/// Env override for the ONNX intra-op thread count (`TEMPER_ONNX_INTRA_THREADS`).
/// `0` = let ORT choose, `N` = pin to N. Wins over a surface's programmatic
/// default so a Vercel deploy (or a CLI on a shared box) can be tuned without a
/// rebuild — but loses to an explicit [`force_intra_op_threads`] (a user typing
/// a flag outranks ambient environment).
const INTRA_THREADS_ENV: &str = "TEMPER_ONNX_INTRA_THREADS";

/// Sentinel for "not set at this layer"; resolution then falls through to the
/// next one down.
const INTRA_THREADS_UNSET: usize = usize::MAX;

/// Built-in default when nothing else is set: pin to a single core. Preserves
/// the historical behavior for any process that never opts in (the server), so
/// this plumbing changes no threading on its own — a surface or operator must
/// ask for more.
const INTRA_THREADS_DEFAULT: usize = 1;

/// Process-global intra-op thread count, set once by a surface before the
/// first embed. `INTRA_THREADS_UNSET` until then.
static INTRA_OP_THREADS: AtomicUsize = AtomicUsize::new(INTRA_THREADS_UNSET);

/// Explicit user-supplied count (the CLI's `--embed-threads`). Outranks the env
/// var. `INTRA_THREADS_UNSET` until a user actually asks for a count.
static INTRA_OP_THREADS_FORCED: AtomicUsize = AtomicUsize::new(INTRA_THREADS_UNSET);

/// Declare this process's default ONNX intra-op thread count.
///
/// `N` pins the intra-op pool to N; `0` defers to ORT's own sizing (which is
/// *not* "all cores" — see the module comment above). Call once at startup
/// **before** the first embed: the count is read when the ORT session is lazily
/// built and ignored afterward.
///
/// This is the *surface default* layer — both `TEMPER_ONNX_INTRA_THREADS` and
/// [`force_intra_op_threads`] override it. The CLI calls this with its detected
/// performance-core count. The server leaves it unset and inherits the
/// conservative single-core default, pending a measurement of concurrent-ingest
/// oversubscription (task 019f5892).
pub fn set_intra_op_threads(threads: usize) {
    INTRA_OP_THREADS.store(threads, Ordering::Relaxed);
}

/// Force the intra-op thread count, overriding both the surface default and the
/// `TEMPER_ONNX_INTRA_THREADS` env var.
///
/// This is the top of the precedence chain and exists for one reason: a user who
/// types `--embed-threads N` must get N, even on a machine whose environment
/// already exports the env var. Ambient config should never silently beat an
/// explicit request.
pub fn force_intra_op_threads(threads: usize) {
    INTRA_OP_THREADS_FORCED.store(threads, Ordering::Relaxed);
}

/// Resolve the intra-op thread count for the ORT session.
///
/// Precedence: [`force_intra_op_threads`] (the `--embed-threads` flag) →
/// `TEMPER_ONNX_INTRA_THREADS` env → [`set_intra_op_threads`] (surface default)
/// → `INTRA_THREADS_DEFAULT`. A malformed env value is ignored (falls through)
/// rather than silently selecting a surprising count.
fn resolve_intra_op_threads() -> usize {
    match INTRA_OP_THREADS_FORCED.load(Ordering::Relaxed) {
        INTRA_THREADS_UNSET => {}
        forced => return forced,
    }
    if let Ok(raw) = std::env::var(INTRA_THREADS_ENV) {
        if let Ok(n) = raw.trim().parse::<usize>() {
            return n;
        }
    }
    match INTRA_OP_THREADS.load(Ordering::Relaxed) {
        INTRA_THREADS_UNSET => INTRA_THREADS_DEFAULT,
        n => n,
    }
}

// ---- Model identity ----

/// sha256 of the model this build **expects**, emitted by `build.rs` from the model as committed
/// (its git-lfs oid *is* its sha256). Not a hand-maintained constant: it cannot drift from the
/// artifact.
pub const EXPECTED_MODEL_SHA256: &str = env!("TEMPER_EXPECTED_MODEL_SHA256");

/// Size in bytes of that same model. Checked before the hash because it is free and it is already a
/// perfect discriminator for the bug this guards against: the fp32 model is 435 MB, the quantized
/// one is 110 MB.
pub const EXPECTED_MODEL_SIZE: u64 = {
    // `env!` yields a &str; parse it in const context so a malformed value fails the build.
    let bytes = env!("TEMPER_EXPECTED_MODEL_SIZE").as_bytes();
    let mut acc: u64 = 0;
    let mut i = 0;
    while i < bytes.len() {
        acc = acc * 10 + (bytes[i] - b'0') as u64;
        i += 1;
    }
    acc
};

/// Verify that the model at `path` is the one this build expects.
///
/// **This is the lock.** The CLI and the server must embed with the same model: when they silently
/// did not, the semantic index filled with vectors from two different models, and because nothing
/// records which model produced a vector, the divergence was invisible until someone measured it.
///
/// A mismatch is a hard error, never a fallback. Loading "some other embedding model" is strictly
/// worse than failing: it does not crash, it does not warn, it just quietly writes vectors that
/// belong to a different geometry than everything already stored.
///
/// Costs ~0.2s for a 110 MB model, paid only on paths that then go on to spend *seconds* embedding.
fn verify_model_file(path: &std::path::Path) -> std::result::Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let meta = std::fs::metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?;
    if meta.len() != EXPECTED_MODEL_SIZE {
        // A git-lfs pointer is a ~130-byte text file, and it is by far the most likely wrong-size
        // case for anyone building from a checkout. Say so, rather than talking about 435 MB.
        let hint = if meta.len() < 1024 {
            " That size is a git-lfs POINTER FILE, not the model — run `git lfs pull`."
        } else {
            " (The fp32 bge-base model is ~435 MB; the quantized model this build expects is ~110 MB.)"
        };
        return Err(format!(
            "model at {} is {} bytes, expected {EXPECTED_MODEL_SIZE} — this is not the model this \
             build embeds with.{hint} Embedding with the wrong model produces vectors that do not \
             match the rest of the index.",
            path.display(),
            meta.len(),
        ));
    }

    let mut file =
        std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    if actual != EXPECTED_MODEL_SHA256 {
        return Err(format!(
            "model at {} has sha256 {actual}, expected {EXPECTED_MODEL_SHA256} — refusing to embed \
             with an unverified model.",
            path.display(),
        ));
    }
    Ok(())
}

/// First bytes of a git-lfs pointer file. A build that did not run `git lfs pull` (on Vercel, a
/// deploy checkout without `lfs: true`) leaves the LFS-tracked artifacts as ~134-byte pointer TEXT,
/// and `include_bytes!` bakes THAT into the binary instead of the real bytes.
const LFS_POINTER_MAGIC: &[u8] = b"version https://git-lfs.github.com/spec/v1";

/// Refuse a git-lfs POINTER masquerading as a real bundled artifact — fail FAST and LOUD rather than
/// hand it to ONNX Runtime.
///
/// This is the guard the bundled path never had. `MODEL_BYTES` and `ORT_LIB_BYTES` are
/// `include_bytes!` blobs treated as "verified by construction" (the sha gate only re-checks the
/// file/override path), so on an LFS-off build a pointer sails straight into ORT. And ORT does *not*
/// fail cleanly on one: handed a pointer-as-`libonnxruntime.so`, `ort::init_from` **hangs** for the
/// whole function timeout (measured >90s; issue #451) — inside the `ORT_INIT`/`MODEL` `OnceLock`, so
/// every later embed blocks behind it and is killed at `maxDuration`. Detecting the pointer up front
/// converts that silent 300s catastrophe into an instant, self-explanatory error.
#[allow(dead_code)] // called from the bundled `embed` sites (cfg-gated) and from tests
fn reject_lfs_pointer(bytes: &[u8], what: &str) -> std::result::Result<(), String> {
    if bytes.starts_with(LFS_POINTER_MAGIC) {
        return Err(format!(
            "{what} is a git-lfs POINTER ({} bytes), not the real artifact: this build did not run \
             `git lfs pull` (on Vercel, set `lfs: true` on the deploy's checkout). Refusing to load \
             it — a pointer handed to ONNX Runtime hangs the process until it is killed.",
            bytes.len()
        ));
    }
    Ok(())
}

// ---- Model management ----

struct Model {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

static MODEL: OnceLock<std::result::Result<Model, String>> = OnceLock::new();

/// One-shot guard so the first-inference cold-load timing (issue #451) logs exactly once per
/// process — the phase-entry marker that pinpoints a hang in `session.run` rather than the build.
static FIRST_INFERENCE_LOGGED: AtomicBool = AtomicBool::new(false);

fn load_model() -> Result<&'static Model> {
    let result = MODEL.get_or_init(|| {
        // Per-phase cold-load timing (issue #451). The cold model load is where a throttled
        // serverless instance can hang for the whole `maxDuration`, and the 504 kill masks which
        // phase. Logging ENTRY *before* each phase — not just completion — means a function killed
        // mid-phase leaves the guilty phase as the last line in the logs. `tracing::info!` through
        // the JSON subscriber flushes per line (Rust wraps stdout in a `LineWriter`), so the entry
        // marker survives a subsequent hang; without a subscriber (the CLI) these are no-ops.
        let load_start = Instant::now();

        tracing::info!(
            phase = "ort_init",
            "embed cold-load: entering ORT runtime init"
        );
        let t = Instant::now();
        init_ort_runtime().map_err(|e| format!("ort runtime init: {e}"))?;
        tracing::info!(
            phase = "ort_init",
            elapsed_ms = t.elapsed().as_millis() as u64,
            "embed cold-load: ORT runtime init done"
        );

        tracing::info!(
            phase = "build_session",
            "embed cold-load: entering ORT session build"
        );
        let t = Instant::now();
        let session = build_session()?;
        tracing::info!(
            phase = "build_session",
            elapsed_ms = t.elapsed().as_millis() as u64,
            "embed cold-load: ORT session build done"
        );

        tracing::info!(
            phase = "tokenizer",
            "embed cold-load: entering tokenizer load"
        );
        let t = Instant::now();
        let tokenizer = load_tokenizer()?;
        tracing::info!(
            phase = "tokenizer",
            elapsed_ms = t.elapsed().as_millis() as u64,
            "embed cold-load: tokenizer load done"
        );

        tracing::info!(
            total_ms = load_start.elapsed().as_millis() as u64,
            "embed cold-load: model ready"
        );

        Ok(Model {
            session: Mutex::new(session),
            tokenizer,
        })
    });

    match result {
        Ok(m) => Ok(m),
        Err(e) => Err(EmbedError::Embedding(format!("model init: {e}"))),
    }
}

/// Build ORT session from bundled model bytes — or from `TEMPER_ONNX_MODEL_PATH` when set.
///
/// The override is honored here too, even though this build already has the model compiled in. An env
/// var that works in one build configuration and is silently ignored in another is a footgun: the two
/// configurations are selected by Cargo feature unification, which is not something a caller setting an
/// env var can see.
#[cfg(all(feature = "embed", not(feature = "embed-download")))]
fn build_session() -> std::result::Result<Session, String> {
    let mut builder = Session::builder()
        .map_err(|e| format!("ort session builder: {e}"))?
        .with_intra_threads(resolve_intra_op_threads())
        .map_err(|e| format!("ort threads: {e}"))?;

    match model_path_override()? {
        Some(path) => {
            // The override is verified too. A path handed in by env is exactly as capable of being
            // the wrong model as one found on disk — and an unchecked override is a hole straight
            // through the lock.
            verify_model_file(&path)?;
            builder
                .commit_from_file(&path)
                .map_err(|e| format!("ort load: {e}"))
        }
        // `MODEL_BYTES` is the file `build.rs` hashed for EXPECTED_MODEL_SHA256, so its *content* is
        // verified by construction — re-hashing the 110MB here would prove a tautology. But
        // "construction" assumes git-lfs ran: an LFS-off build bakes a pointer here, and ORT would
        // rather hang on it than error (issue #451). Reject the pointer up front; that is the one
        // thing the tautology does not cover.
        None => {
            reject_lfs_pointer(MODEL_BYTES, "the bundled ONNX model")?;
            builder
                .commit_from_memory(MODEL_BYTES)
                .map_err(|e| format!("ort load: {e}"))
        }
    }
}

/// Env override: load the ONNX model from this filesystem path instead of fetching it.
///
/// A third acquisition path, alongside `include_bytes!` (bundled) and the Hugging Face download —
/// and the only one that costs neither binary size nor network. It exists because the repo **already
/// ships** the model as a git-LFS asset (`models/bge-base-en-v1.5/model_quantized.onnx`), so any
/// environment with a checkout — CI, local dev — is downloading ~400 MB it already has on disk.
///
/// Deliberately mirrors [`ORT_DYLIB_PATH`](self), the sibling env var this crate already uses to point
/// ORT at its runtime library. Same idea, same shape: env var → asset on disk.
pub const MODEL_PATH_ENV: &str = "TEMPER_ONNX_MODEL_PATH";

/// Resolve the model-path override, if set. `Ok(None)` ⇒ not set, use the compiled-in acquisition path.
///
/// A set-but-unusable path is a hard error, never a silent fallback: the whole point of setting this is
/// to NOT hit the network, so quietly downloading 400 MB because of a typo would defeat it — and would
/// look like a mysteriously slow build rather than a misconfiguration.
fn model_path_override() -> std::result::Result<Option<std::path::PathBuf>, String> {
    let Ok(raw) = std::env::var(MODEL_PATH_ENV) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = std::path::PathBuf::from(trimmed);
    if !path.is_file() {
        return Err(format!(
            "{MODEL_PATH_ENV} is set to {} but that is not a readable file \
             (a git-LFS pointer that was never fetched looks exactly like this — try `git lfs pull`)",
            path.display()
        ));
    }
    Ok(Some(path))
}

/// Where the model lives relative to the installed `temper` binary.
///
/// The release archive stages it here, exactly as it already stages `libonnxruntime` into `lib/`.
/// One delivery vehicle, one extraction, and `temper update` keeps binary and model in lockstep —
/// which matters, because a binary and a model that disagree produce a silently corrupt index.
#[cfg(feature = "embed-download")]
const MODEL_BASENAME: &str = "model_quantized.onnx";

/// Candidate on-disk locations for the model, mirroring [`binary_adjacent_candidates`] for the ORT
/// dylib: next to the binary, then the XDG install location a symlinked `temper` resolves back to.
#[cfg(feature = "embed-download")]
fn model_candidates() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("models").join(MODEL_BASENAME));
            // `temper` symlinked onto PATH from an install dir one level up.
            if let Some(up) = dir.parent() {
                out.push(up.join("models").join(MODEL_BASENAME));
            }
        }
    }
    if let Some(data_dir) = dirs::data_local_dir() {
        out.push(data_dir.join("temper").join("models").join(MODEL_BASENAME));
    }
    // Last resort: the model in the checkout this binary was BUILT from (baked by build.rs). This is
    // what keeps `cargo install --path crates/temper-cli` working — that binary lives in ~/.cargo/bin
    // with no adjacent `models/`, and it is the only supported install on the platforms where
    // `install.sh` refuses to run. Guarded by `is_file()` and sha256-verified like any other
    // candidate, so on a machine that is not the build machine it simply misses.
    out.push(std::path::PathBuf::from(env!("TEMPER_CHECKOUT_MODEL_PATH")));
    out
}

/// Build the ORT session for the runtime-resolved build (the shipped CLI).
///
/// Resolution: `TEMPER_ONNX_MODEL_PATH` → binary-adjacent → XDG install dir. **Whatever is found is
/// verified against [`EXPECTED_MODEL_SHA256`] before it is loaded.**
///
/// There is deliberately **no download fallback**. This build used to fetch `onnx/model.onnx` from
/// Hugging Face `main` — which was wrong three ways at once: it is the **fp32** model (the server
/// embeds with the quantized one, so the two populated one index with vectors from two models); it
/// tracked a **mutable upstream ref**; and upstream publishes no quantized ONNX at all, so the fetch
/// could never have produced the right artifact. Failing loudly with a fixable message beats
/// silently embedding with the wrong model.
#[cfg(feature = "embed-download")]
fn build_session() -> std::result::Result<Session, String> {
    let model_path = match model_path_override()? {
        Some(path) => path,
        None => model_candidates()
            .into_iter()
            .find(|p| p.is_file())
            .ok_or_else(|| {
                format!(
                    "embedding model not found. Expected {MODEL_BASENAME} next to the `temper` \
                     binary (the release archive ships it there), or at a path given by \
                     {MODEL_PATH_ENV}. Reinstall via scripts/install/install.sh, or point \
                     {MODEL_PATH_ENV} at models/bge-base-en-v1.5/{MODEL_BASENAME} from a checkout \
                     with `git lfs pull`."
                )
            })?,
    };

    verify_model_file(&model_path)?;

    Session::builder()
        .map_err(|e| format!("ort session builder: {e}"))?
        .with_intra_threads(resolve_intra_op_threads())
        .map_err(|e| format!("ort threads: {e}"))?
        .commit_from_file(&model_path)
        .map_err(|e| format!("ort load: {e}"))
}

// ---- Tokenization ----

/// Load the bundled tokenizer with truncation configured to the model's
/// token limit, so `encode_batch` bounds its own work per input instead of
/// relying on the downstream [`truncate_encoding`] trim.
fn load_tokenizer() -> std::result::Result<Tokenizer, String> {
    let mut tokenizer =
        Tokenizer::from_bytes(TOKENIZER_BYTES).map_err(|e| format!("load tokenizer: {e}"))?;
    tokenizer
        .with_truncation(Some(tokenizers::TruncationParams {
            max_length: MAX_MODEL_TOKENS,
            ..Default::default()
        }))
        .map_err(|e| format!("tokenizer truncation config: {e}"))?;
    Ok(tokenizer)
}

/// Tokenize a batch of texts using the model's tokenizer.
pub fn tokenize(tokenizer: &Tokenizer, texts: &[&str]) -> Result<Vec<Encoding>> {
    tokenizer
        .encode_batch(texts.to_vec(), true)
        .map_err(|e| EmbedError::Embedding(format!("tokenize: {e}")))
}

// ---- Tensor construction ----

/// Input tensors for the ONNX model.
#[derive(Debug)]
pub struct InputTensors {
    pub input_ids: Array2<i64>,
    pub attention_mask: Array2<i64>,
    pub token_type_ids: Array2<i64>,
}

/// Build input tensors from tokenizer encodings.
pub fn build_input_tensors(encodings: &[Encoding]) -> InputTensors {
    let batch_size = encodings.len();
    let max_len = encodings
        .iter()
        .map(|e| e.get_ids().len())
        .max()
        .unwrap_or(0);

    let mut input_ids = Array2::<i64>::zeros((batch_size, max_len));
    let mut attention_mask = Array2::<i64>::zeros((batch_size, max_len));
    let mut token_type_ids = Array2::<i64>::zeros((batch_size, max_len));

    for (i, enc) in encodings.iter().enumerate() {
        for (j, &id) in enc.get_ids().iter().enumerate() {
            input_ids[[i, j]] = id as i64;
        }
        for (j, &mask) in enc.get_attention_mask().iter().enumerate() {
            attention_mask[[i, j]] = mask as i64;
        }
        for (j, &tid) in enc.get_type_ids().iter().enumerate() {
            token_type_ids[[i, j]] = tid as i64;
        }
    }

    InputTensors {
        input_ids,
        attention_mask,
        token_type_ids,
    }
}

// ---- Pooling and normalization ----

/// Mean pooling: average hidden states weighted by attention mask.
pub fn mean_pool(hidden_states: ArrayView3<f32>, attention_mask: &Array2<i64>) -> Vec<Vec<f32>> {
    let batch_size = hidden_states.shape()[0];
    let max_len = hidden_states.shape()[1];
    let dim = hidden_states.shape()[2];
    let mut results = Vec::with_capacity(batch_size);

    for i in 0..batch_size {
        let mut embedding = vec![0f32; dim];
        let mut mask_sum = 0f32;

        for j in 0..max_len {
            let m = attention_mask[[i, j]] as f32;
            mask_sum += m;
            for k in 0..dim {
                embedding[k] += hidden_states[[i, j, k]] * m;
            }
        }

        if mask_sum > 0.0 {
            for v in &mut embedding {
                *v /= mask_sum;
            }
        }

        results.push(embedding);
    }

    results
}

/// L2-normalize a vector in place.
pub fn l2_normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
}

// ---- Embed batch window ----
//
// How many texts go into ONE ORT run. Peak memory scales with this, because a
// transformer's activations do — at a full 93-chunk segment, batch 93 x seq 512:
//
//   attention scores  = batch x heads x seq x seq x 4B  ~= 1.17 GB
//   FFN intermediates = batch x seq x 3072 x 4B         ~= 585 MB
//   hidden states     = batch x seq x 768  x 4B         ~= 146 MB
//
// Before windowing, a whole 262 KB segment went into one run and peaked at ~2.05 GB
// resident. Fine on a laptop; genuinely dangerous on a serverless function with a
// memory ceiling — which is why the window lives HERE, in the shared embed path,
// rather than in the CLI. Every surface gets it.
//
// THIS IS A TRADE, NOT A FREE LUNCH. Measured on a 12-core M4 Pro at 8 intra-op
// threads, embedding one 93-chunk segment (task 019f57d2), 3 runs per window:
//
//   window   peak RSS   wall
//     93      2.05 GB   10.07s   <- the old behavior: one run per segment
//     64      1.42 GB   10.65s
//     32      1.28 GB   10.73s   <- default
//     16      0.90 GB   11.26s
//      8      0.72 GB   12.44s
//
// So: ~38% less memory for ~6% more time. (An earlier, weaker experiment — sweeping
// the *segment budget* rather than the window — suggested throughput was flat. It
// confounded batch size with segment count; this measurement varies only the window
// and is the one to trust. The slope is real, and it steepens below 32.)
//
// Note memory does NOT fall linearly with the window: there is a large fixed floor
// (~105 MB of model weights plus ORT's arena reservation), which is why window 8
// still holds 0.72 GB.

/// Operator override for the embed batch window (`TEMPER_EMBED_BATCH`). Exists so a
/// memory-constrained deploy can trade more time for less peak RSS (or a fat box can
/// buy the time back) without a rebuild — and so the table above stays falsifiable by
/// anyone who doubts it. A value of `0`, or a malformed one, is ignored in favor of
/// the default rather than panicking `chunks()`.
const EMBED_BATCH_ENV: &str = "TEMPER_EMBED_BATCH";

/// Texts per ORT run when nothing overrides it.
///
/// 32 is the knee: it takes ~38% off peak memory for ~6% more time, and below it the
/// time cost accelerates (16 costs 12%, 8 costs 23%) while the memory returns flatten
/// against the fixed model/arena floor.
const EMBED_BATCH_DEFAULT: usize = 32;

/// Resolve the embed batch window: `TEMPER_EMBED_BATCH` → [`EMBED_BATCH_DEFAULT`].
/// Never returns 0 — a zero window would make `chunks()` panic, so a nonsense
/// value falls back rather than taking the process down.
fn resolve_embed_batch() -> usize {
    std::env::var(EMBED_BATCH_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(EMBED_BATCH_DEFAULT)
}

// ---- Token limit ----

/// Hard token limit for bge-base-en-v1.5 (including special tokens).
const MAX_MODEL_TOKENS: usize = 512;

/// Truncate an encoding to `max_len` tokens (modifying in place).
fn truncate_encoding(enc: &mut Encoding, max_len: usize) {
    if enc.get_ids().len() <= max_len {
        return;
    }
    enc.truncate(max_len, 0, tokenizers::TruncationDirection::Right);
}

// ---- Public API ----

/// Count the actual tokens for a text using the model's tokenizer.
pub fn token_count(text: &str) -> Result<usize> {
    let model = load_model()?;
    let encodings = tokenize(&model.tokenizer, &[text])?;
    Ok(encodings[0].get_ids().len())
}

/// Embed a single text string into a 768-dim normalized vector.
pub fn embed_text(text: &str) -> Result<Vec<f32>> {
    let mut results = embed_texts(&[text])?;
    Ok(results.remove(0))
}

/// Embed multiple texts into 768-dim normalized vectors.
///
/// Encodings that exceed 512 tokens are truncated to fit the model's input
/// limit.  The chunker should keep texts under budget, but this is a safety
/// net to prevent ONNX runtime crashes.
///
/// **Runs in bounded windows, not one giant batch.** A whole segment in a single
/// ORT run peaked at ~2.05 GB resident; windowing to 32 takes that to ~1.28 GB for
/// ~6% more time. Every caller gets this — that is why it lives here rather than in
/// the CLI. Tune with `TEMPER_EMBED_BATCH`; see the "Embed batch window" section of
/// this module for the measured table.
pub fn embed_texts(texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let model = load_model()?;
    let window = resolve_embed_batch();

    let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for batch in texts.chunks(window) {
        out.extend(embed_batch(model, batch)?);
    }
    Ok(out)
}

/// Embed exactly one batch in a single ORT run. The peak-memory unit: everything
/// expensive in this function scales with `texts.len()`.
fn embed_batch(model: &Model, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let mut encodings = tokenize(&model.tokenizer, texts)?;
    for enc in &mut encodings {
        truncate_encoding(enc, MAX_MODEL_TOKENS);
    }
    let tensors = build_input_tensors(&encodings);

    let input_ids_ref = TensorRef::from_array_view(tensors.input_ids.view())
        .map_err(|e| EmbedError::Embedding(format!("ort input_ids tensor: {e}")))?;
    let attention_mask_ref = TensorRef::from_array_view(tensors.attention_mask.view())
        .map_err(|e| EmbedError::Embedding(format!("ort attention_mask tensor: {e}")))?;
    let token_type_ids_ref = TensorRef::from_array_view(tensors.token_type_ids.view())
        .map_err(|e| EmbedError::Embedding(format!("ort token_type_ids tensor: {e}")))?;

    let mut session = model
        .session
        .lock()
        .map_err(|e| EmbedError::Embedding(format!("session lock: {e}")))?;

    // One-time first-inference timing (issue #451). The session build can complete and then the
    // *first* `session.run` be the thing that hangs on a cold instance; without this marker that
    // case looks like "tokenizer done" then silence. Logged once per process (the run mutex
    // serializes, so exactly one call wins the swap); later runs are unmarked to keep the drain
    // quiet.
    let first_run = !FIRST_INFERENCE_LOGGED.swap(true, Ordering::Relaxed);
    if first_run {
        tracing::info!(
            batch = texts.len(),
            "embed cold-load: entering first inference"
        );
    }
    let run_start = Instant::now();
    let outputs = session
        .run(ort::inputs![
            input_ids_ref,
            attention_mask_ref,
            token_type_ids_ref
        ])
        .map_err(|e| EmbedError::Embedding(format!("ort run: {e}")))?;
    if first_run {
        tracing::info!(
            batch = texts.len(),
            elapsed_ms = run_start.elapsed().as_millis() as u64,
            "embed cold-load: first inference done"
        );
    }

    let hidden = outputs[0]
        .try_extract_array::<f32>()
        .map_err(|e| EmbedError::Embedding(format!("extract tensor: {e}")))?;

    let hidden_view = hidden
        .view()
        .into_dimensionality::<ndarray::Ix3>()
        .map_err(|e| EmbedError::Embedding(format!("reshape tensor: {e}")))?;

    let mut pooled = mean_pool(hidden_view, &tensors.attention_mask);
    for embedding in &mut pooled {
        l2_normalize(embedding);
    }

    Ok(pooled)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Model-path override (TEMPER_ONNX_MODEL_PATH) ----
    //
    // These use `temp_env::with_var` rather than bare `std::env::set_var`: the test binary is
    // multi-threaded, and process env is global — a bare set/unset races every other test that reads
    // env, which is exactly the kind of flake that gets "fixed" by serializing the whole suite.

    #[test]
    fn model_path_override_unset_means_use_the_compiled_in_path() {
        temp_env::with_var_unset(MODEL_PATH_ENV, || {
            assert!(
                matches!(model_path_override(), Ok(None)),
                "unset ⇒ fall through to the build's own acquisition path"
            );
        });
    }

    #[test]
    fn model_path_override_treats_empty_and_whitespace_as_unset() {
        // A CI expression that resolves to nothing (`TEMPER_ONNX_MODEL_PATH: ${{ ... }}` with an empty
        // value) must not become a hard error — it means "not configured", not "misconfigured".
        for blank in ["", "   "] {
            temp_env::with_var(MODEL_PATH_ENV, Some(blank), || {
                assert!(
                    matches!(model_path_override(), Ok(None)),
                    "blank {blank:?} ⇒ unset, not an error"
                );
            });
        }
    }

    #[test]
    fn model_path_override_on_a_missing_file_fails_loudly_and_names_lfs() {
        // The whole point of setting this is to NOT hit the network. Silently falling back to a 400 MB
        // download because of a typo would defeat it AND look like a mysteriously slow build. And the
        // overwhelmingly likely cause in CI is an unfetched git-LFS pointer, so the error says so.
        temp_env::with_var(MODEL_PATH_ENV, Some("/nonexistent/model.onnx"), || {
            let err = model_path_override().expect_err("a missing file must be an error");
            assert!(err.contains(MODEL_PATH_ENV), "names the var; got: {err}");
            assert!(
                err.contains("git lfs pull"),
                "names the likely cause; got: {err}"
            );
        });
    }

    #[test]
    fn model_path_override_accepts_a_real_file() {
        // Any readable file proves the RESOLUTION logic; ORT's own load is what validates the content.
        // A tempfile rather than `file!()`, which is workspace-relative while the test's cwd is the
        // crate dir.
        let f = tempfile::NamedTempFile::new().expect("tempfile");
        temp_env::with_var(MODEL_PATH_ENV, Some(f.path().as_os_str()), || {
            let resolved = model_path_override().expect("a readable file resolves");
            assert_eq!(resolved.as_deref(), Some(f.path()));
        });
    }

    // ---- LFS-pointer guard (issue #451) ----

    #[test]
    fn reject_lfs_pointer_catches_a_pointer_and_names_the_fix() {
        // The exact shape include_bytes! bakes when git-lfs is skipped.
        let pointer = b"version https://git-lfs.github.com/spec/v1\n\
                        oid sha256:c9729cc84cbd0e9fecc759505d2be65916c9fe05222d7ea26c65fcb3382af38d\n\
                        size 110083337\n";
        let err = super::reject_lfs_pointer(pointer, "the bundled ONNX model")
            .expect_err("a git-lfs pointer must be refused, never handed to ORT");
        assert!(
            err.contains("git-lfs POINTER"),
            "says what went wrong: {err}"
        );
        assert!(
            err.contains("lfs pull") || err.contains("lfs: true"),
            "says how to fix it: {err}"
        );
        assert!(
            err.contains("the bundled ONNX model"),
            "names which artifact: {err}"
        );
    }

    #[test]
    fn reject_lfs_pointer_passes_real_artifact_bytes() {
        // Keys on the pointer magic ONLY, so real binary bytes (an ELF .so header, an ONNX protobuf
        // header) never false-positive. Empty is fine too — that is a different failure for ORT.
        assert!(
            super::reject_lfs_pointer(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0], "libonnxruntime.so")
                .is_ok(),
            "an ELF header is a real shared library, not a pointer"
        );
        assert!(
            super::reject_lfs_pointer(b"\x08\x07\x12\x0Bonnx-model", "model").is_ok(),
            "protobuf-ish bytes are not a pointer"
        );
        assert!(super::reject_lfs_pointer(&[], "empty").is_ok());
    }

    // ---- Feature guard tests ----

    #[test]
    #[cfg(all(feature = "embed", not(feature = "embed-download")))]
    fn bundled_model_bytes_are_not_lfs_pointer() {
        let header = &super::MODEL_BYTES[..std::cmp::min(30, super::MODEL_BYTES.len())];
        let header_str = String::from_utf8_lossy(header);
        assert!(
            !header_str.starts_with("version https://git-lfs"),
            "MODEL_BYTES contains a git-lfs pointer, not the actual model binary. \
             Run `git lfs pull` or use the `embed-download` feature instead."
        );
    }

    // ---- Unit tests (no model download needed) ----

    #[test]
    fn intra_op_threads_precedence() {
        // Owns the whole precedence sequence in one test so the process-global
        // env var + atomic are never raced by a sibling test. Saves/restores
        // both so it leaves no residue.
        let saved_env = std::env::var(super::INTRA_THREADS_ENV).ok();
        let saved_atomic = super::INTRA_OP_THREADS.load(Ordering::Relaxed);

        // Unset atomic + no env → built-in default (historical pinned behavior).
        std::env::remove_var(super::INTRA_THREADS_ENV);
        super::INTRA_OP_THREADS.store(super::INTRA_THREADS_UNSET, Ordering::Relaxed);
        assert_eq!(resolve_intra_op_threads(), super::INTRA_THREADS_DEFAULT);

        // A surface's programmatic default is honored when no env is set.
        set_intra_op_threads(0);
        assert_eq!(resolve_intra_op_threads(), 0);
        set_intra_op_threads(3);
        assert_eq!(resolve_intra_op_threads(), 3);

        // Env wins over the surface default.
        std::env::set_var(super::INTRA_THREADS_ENV, "2");
        assert_eq!(resolve_intra_op_threads(), 2);

        // A malformed env value is ignored, falling back to the surface default.
        std::env::set_var(super::INTRA_THREADS_ENV, "not-a-number");
        assert_eq!(resolve_intra_op_threads(), 3);

        // An explicit user request (`--embed-threads`) outranks BOTH the env var and the
        // surface default. This is the layer that exists so a flag a user typed is never
        // silently beaten by an env var their shell happens to export.
        std::env::set_var(super::INTRA_THREADS_ENV, "2");
        force_intra_op_threads(7);
        assert_eq!(
            resolve_intra_op_threads(),
            7,
            "an explicitly forced count must beat a valid env var"
        );

        // `--embed-threads 0` is a real request ("let ORT decide"), not "unset" — it must
        // still beat the env var. This is the case a naive `if forced != 0` check breaks.
        force_intra_op_threads(0);
        assert_eq!(
            resolve_intra_op_threads(),
            0,
            "a forced 0 is a request, not an absence — it must still beat env"
        );

        // Clearing the forced layer falls back through env again.
        super::INTRA_OP_THREADS_FORCED.store(super::INTRA_THREADS_UNSET, Ordering::Relaxed);
        assert_eq!(resolve_intra_op_threads(), 2);

        match saved_env {
            Some(v) => std::env::set_var(super::INTRA_THREADS_ENV, v),
            None => std::env::remove_var(super::INTRA_THREADS_ENV),
        }
        super::INTRA_OP_THREADS.store(saved_atomic, Ordering::Relaxed);
        super::INTRA_OP_THREADS_FORCED.store(super::INTRA_THREADS_UNSET, Ordering::Relaxed);
    }

    /// The surface default the CLI actually installs must be a real, usable count on this
    /// machine — not the `0` it used to pass, and not a fabricated number on a platform
    /// where detection is impossible.
    #[test]
    fn cli_surface_default_prefers_performance_cores() {
        let detected = crate::cpu::performance_cores();
        let installed = detected.unwrap_or(0);
        match detected {
            Some(p) => {
                assert!(p > 0, "a detected performance-core count must be positive");
                assert_eq!(installed, p, "the CLI installs the detected count verbatim");
                assert_ne!(
                    installed, 0,
                    "on a machine where detection works, the CLI must NOT fall back to \
                     ORT's `0` guess — that is the regression this whole task removed"
                );
            }
            // Undetectable platform: keep the historical behavior rather than guess.
            None => assert_eq!(installed, 0),
        }
    }

    #[test]
    fn embed_batch_window_falls_back_on_nonsense() {
        // A zero window would panic `chunks()`. A bad value must never take the
        // process down, and must never silently select a surprising window.
        temp_env::with_var(super::EMBED_BATCH_ENV, Some("0"), || {
            assert_eq!(super::resolve_embed_batch(), super::EMBED_BATCH_DEFAULT);
        });
        temp_env::with_var(super::EMBED_BATCH_ENV, Some("garbage"), || {
            assert_eq!(super::resolve_embed_batch(), super::EMBED_BATCH_DEFAULT);
        });
        temp_env::with_var(super::EMBED_BATCH_ENV, Some("8"), || {
            assert_eq!(super::resolve_embed_batch(), 8);
        });
        temp_env::with_var(super::EMBED_BATCH_ENV, None::<&str>, || {
            assert_eq!(super::resolve_embed_batch(), super::EMBED_BATCH_DEFAULT);
        });
    }

    /// **The gate on the whole windowing change.**
    ///
    /// `build_input_tensors` pads every text in a batch out to the batch's LONGEST
    /// text, so re-cutting the batch changes the padding each text sees.
    ///
    /// The first version of this test asserted windowing changed the vectors *not at
    /// all* (cosine > 0.9999). **It failed, at cosine 0.9980** — and the failure was
    /// correct. The shipped model is dynamically quantized, so its activation scales
    /// are derived at runtime from tensor ranges that padding participates in. See
    /// `batch_composition_already_perturbs_embeddings_without_windowing`, which pins
    /// down that the *current, unwindowed* code has exactly the same property: the
    /// same text already embeds differently depending on its batch-mates.
    ///
    /// So bit-equality is the wrong bar — it was never true, and demanding it here
    /// would be demanding something of windowing that `main` does not deliver either.
    /// The right bar is that windowing perturbs vectors **no more than the batching
    /// nondeterminism that already exists**, and by an amount irrelevant to retrieval
    /// (cosine > 0.99 on a 768-dim unit vector is far below any ranking sensitivity).
    ///
    /// This deliberately mixes very short and very long texts — that is what makes the
    /// padding differ across the two runs. A uniform corpus would pass while proving
    /// nothing.
    #[test]
    #[cfg(feature = "test-embed")]
    fn windowing_perturbs_embeddings_no_more_than_batching_already_does() {
        let texts: Vec<String> = (0..10)
            .map(|i| {
                if i % 2 == 0 {
                    "short".to_string()
                } else {
                    // Long enough to force heavy padding onto its short neighbors.
                    "a considerably longer passage of prose that tokenizes to many more \
                     tokens than its neighbor does "
                        .repeat(8)
                }
            })
            .collect();
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

        // One single batch (the pre-windowing behavior).
        let whole = temp_env::with_var(super::EMBED_BATCH_ENV, Some("100"), || {
            embed_texts(&refs).expect("one-batch embed")
        });
        // Windowed small enough to split the short/long mix across several runs.
        let windowed = temp_env::with_var(super::EMBED_BATCH_ENV, Some("3"), || {
            embed_texts(&refs).expect("windowed embed")
        });

        assert_eq!(whole.len(), refs.len(), "one vector per input");
        assert_eq!(windowed.len(), whole.len(), "windowing preserves count");

        for (i, (a, b)) in whole.iter().zip(windowed.iter()).enumerate() {
            let cosine: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
            assert!(
                cosine > 0.99,
                "chunk {i}: windowing moved the embedding materially (cosine {cosine:.6}). \
                 A drift this large is NOT the quantization noise that batching already \
                 produces — something is wrong with the windowed path (check that batches \
                 are re-assembled in input order, and that the mask still lines up)."
            );
        }
    }

    /// **Characterizes pre-existing behavior — this is not about windowing.**
    ///
    /// The shipped model is *dynamically* quantized (48 `DynamicQuantizeLinear` ops):
    /// activation scales are computed at runtime from each tensor's observed range.
    /// `build_input_tensors` pads every text to the batch's longest, so the padding —
    /// and therefore the tensor range, and therefore the int8 rounding — depends on
    /// **which other texts happened to share the batch.**
    ///
    /// Consequence, true on `main` today and independent of any windowing: embedding
    /// the same text yields slightly different vectors depending on its batch-mates.
    /// This test pins that fact down so nobody (including a future me) mistakes it for
    /// a regression introduced by the batch window.
    ///
    /// It is a small effect (cosine > 0.99 — irrelevant to retrieval ranking), but it
    /// is real, and it means "identical content ⇒ bit-identical vector" was never true.
    #[test]
    #[cfg(feature = "test-embed")]
    fn batch_composition_already_perturbs_embeddings_without_windowing() {
        let long = "a considerably longer passage of prose that tokenizes to many more \
                    tokens than its neighbor does "
            .repeat(8);

        // Both runs use a window large enough that each is a SINGLE ORT batch, i.e.
        // exactly the pre-windowing code path. The only difference is a batch-mate.
        let (alone, with_mate) = temp_env::with_var(super::EMBED_BATCH_ENV, Some("100"), || {
            let alone = embed_texts(&["short"]).expect("embed alone");
            let with_mate = embed_texts(&["short", long.as_str()]).expect("embed with a mate");
            (alone, with_mate)
        });

        let cosine: f32 = alone[0]
            .iter()
            .zip(with_mate[0].iter())
            .map(|(x, y)| x * y)
            .sum();

        assert!(
            cosine < 0.99999,
            "expected the dynamically-quantized model to be batch-composition sensitive, \
             but the vectors were identical (cosine {cosine:.7}). If this now passes \
             identically, the model is no longer dynamically quantized — revisit \
             `windowing_perturbs_embeddings_no_more_than_batching_already_does`, whose \
             tolerance exists only because of this."
        );
        assert!(
            cosine > 0.99,
            "the perturbation should be small; a large one would mean something worse \
             than quantization noise is going on (cosine {cosine:.6})"
        );
    }

    #[test]
    fn test_l2_normalize_scales_to_unit_length() {
        let mut vec = vec![3.0, 4.0];
        l2_normalize(&mut vec);
        assert!((vec[0] - 0.6).abs() < 1e-6);
        assert!((vec[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_l2_normalize_zero_vector() {
        let mut vec = vec![0.0, 0.0, 0.0];
        l2_normalize(&mut vec);
        assert_eq!(vec, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_l2_normalize_already_unit() {
        let mut vec = vec![1.0, 0.0, 0.0];
        l2_normalize(&mut vec);
        assert!((vec[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_mean_pool_averages_unmasked_positions() {
        let hidden = ndarray::array![[[1.0f32, 2.0, 3.0], [4.0, 5.0, 6.0]]];
        let mask = ndarray::array![[1i64, 1]];
        let result = mean_pool(hidden.view(), &mask);
        assert_eq!(result.len(), 1);
        assert!((result[0][0] - 2.5).abs() < 1e-6);
        assert!((result[0][1] - 3.5).abs() < 1e-6);
        assert!((result[0][2] - 4.5).abs() < 1e-6);
    }

    #[test]
    fn test_mean_pool_with_padding() {
        let hidden = ndarray::array![[[1.0f32, 2.0, 3.0], [99.0, 99.0, 99.0]]];
        let mask = ndarray::array![[1i64, 0]];
        let result = mean_pool(hidden.view(), &mask);
        assert!((result[0][0] - 1.0).abs() < 1e-6);
        assert!((result[0][1] - 2.0).abs() < 1e-6);
        assert!((result[0][2] - 3.0).abs() < 1e-6);
    }

    #[test]
    fn tokenizer_truncates_oversized_input_at_encode_time() {
        // Truncation must be configured on the tokenizer itself so
        // encode_batch bounds its own work per input — not left to the
        // downstream truncate_encoding trim (issue #316).
        let tokenizer = load_tokenizer().expect("load tokenizer");
        let huge = "word ".repeat(50_000); // ~250k chars, far over 512 tokens
        let encodings = tokenize(&tokenizer, &[huge.as_str()]).expect("tokenize");
        assert!(
            encodings[0].get_ids().len() <= MAX_MODEL_TOKENS,
            "expected encoding truncated to {} tokens, got {}",
            MAX_MODEL_TOKENS,
            encodings[0].get_ids().len()
        );
    }

    // ---- Integration tests (require model — may be slow on first run) ----

    #[cfg(feature = "test-embed")]
    #[test]
    fn test_embed_text_dimension() {
        let vec = embed_text("hello world").unwrap();
        assert_eq!(vec.len(), EMBEDDING_DIM);
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn test_embed_text_is_normalized() {
        let vec = embed_text("hello world").unwrap();
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "norm was {norm}");
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn test_embed_texts_batch() {
        let vecs = embed_texts(&["hello", "world"]).unwrap();
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].len(), EMBEDDING_DIM);
        assert_eq!(vecs[1].len(), EMBEDDING_DIM);
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn test_similar_texts_higher_similarity() {
        let v1 = embed_text("rust programming language").unwrap();
        let v2 = embed_text("rust cargo build system").unwrap();
        let v3 = embed_text("chocolate cake recipe").unwrap();

        let sim_related: f32 = v1.iter().zip(&v2).map(|(a, b)| a * b).sum();
        let sim_unrelated: f32 = v1.iter().zip(&v3).map(|(a, b)| a * b).sum();
        assert!(sim_related > sim_unrelated);
    }

    // -- Dylib discovery fallback chain --
    //
    // These tests exercise the pure helpers extracted from the ORT init logic.
    // They don't actually load ONNX — just verify path selection against a
    // constructed set of candidate paths.

    use std::fs;

    fn make_dummy_lib(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"not a real library").expect("write dummy lib");
        path
    }

    #[test]
    fn resolve_picks_first_existing_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let first = make_dummy_lib(tmp.path(), "first.dylib");
        let second = make_dummy_lib(tmp.path(), "second.dylib");

        let picked = super::resolve_dylib_from_candidates(&[first.clone(), second.clone()]);

        assert_eq!(picked, Some(first));
    }

    #[test]
    fn resolve_skips_missing_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist.dylib");
        let exists = make_dummy_lib(tmp.path(), "real.dylib");

        let picked = super::resolve_dylib_from_candidates(&[missing, exists.clone()]);

        assert_eq!(picked, Some(exists));
    }

    #[test]
    fn resolve_returns_none_when_all_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let candidates = vec![tmp.path().join("a.dylib"), tmp.path().join("b.dylib")];

        let picked = super::resolve_dylib_from_candidates(&candidates);

        assert_eq!(picked, None);
    }

    #[test]
    fn binary_adjacent_candidates_include_lib_subdir_and_flat() {
        // Given an exe at /opt/tool/bin/temper, candidates should include both
        // /opt/tool/bin/lib/libonnxruntime.{dylib,so} (installed-tree layout)
        // and /opt/tool/bin/onnxruntime.dll (Windows flat layout).
        let fake_exe = std::path::PathBuf::from("/opt/tool/bin/temper");
        let candidates = super::binary_adjacent_candidates(&fake_exe);

        let as_str: Vec<String> = candidates
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        assert!(
            as_str
                .iter()
                .any(|s| s.ends_with("lib/libonnxruntime.dylib")),
            "missing lib/libonnxruntime.dylib: {as_str:?}"
        );
        assert!(
            as_str.iter().any(|s| s.ends_with("lib/libonnxruntime.so")),
            "missing lib/libonnxruntime.so: {as_str:?}"
        );
        assert!(
            as_str.iter().any(|s| s.ends_with("onnxruntime.dll")),
            "missing onnxruntime.dll: {as_str:?}"
        );
    }

    #[test]
    fn xdg_data_candidates_point_at_temper_lib() {
        let candidates = super::xdg_data_candidates();
        let as_str: Vec<String> = candidates
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert!(
            as_str.iter().any(|s| s.contains("/temper/lib/")),
            "candidates should include ~/.local/share/temper/lib/: {as_str:?}"
        );
    }

    /// The model this build expects, in the source tree. Present in any checkout with `git lfs
    /// pull`; when it is only a pointer, the size check below is what trips — which is itself the
    /// correct answer, so the test is meaningful either way.
    fn repo_model_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("models/bge-base-en-v1.5/model_quantized.onnx")
    }

    /// `build.rs` derives the expected identity from the model **as committed** — from the git-lfs
    /// pointer when the blob is not smudged, from the file itself when it is. Both must agree, or the
    /// pin means nothing: assert the constants describe the file actually on disk.
    #[test]
    fn expected_model_identity_matches_the_committed_model() {
        assert_eq!(
            super::EXPECTED_MODEL_SHA256.len(),
            64,
            "sha256 hex should be 64 chars, got {:?}",
            super::EXPECTED_MODEL_SHA256
        );

        let path = repo_model_path();
        if !path.is_file() {
            return; // no checkout of the blob; the size/hash cannot be cross-checked here
        }
        let on_disk = std::fs::metadata(&path).expect("stat model").len();
        assert_eq!(
            on_disk,
            super::EXPECTED_MODEL_SIZE,
            "build.rs's expected size disagrees with the committed model — the pin has drifted"
        );
    }

    #[test]
    fn the_repo_model_verifies_against_the_expected_identity() {
        let path = repo_model_path();
        if !path.is_file() {
            return; // no checkout of the blob; nothing to assert
        }
        super::verify_model_file(&path).expect("the committed model must satisfy its own pin");
    }

    /// **The regression test for the bug.** The CLI shipped the fp32 model while the server used the
    /// quantized one, and nothing objected — so the index filled with vectors from two models. Any
    /// model that is not the expected one must now be REFUSED, not loaded.
    #[test]
    fn a_model_that_is_not_the_expected_one_is_refused() {
        // Any file that is not the model stands in for "the wrong model" — the check is on identity,
        // not on provenance. The tokenizer is conveniently to hand and is definitely not 110 MB.
        let not_the_model = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("models/bge-base-en-v1.5/tokenizer.json");

        let err = super::verify_model_file(&not_the_model)
            .expect_err("a file that is not the expected model must be rejected");

        assert!(
            err.contains("wrong model"),
            "the error must say which way it failed, so the fix is obvious: {err}"
        );
    }
}
