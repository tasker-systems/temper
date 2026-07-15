#!/usr/bin/env bash
# Throttled cold-load bench for issue #451.
#
# Builds the bed image once, then runs examples/coldload_probe.rs under a Vercel-like CPU/memory
# cap, sweeping ONNX intra-op thread counts. The probe forces the one-time model load (the cold
# path that 504s on the deploy) and the per-phase markers in embed.rs localize where the time goes.
#
# Usage:
#   crates/temper-ingest/bench/run.sh                 # default sweep: cpus=1.5, threads 1 2
#   CPUS=1.0 THREADS="1 2 4" crates/temper-ingest/bench/run.sh
#   REBUILD=1 crates/temper-ingest/bench/run.sh        # force image rebuild
#
# Env knobs:
#   CPUS     vCPU cap (default 1.5 — Vercel gives ~2048 MB/vCPU, the deploy is 3009 MB ≈ 1.47)
#   MEMORY   memory cap (default 3009m — matches vercel.json api/internal)
#   THREADS  space-separated TEMPER_ONNX_INTRA_THREADS values to sweep (default "1 2")
set -euo pipefail

# Repo root = three levels up from this script (crates/temper-ingest/bench/).
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
IMAGE=temper-embed-bench
TARGET_VOL=temper-embed-bench-target
# Persist the downloaded crate registry across the separate `docker run`s (build + per-thread runs)
# so only the first invocation pays the fetch. Mount the registry subdir, not all of CARGO_HOME —
# a volume over /usr/local/cargo would hide the toolchain's own cargo binary.
REG_VOL=temper-embed-bench-registry
MODEL_REL=crates/temper-ingest/models/bge-base-en-v1.5/model_quantized.onnx

CPUS="${CPUS:-1.5}"
MEMORY="${MEMORY:-3009m}"
THREADS="${THREADS:-1 2}"

if [[ "${REBUILD:-0}" == "1" ]] || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "==> building $IMAGE"
  docker build -t "$IMAGE" "$ROOT/crates/temper-ingest/bench"
fi

# Warm the build once (release compile of ort + deps) so per-thread runs don't each rebuild.
echo "==> compiling probe (release) — first run downloads crates + builds ort, later runs are cached"
docker run --rm \
  -v "$ROOT:/repo:ro" -v "$TARGET_VOL:/target" -v "$REG_VOL:/usr/local/cargo/registry" \
  "$IMAGE" \
  cargo build --release --locked -p temper-ingest \
    --no-default-features --features embed-download --example coldload_probe

for t in $THREADS; do
  echo
  echo "======================================================================"
  echo "  cpus=$CPUS  memory=$MEMORY  TEMPER_ONNX_INTRA_THREADS=$t"
  echo "======================================================================"
  # --cpus throttles to a Vercel-like slice; the model is read from the mounted checkout.
  docker run --rm \
    --cpus="$CPUS" --memory="$MEMORY" \
    -e TEMPER_ONNX_INTRA_THREADS="$t" \
    -e TEMPER_ONNX_MODEL_PATH="/repo/$MODEL_REL" \
    -v "$ROOT:/repo:ro" -v "$TARGET_VOL:/target" -v "$REG_VOL:/usr/local/cargo/registry" \
    "$IMAGE" \
    cargo run --release --locked -p temper-ingest \
      --no-default-features --features embed-download --example coldload_probe
done
