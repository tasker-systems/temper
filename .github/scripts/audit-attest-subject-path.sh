#!/usr/bin/env bash
# audit-attest-subject-path.sh — assert the build provenance attestation covers BOTH shipped objects:
# the release archive AND the per-file `.manifest.json`.
#
# WHY THIS EXISTS
# ---------------
# `build-cli-binaries.yml` runs `actions/attest-build-provenance@v4` with a `subject-path:` block
# listing three lines — the two archive shapes and the manifest:
#
#     subject-path: |
#       temper-v${{ inputs.version }}-${{ matrix.target.triple }}.tar.gz
#       temper-v${{ inputs.version }}-${{ matrix.target.triple }}.zip
#       temper-v${{ inputs.version }}-${{ matrix.target.triple }}.manifest.json
#
# The manifest line is NOT redundant with the archive lines, and that is the whole point. An
# attestation is keyed on the digest of ONE object; a genuine, still-attested archive says nothing
# about a manifest sitting next to it. Two shipped code paths depend on the manifest being its own
# attested subject:
#
#   version.rs  `--verify --online`  — attests the MANIFEST's digest, because the manifest is the
#                                      object that comparison actually depends on.
#   update.rs   `download_and_verify_release` — attests the archive's digest (what gets installed)
#                                      AND the manifest's, because install.sh persists the manifest
#                                      into the install dir as `.temper-manifest.json`, the
#                                      permanent baseline for every future offline verdict.
#
# Delete the manifest line and BOTH of those start resolving `/attestations/sha256:{digest}` against
# a subject that was never attested. Nothing in this repo would notice: the workflow still runs, the
# archive is still signed, every test still passes, and the first symptom is a real release failing
# in a user's terminal. `subject-path` is also specifically forgiving here — @actions/glob resolves
# each line independently and only errors if the COMBINED match set is empty (see the comment above
# the block in the workflow), so losing one line of three does not even fail the signing job.
#
# WHAT IT ASSERTS
#   (a) The attest step exists and its `subject-path` block is non-empty. A guard that finds nothing
#       must fail: an empty set satisfies every assertion below vacuously.
#   (b) Some entry names a release archive (`.tar.gz` or `.zip`).
#   (c) Some entry ends in `.manifest.json` — the LITERAL suffix, deliberately. A coordinated rename
#       to `.manifest.txt` across the emit step and this block would leave CI green while
#       `update.rs:372` (`format!("temper-{tag}-{target}.manifest.json")`), `version.rs:282` and
#       `install.sh:121` all still fetch `.manifest.json`. Pinning the suffix makes that rename go
#       red here and forces the four sites to move together.
#   (d) The attested manifest filename is EXACTLY the one the release emits — the `OUTPUT:` env of
#       the `emit-manifest.sh` step. This is the one-sided-drift case: rename what is produced but
#       not what is attested (or the reverse) and the attestation covers a file that never exists.
#
# WHAT IT DOES NOT ASSERT — it reads this workflow and nothing else. That the CLI's constructed
# manifest URL agrees with `install.sh`'s `MANIFEST=` line is a separate invariant, already pinned by
# unit tests in `update.rs` and `version.rs`; duplicating it here would be a second place to update.
#
# USAGE
#   .github/scripts/audit-attest-subject-path.sh          # verify (CI mode)
#   .github/scripts/audit-attest-subject-path.sh --list   # print the extracted subject-path entries
#   .github/scripts/audit-attest-subject-path.sh <path>   # audit a fixture (see the test harness)

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

DEFAULT_WORKFLOW=".github/workflows/build-cli-binaries.yml"

LIST=0
if [[ "${1:-}" == "--list" ]]; then
  LIST=1
  shift
fi
WORKFLOW="${1:-$DEFAULT_WORKFLOW}"

if [[ ! -f "$WORKFLOW" ]]; then
  echo "audit-attest-subject-path: FAIL — workflow not found: $WORKFLOW" >&2
  exit 1
fi

# Steps are split on the list-item marker (`- name:` / `- uses:`) rather than on a hard-coded
# indent, so a reindent of the workflow does not silently turn this guard into a no-op. Each step is
# buffered whole because the keys we need sit on BOTH sides of the line that identifies the step:
# `subject-path:` follows `uses:`, but `OUTPUT:` precedes `run:` (env blocks come first).

# The `subject-path` entries of the attest-build-provenance step, one per line.
extract_subjects() {
  awk '
    function indent_of(s,   n) { n = match(s, /[^ ]/); return (n == 0 ? -1 : n - 1) }
    function process(b,   n, i, arr, sp, ind, line, v) {
      if (b !~ /attest-build-provenance/) return
      n = split(b, arr, "\n")
      sp = -1
      for (i = 1; i <= n; i++) {
        line = arr[i]
        if (sp < 0) {
          if (line ~ /^[[:space:]]*subject-path:/) {
            sp = indent_of(line)
            # Inline scalar form (`subject-path: one/file`) rather than a `|` block.
            v = line
            sub(/^[[:space:]]*subject-path:[[:space:]]*/, "", v)
            sub(/[[:space:]]+$/, "", v)
            if (v != "" && v != "|" && v != ">" && v != "|-" && v != ">-") print v
          }
          continue
        }
        if (line ~ /^[[:space:]]*$/) continue
        if (indent_of(line) <= sp) { sp = -1; continue }
        v = line
        sub(/^[[:space:]]*/, "", v)
        sub(/[[:space:]]+$/, "", v)
        # A `#` line inside a block scalar is literal content, never a comment — but it is also
        # never a path, so dropping it cannot hide a real subject.
        if (v != "" && substr(v, 1, 1) != "#") print v
      }
    }
    /^[[:space:]]*-[[:space:]]+(name|uses):/ { process(buf); buf = "" }
    { buf = buf $0 "\n" }
    END { process(buf) }
  ' "$WORKFLOW"
}

# The manifest filename the release actually emits: the `OUTPUT:` env of the emit-manifest.sh step.
extract_emitted_manifest() {
  awk '
    function process(b,   n, i, arr, v) {
      if (b !~ /emit-manifest\.sh/) return
      n = split(b, arr, "\n")
      for (i = 1; i <= n; i++) {
        if (arr[i] ~ /^[[:space:]]*OUTPUT:[[:space:]]*[^[:space:]]/) {
          v = arr[i]
          sub(/^[[:space:]]*OUTPUT:[[:space:]]*/, "", v)
          sub(/[[:space:]]+$/, "", v)
          print v
        }
      }
    }
    /^[[:space:]]*-[[:space:]]+(name|uses):/ { process(buf); buf = "" }
    { buf = buf $0 "\n" }
    END { process(buf) }
  ' "$WORKFLOW"
}

SUBJECTS="$(extract_subjects)"

if [[ "$LIST" == "1" ]]; then
  printf '%s\n' "$SUBJECTS"
  exit 0
fi

fail=0

# (a) Non-vacuity. No attest step, a renamed action, or an emptied block all land here — and all of
# them mean the release signs nothing, so none of them may pass quietly.
if [[ -z "$SUBJECTS" ]]; then
  echo "audit-attest-subject-path: FAIL — no \`subject-path\` entries found in $WORKFLOW." >&2
  echo "  Either the \`actions/attest-build-provenance\` step is gone, it was renamed, or its" >&2
  echo "  subject list is empty. A guard that finds nothing must fail: an empty set satisfies" >&2
  echo "  'attests the archive' and 'attests the manifest' vacuously." >&2
  exit 1
fi

# (b) The archive half.
if ! printf '%s\n' "$SUBJECTS" | grep -qE '\.(tar\.gz|zip)$'; then
  echo "audit-attest-subject-path: FAIL — no \`subject-path\` entry names a release archive." >&2
  echo "  Expected an entry ending in .tar.gz or .zip. The archive is what gets installed;" >&2
  echo "  \`temper update\` verifies its attestation before handing it to install.sh." >&2
  echo "  subject-path entries found:" >&2
  printf '%s\n' "$SUBJECTS" | sed 's/^/    /' >&2
  fail=1
fi

# (c) The manifest half — the one that is silently deletable.
ATTESTED_MANIFEST="$(printf '%s\n' "$SUBJECTS" | grep -E '\.manifest\.json$' || true)"
if [[ -z "$ATTESTED_MANIFEST" ]]; then
  echo "audit-attest-subject-path: FAIL — no \`subject-path\` entry ends in .manifest.json." >&2
  echo "  The manifest must be its OWN attested subject. An attestation is keyed on the digest of" >&2
  echo "  one object, so the archive's attestation says nothing about the manifest beside it." >&2
  echo "  Without this entry, \`temper version --verify --online\` and \`temper update\`'s manifest" >&2
  echo "  check both resolve /attestations/sha256:{digest} against a subject nobody attested." >&2
  echo "  The .manifest.json suffix is pinned on purpose: update.rs, version.rs and install.sh all" >&2
  echo "  construct that literal filename, so a rename here must not be able to go green alone." >&2
  echo "  subject-path entries found:" >&2
  printf '%s\n' "$SUBJECTS" | sed 's/^/    /' >&2
  fail=1
fi

# (d) Emitted vs attested. A one-sided rename attests a file the release never produces.
EMITTED_MANIFEST="$(extract_emitted_manifest)"
if [[ -z "$EMITTED_MANIFEST" ]]; then
  echo "audit-attest-subject-path: FAIL — could not find the emit-manifest.sh step's OUTPUT in $WORKFLOW." >&2
  echo "  Without it there is nothing to compare the attested manifest name against, and this" >&2
  echo "  guard would assert only that SOME .manifest.json is listed — not that it is the one" >&2
  echo "  the release actually writes." >&2
  fail=1
elif [[ -n "$ATTESTED_MANIFEST" && "$ATTESTED_MANIFEST" != "$EMITTED_MANIFEST" ]]; then
  echo "audit-attest-subject-path: FAIL — the attested manifest is not the one the release emits." >&2
  echo "  emit-manifest.sh writes: $EMITTED_MANIFEST" >&2
  echo "  subject-path attests:    $ATTESTED_MANIFEST" >&2
  echo "  These must be byte-identical. If they drift, the attestation covers a path that does not" >&2
  echo "  exist and the manifest that IS published ships unattested." >&2
  fail=1
fi

if [[ "$fail" == "0" ]]; then
  echo "audit-attest-subject-path: OK — $(printf '%s\n' "$SUBJECTS" | grep -c .) attested subjects; archive and manifest ($ATTESTED_MANIFEST) both covered."
fi
exit "$fail"
