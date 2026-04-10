# ONNX Runtime Linking Strategy for temper-ingest

## Decision: Static linking via vendored `libonnxruntime.a`

Static linking produces a single self-contained binary with no runtime
asset-loading overhead. This is the preferred mode for Vercel serverless
functions.

## How It Works

### ort v2 linking modes

The `ort` crate (v2.0.0-rc.12) delegates linking to `ort-sys`, which has
three modes:

| Mode | Feature flag | Mechanism |
|------|-------------|-----------|
| **Prebuilt download** | `download-binaries` (default) | Downloads static `libonnxruntime.a` from pyke CDN at build time |
| **User-supplied static** | (none) | `ORT_LIB_PATH` env var points at a directory containing `libonnxruntime.a` |
| **Runtime dynamic** | `load-dynamic` | No compile-time linking; loads `libonnxruntime.so` at runtime via `ORT_DYLIB_PATH` |

### Chosen configuration

```toml
# crates/temper-ingest/Cargo.toml
ort = { version = "2.0.0-rc.12", default-features = false, features = ["std", "ndarray", "tracing"], optional = true }
```

Key: `default-features = false` disables `download-binaries` and `copy-dylibs`
(both are only useful when the build has outbound network or needs dynamic
libs). The `std`, `ndarray`, and `tracing` features are retained for runtime
functionality.

### Build-time requirements

Set `ORT_LIB_PATH` to a directory containing the platform-appropriate static
library:

```bash
# Local (macOS arm64)
ORT_LIB_PATH=/path/to/ort-libs cargo build -p temper-ingest --features embed

# Vercel build (Linux x86_64)
ORT_LIB_PATH=./crates/temper-ingest/lib/x86_64-unknown-linux-gnu cargo build ...
```

The static lib source is pyke's prebuilt CDN artifacts (same ones
`download-binaries` would fetch). They are ~74 MB per architecture. For
Vercel, the Linux x86_64 variant should be vendored via Git LFS.

### Environment variables

| Variable | Purpose | When needed |
|----------|---------|-------------|
| `ORT_LIB_PATH` | Directory containing `libonnxruntime.a` | Always (build time) |
| `ORT_SKIP_DOWNLOAD` | Prevents ort-sys from attempting CDN download | Optional safeguard |
| `ORT_LIB_LOCATION` | Alias for `ORT_LIB_PATH` | Alternative |

### Verified build output

From the spike branch (commit `c6680ca`):

```
cargo:rustc-link-search=native=/tmp/ort-spike/lib
cargo:rustc-link-lib=static=onnxruntime
```

`otool -L` on the release binary shows no `libonnxruntime` dynamic reference.
Binary size: ~32 MB (temper-cli with embed feature, release profile).

## Task 4 Workflow

When implementing the `include_bytes!` model loading (Task 4), follow this
sequence:

1. Download the `x86_64-unknown-linux-gnu` static lib from pyke's CDN (or
   extract from a local Linux build of ort). Place it under
   `crates/temper-ingest/lib/x86_64-unknown-linux-gnu/libonnxruntime.a`.
2. Track it with Git LFS (`.gitattributes` rule).
3. Set `ORT_LIB_PATH` in `vercel.json` build env to point at the vendored
   lib directory.
4. The `build.rs` already declares `rerun-if-env-changed` for the relevant
   variables.
5. Verify with a Vercel preview deploy (Task 14/20).

## Contingency: load-dynamic

If static linking fails on Vercel (e.g., missing system libraries for the
static link, or binary exceeds Vercel's 250 MB function size limit), the
fallback is:

```toml
ort = { version = "2.0.0-rc.12", default-features = false, features = ["load-dynamic", "std", "ndarray", "tracing"], optional = true }
```

With `load-dynamic`, the binary compiles without any ONNX Runtime library.
At runtime, `include_bytes!` the `libonnxruntime.so` into the binary, write
it to `/tmp/libonnxruntime.so` on cold start, then call
`ort::init_from("/tmp/libonnxruntime.so")` before any session creation.

This adds cold-start latency (~50-100ms for the file write) but avoids all
compile-time linking complexity. The `load-dynamic` feature was verified to
compile in the spike.

## Spike Branch

Branch: `jct/ort-static-linking-spike`
Commit: `c6680ca`
Remote: `origin/jct/ort-static-linking-spike`

Do not merge this branch. It exists as a reference artifact.
