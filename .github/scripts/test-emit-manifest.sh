#!/usr/bin/env bash
# Harness for emit-manifest.sh. Proves the generator emits the wire shape that
# crates/temper-cli/src/manifest.rs parses — the two are one contract.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EMIT="${SCRIPT_DIR}/release/emit-manifest.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# A staging dir shaped like a real unix release archive.
mkdir -p "$TMP/staging/lib" "$TMP/staging/models"
printf 'binary' > "$TMP/staging/temper"
printf 'ort'    > "$TMP/staging/lib/libonnxruntime.so"
printf 'model'  > "$TMP/staging/models/model_quantized.onnx"
printf 'lic'    > "$TMP/staging/LICENSE"

OUT="$TMP/out.json"
VERSION=0.3.0 TARGET=x86_64-unknown-linux-gnu STAGING="$TMP/staging" OUTPUT="$OUT" \
  bash "$EMIT"

[ -s "$OUT" ] || fail "no manifest emitted"

# 1. Valid JSON with the expected top-level keys.
jq -e '.version == "0.3.0"' "$OUT" >/dev/null || fail "version key wrong"
jq -e '.target == "x86_64-unknown-linux-gnu"' "$OUT" >/dev/null || fail "target key wrong"

# 2. Every shipped file is listed, with a paths-are-relative invariant.
for p in temper lib/libonnxruntime.so models/model_quantized.onnx LICENSE; do
  jq -e --arg p "$p" 'any(.files[]; .path == $p)' "$OUT" >/dev/null \
    || fail "missing entry for $p"
done

# 3. Hashes are real sha256 of the real bytes — the bite. A generator that
#    emitted a constant, or hashed the wrong file, fails here.
EXPECTED=$(printf 'binary' | { sha256sum 2>/dev/null || shasum -a 256; } | awk '{print $1}')
ACTUAL=$(jq -r '.files[] | select(.path == "temper") | .sha256' "$OUT")
[ "$EXPECTED" = "$ACTUAL" ] || fail "sha256 for temper is $ACTUAL, expected $EXPECTED"

# 4. Sizes are real.
SIZE=$(jq -r '.files[] | select(.path == "temper") | .size' "$OUT")
[ "$SIZE" = "6" ] || fail "size for temper is $SIZE, expected 6"

# 5. No absolute paths or staging-dir leakage.
jq -e 'all(.files[]; (.path | startswith("/")) | not)' "$OUT" >/dev/null \
  || fail "manifest contains absolute paths"

echo "PASS: emit-manifest emits the golden wire shape over real bytes"
