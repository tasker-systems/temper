# Throttled embed cold-load bench (issue #451)

A local, reproducible bed for characterizing server-side ONNX embedding under a Vercel-like
constrained CPU. Born to diagnose #451 — server-side embedding 504s at `maxDuration` on the
enterprise deploy, and the platform kill masks *which phase* of the cold model load is to blame —
and kept as a bench bed for tuning the embed path (thread counts, model swaps, graph-optimization
levels) without a deploy cycle.

## What it does

`examples/coldload_probe.rs` mirrors `warm_embedder()` (the `/api/embed/warm` cron path): the first
`embed_text` forces the one-time `load_model()` — **ORT runtime init → ORT session build → tokenizer
load → first inference** — then a few warm embeds. The per-phase markers wired into
`src/embed.rs` (`embed cold-load: entering …` / `… done`) print via a tracing subscriber, so a slow
or hung phase is named rather than hidden behind an aggregate timing.

`run.sh` runs it inside Docker with `--cpus`/`--memory` caps to approximate the deploy's
~1.5 vCPU / 3009 MB function.

## Run

```bash
crates/temper-ingest/bench/run.sh                    # default: cpus=1.5, sweep threads 1 2
CPUS=1.0 THREADS="1 2 4" crates/temper-ingest/bench/run.sh
REBUILD=1 crates/temper-ingest/bench/run.sh          # force image rebuild
```

First run downloads crates and builds `ort` in release (slow); the cargo target is a named Docker
volume, so later runs are cached. Source is bind-mounted read-only, so host edits are picked up with
no image rebuild.

## Fidelity notes

- **arm64, not x86_64.** The bed is arm64-native (fast, clean numbers); the deploy is x86_64. It runs
  the `embed-download` variant with an arm64 ONNX Runtime because the repo's bundled runtime is
  x86_64-only. The `build_session` + inference code under test is **identical** to the server's — only
  model/runtime *acquisition* differs — so this measures the compute mechanism, not the exact silicon.
  Confirm arch-faithfulness with an emulated `--platform linux/amd64` run or a Vercel preview.
- **ONNX Runtime is pinned to the server's version** (see `Dockerfile` `ORT_VERSION`, matched to
  `VERS_*` in the bundled `.so`) so the graph-optimization implementation matches the deploy.
- `TEMPER_ONNX_INTRA_THREADS` (server default 1) and `TEMPER_ONNX_MODEL_PATH` are read by the lib, so
  threads and model can be swept without a rebuild.
