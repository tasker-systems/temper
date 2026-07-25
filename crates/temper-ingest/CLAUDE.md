# temper-ingest

Embedding (ort/ONNX with BAAI/bge-base-en-v1.5, 768-dim) and document extraction
(kreuzberg), both behind feature flags: `embed`, `extract`.

## Both surfaces must embed with the same model — enforced, not assumed

`temper-ingest/build.rs` derives the expected model sha256 from the LFS-pinned
`model_quantized.onnx` **as committed** (from the git-lfs pointer when the blob is
unsmudged — its `oid` *is* the sha256 — from the file when it is), and every model loaded
from disk is verified against it. A mismatch is a hard error.

This exists because it silently went wrong: the CLI's `embed-download` used to fetch the
**fp32** model from Hugging Face `main` while the server used the quantized one, so the
index filled with vectors from two different models with nothing recording which.
`embed-download` no longer downloads anything — it resolves the model from disk next to
the binary (the release archive ships it there, which is why the release checkout needs
`lfs: true`).
