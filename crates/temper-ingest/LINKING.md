# ONNX Runtime Linking Strategy for temper-ingest

## Decision: load-dynamic with bundled `libonnxruntime.so`

The static linking approach (`libonnxruntime.a`) was built against glibc 2.38+ but
Vercel runs Amazon Linux 2 (glibc ~2.26), causing a glibc version mismatch at deploy
time. The fallback strategy documented below is now the active approach.

## How It Works

### ort v2 linking modes

The `ort` crate (v2.0.0-rc.12) delegates linking to `ort-sys`, which has
three modes:

| Mode | Feature flag | Mechanism |
|------|-------------|-----------|
| **Prebuilt download** | `download-binaries` (default) | Downloads static `libonnxruntime.a` from pyke CDN at build time |
| **User-supplied static** | (none) | `ORT_LIB_PATH` env var points at a directory containing `libonnxruntime.a` |
| **Runtime dynamic** | `load-dynamic` | No compile-time linking; loads `libonnxruntime.so` at runtime via `ort::init_from` |

### Chosen configuration

```toml
# crates/temper-ingest/Cargo.toml
ort = { version = "2.0.0-rc.12", default-features = false, features = ["load-dynamic", "std", "ndarray", "tracing", "api-18"], optional = true }
```

Key notes:
- `load-dynamic` disables compile-time linking entirely and enables `ort::init_from`.
- `api-18` is required alongside `load-dynamic` because the VitisAI execution provider
  code is compiled in under `#[cfg(feature = "load-dynamic")]` and accesses
  `SessionOptionsAppendExecutionProvider_VitisAI`, which is gated on `api-18` in
  ort-sys. Without `api-18`, the crate fails to compile (ort v2.0.0-rc.12 bug).
- `default-features = false` prevents `download-binaries` and `copy-dylibs`.

### Runtime initialization

At cold start, `embed.rs` writes the bundled `libonnxruntime.so` to `/tmp` and
calls `ort::init_from` before any session is created:

```rust
static ORT_LIB_BYTES: &[u8] =
    include_bytes!("../lib/x86_64-unknown-linux-gnu/libonnxruntime.so");

fn init_ort_runtime() -> std::result::Result<(), String> {
    // write .so to /tmp on first call, then init_from
}
```

`ort::init_from(path)` returns `Result<EnvironmentBuilder>` and `.commit()` returns
`bool` (true = committed, false = already set by a prior call). Both outcomes are fine.

### Vendored library

- Source: ONNX Runtime v1.24.2 official GitHub release
  (`onnxruntime-linux-x64-1.24.2.tgz`, `lib/libonnxruntime.so.1.24.2`)
- Location: `crates/temper-ingest/lib/x86_64-unknown-linux-gnu/libonnxruntime.so`
- Size: ~21 MB (tracked via Git LFS)
- glibc requirement: glibc 2.17+ (compatible with Amazon Linux 2 / glibc 2.26)

The version 1.24.2 matches the pyke CDN artifacts (`ms@1.24.2`) that ort-sys
v2.0.0-rc.12 uses for its `download-binaries` feature.

### Build-time requirements

No special environment variables needed. The `.so` is bundled via `include_bytes!`
and written to `/tmp` at runtime.

### Environment variables

No `ORT_LIB_PATH` is needed (removed from `vercel.json`). The binary is self-contained
modulo the `/tmp` write on cold start.

### Cold start cost

~50-100ms for the first `std::fs::write` of the 21 MB `.so` to `/tmp`. Subsequent
invocations skip the write (`Path::exists()` check). The `OnceLock` ensures
`init_ort_runtime()` is called at most once per process.

## Spike Branch

Branch: `jct/ort-static-linking-spike`
Commit: `c6680ca`
Remote: `origin/jct/ort-static-linking-spike`

Do not merge this branch. It exists as a reference artifact for the static linking
approach that was superseded.
