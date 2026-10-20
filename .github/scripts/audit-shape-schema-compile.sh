#!/usr/bin/env bash
# audit-shape-schema-compile.sh — fail if any server code compiles a JSON Schema through a
# `jsonschema` entry point other than the one no-fetch constructor.
#
# WHY THIS EXISTS
# ---------------
# A data-artifact shape's schema is written by any author of its home. `jsonschema` under its
# default options follows a `$ref` (or an unrecognised `$schema`) by fetching it: an http(s) URI
# over the network, a `file:` URI from the server's disk. So a compile with default options is a
# fetch trigger any shape author can fire, and the compiler's error text can echo what came back.
# Task 01a15661-8460-739d-9ed3-f1e8ccbf9b69 closed that: every compile of a shape schema now goes
# through `temper_substrate::shape_schema::compile`, which builds with a retriever that refuses
# every URI.
#
# The constructor only holds while nothing else compiles. Calling `jsonschema::validator_for(..)`
# in a new place silently restores the defaults, and nothing in the type system objects. This
# script pins the set of `jsonschema` compile entry points under `crates/*/src` to the reviewed
# allowlist below, so a new one cannot land without a human answering: can an author of this
# schema reach it, and if so why is it not `shape_schema::compile`?
#
# FIELD OF VIEW (named, because a guard whose coverage quietly narrows is worse than none)
#   * `crates/*/src` only; `tests/` directories are not scanned. A test may build a validator.
#   * A hit inside a `#[cfg(test)]` module counts, as in the sibling sink guards.
#   * Sees: `jsonschema::<entry point>` paths (validator_for, options, validate, is_valid,
#     evaluate, bundle, draft*, meta, and the async twins); `ValidationOptions::default`;
#     `Registry::(try_new|try_from_resources|options)`; `RegistryOptions::(new|default)`; bare
#     `Validator::(new|options|async_*)`;
#     single-line `use jsonschema::{..}` imports of an entry point; and the aliasing imports
#     `use jsonschema as ..`, `use jsonschema::*` and `extern crate jsonschema`, since each one
#     hides the `jsonschema::` prefix every other pattern keys on.
#   * Whole-line `//` comments are excluded, so doc prose may name an entry point.
#   * RESIDUAL BLIND SPOTS, stated so nobody reads green as proof: a `use jsonschema::{..}` whose
#     braces span lines; a call assembled by a macro (the textual forms above are seen wherever
#     they appear in a macro body, but a name built with `paste!`/`concat!`/`stringify!` is not);
#     a type reached through a re-export from another crate; and `Default::default()` where the
#     type is inferred to be `ValidationOptions`. A reviewer of a new `jsonschema` dependency edge
#     is the backstop for those. The second layer is `default-features = false` on every
#     `jsonschema` dependency, which compiles the http and file resolvers out; it has its own check
#     below (no baseline: a manifest line without it fails outright). That check strips comments,
#     reads one-line entries, dotted keys and `[..dependencies.jsonschema]` tables, accepts
#     `workspace = true` inheritance (the root's `[workspace.dependencies]` line is itself
#     checked), and FAILS CLOSED on any other line whose key starts with `jsonschema`. It does not
#     see a dependency renamed with `package = "jsonschema"`, or a multi-line inline table.
#
# USAGE
#   .github/scripts/audit-shape-schema-compile.sh            # verify against the allowlist (CI mode)
#   .github/scripts/audit-shape-schema-compile.sh --list     # print the current sites
#   UPDATE_BASELINE=1 .github/scripts/audit-shape-schema-compile.sh   # print a proposed allowlist
#
# SCAN_ROOT may be overridden to point at a fixture tree (see test-audit-shape-schema-compile.sh).
# Exit 0 = the set is unchanged. Exit 1 = a site was added, removed, or moved — review required.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

SCAN_ROOT="${SCAN_ROOT:-crates}"
# Where the manifest check looks for Cargo.toml files; overridable for the harness.
MANIFEST_ROOT="${MANIFEST_ROOT:-.}"

# The reviewed allowlist: <count> <path>, sorted by path.
#
# REVIEWED 2026-10-20 (task 01a15661-8460-739d-9ed3-f1e8ccbf9b69, PR A) — initial allowlist.
#   Re-run 2026-10-20 after the patterns were widened (evaluate, bundle, ValidationOptions::default,
#   Registry::*, bare Validator::*, aliasing imports): the three files' counts did not move.
#   crates/temper-substrate/src/shape_schema.rs (1) — THE constructor: `jsonschema::options()`
#     with a retriever that refuses every URI. This is the allowlisted entry because it is the
#     thing every other compile must call.
#   crates/temper-workflow/src/schema.rs (2) — `load_schema` and `load_open_meta_schema` compile
#     the doc-type and open-meta schemas SHIPPED IN THIS REPOSITORY (`include_str!` data under
#     `temper-workflow/schemas/`). They are not shape schemas and no caller authors them: the
#     input is fixed at build time, so there is no author who could name a URI. REVIEWED as not
#     reachable by a schema author. If either ever compiles a schema that arrives at runtime,
#     it stops being allowlistable and must move to `shape_schema::compile`.
read -r -d '' BASELINE <<'EOS' || true
1 crates/temper-substrate/src/shape_schema.rs
2 crates/temper-workflow/src/schema.rs
EOS

# Every `jsonschema` entry point that compiles or validates a schema: the free functions, the
# options builders (sync and async), the bundlers and the bundled-draft modules. Prefix-matched on
# purpose (`bundle` also catches `bundle_with_..`): over-matching only costs a review.
ENTRY_POINTS='validator_for|async_validator_for|options|async_options|validate|is_valid|evaluate|(async_)?bundle|draft[0-9]+|meta'

# Current set: <count> <path> per file, sorted by path. `|| true` keeps a legitimately empty scan
# from tripping `set -e` before the canary below can report it.
current() {
  grep -rnE --include='*.rs' \
     -e "jsonschema::(${ENTRY_POINTS})" \
     -e "jsonschema::Validator::(new|options|async_new|async_options)" \
     -e "(^|[^[:alnum:]_])Validator::(new|options|async_new|async_options)([^[:alnum:]_]|$)" \
     -e "ValidationOptions(::<.*>)?::default" \
     -e "Registry::(try_new|try_from_resources|options)" \
     -e "RegistryOptions::(new|default)" \
     -e "use[[:space:]]+jsonschema::\{[^}]*(validator_for|async_validator_for|options|async_options|validate|is_valid|evaluate|bundle|async_bundle)" \
     -e "use[[:space:]]+jsonschema[[:space:]]+as([^[:alnum:]_]|$)" \
     -e "use[[:space:]]+jsonschema::\*" \
     -e "extern[[:space:]]+crate[[:space:]]+jsonschema" \
     "$SCAN_ROOT" 2>/dev/null \
  | grep -E '^[^:]*/src/[^:]*\.rs:' \
  | grep -vE '^[^:]*:[0-9]+:[[:space:]]*//' \
  | awk -F: '{print $1}' \
  | sort | uniq -c \
  | awk '{printf "%s %s\n", $1, $2}' \
  | sort -k2 \
  || true
}

# Layer 2: one line per `jsonschema` dependency declaration in any Cargo.toml,
# "<file>\t<ok|bad>\t<what was read>".
# `ok` = default-features = false, or inherited from the workspace (whose own line is checked).
# Read, after stripping comments (quote-aware, so `# default-features = false` is not a value):
#   * a one-line entry, `jsonschema = ..`;
#   * dotted keys, `jsonschema.workspace = true`, `jsonschema.default-features = false`,
#     `jsonschema.version = ..`, aggregated per table: any dotted key is `ok` only if the table
#     also has the default-features or workspace one;
#   * a `[..dependencies.jsonschema]` table.
# FAIL CLOSED: any other line whose key starts with `jsonschema` (a quoted key, an unfamiliar
# spelling, a sibling crate such as `jsonschema-foo`) is `bad`, because the scanner cannot say what
# it enables. Extend the scanner for a legitimate new form rather than ignoring the line.
manifest_scan() {
  find "$MANIFEST_ROOT" -name Cargo.toml \
       -not -path '*/target/*' -not -path '*/node_modules/*' -not -path '*/.git/*' -print0 2>/dev/null \
  | xargs -0 awk '
    function emit(f, good, what) { print f "\t" (good ? "ok" : "bad") "\t" what }
    # Drop a trailing `# comment`, honouring quotes so a `#` inside a string survives.
    function strip(line,   i, c, q, out) {
      q = ""; out = ""
      for (i = 1; i <= length(line); i++) {
        c = substr(line, i, 1)
        if (q == "") {
          if (c == "#") break
          if (c == "\"" || c == "\047") q = c
        } else if (c == "\\" && q == "\"") {
          out = out c; i++; c = substr(line, i, 1)
        } else if (c == q) {
          q = ""
        }
        out = out c
      }
      return out
    }
    function flush_table(   ) {
      if (intable) emit(tfile, tok, "[..dependencies.jsonschema] table")
      if (dseen) emit(dfile, dok, "dotted jsonschema.* keys")
      intable = 0; dseen = 0
    }
    FNR == 1 { flush_table() }
    {
      line = strip($0)
      if (line ~ /^[[:space:]]*\[/) {
        flush_table()
        if (line ~ /dependencies\.("?)jsonschema("?)[[:space:]]*\]/) { intable = 1; tfile = FILENAME; tok = 0 }
        next
      }
      if (intable && line ~ /^[[:space:]]*default-features[[:space:]]*=[[:space:]]*false([[:space:]]|$)/) tok = 1
      if (intable && line ~ /^[[:space:]]*workspace[[:space:]]*=[[:space:]]*true([[:space:]]|$)/) tok = 1
      if (line ~ /^[[:space:]]*["\047]?jsonschema/) {
        if (line ~ /^[[:space:]]*jsonschema[[:space:]]*=/) {
          emit(FILENAME, (line ~ /default-features[[:space:]]*=[[:space:]]*false/ || line ~ /workspace[[:space:]]*=[[:space:]]*true/), "one-line entry: " line)
        } else if (line ~ /^[[:space:]]*jsonschema\./) {
          if (!dseen) { dseen = 1; dok = 0; dfile = FILENAME }
          if (line ~ /^[[:space:]]*jsonschema\.default-features[[:space:]]*=[[:space:]]*false([[:space:]]|$)/) dok = 1
          if (line ~ /^[[:space:]]*jsonschema\.workspace[[:space:]]*=[[:space:]]*true([[:space:]]|$)/) dok = 1
        } else {
          emit(FILENAME, 0, "unrecognized form: " line)
        }
      }
    }
    END { flush_table() }
  ' 2>/dev/null || true
}

CURRENT="$(current)"

if [[ "${1:-}" == "--list" ]]; then
  echo "$CURRENT"
  exit 0
fi

if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "$CURRENT"
  echo "^^^ copy the block above into BASELINE in this script, with a REVIEWED note per changed" >&2
  echo "    site saying why no shape author can reach it (only after reviewing each one)." >&2
  exit 0
fi

# An empty scan must fail, not pass: the allowlist is never empty (the constructor itself is on
# it), so zero hits means the scan stopped matching — a moved tree, a changed path shape — and an
# empty set diffs clean against nothing while asserting nothing at all.
if [[ -z "$CURRENT" ]]; then
  echo "audit-shape-schema-compile: FAIL — the scan found NO jsonschema entry points under" >&2
  echo "  $SCAN_ROOT. The shape_schema constructor itself is one, so finding zero means the scan" >&2
  echo "  broke, not that the tree is clean." >&2
  exit 1
fi

DIFF_FILE="$(mktemp)"
trap 'rm -f "$DIFF_FILE"' EXIT

fail=0

NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -k2)"
if ! diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$CURRENT") >"$DIFF_FILE" 2>&1; then
  fail=1
  cat >&2 <<'MSG'
audit-shape-schema-compile: FAIL — the set of jsonschema compile entry points changed.

Every compile of a shape's schema must go through temper_substrate::shape_schema::compile, which
never fetches. `jsonschema::validator_for`, `jsonschema::options()` and friends use the default
options, which follow a `$ref` to an http(s) or file: URI — a fetch any author of the schema can
trigger. Before accepting this change:
  1. Can any caller-authored schema reach the new site? If yes, call shape_schema::compile instead.
  2. If not (the input is fixed at build time), add the site to BASELINE with a REVIEWED note
     saying why no author can name a URI.

diff (allowlist -> current):
MSG
  cat "$DIFF_FILE" >&2
  echo >&2
  echo "If the change is reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-shape-schema-compile.sh" >&2
fi

# Layer 2. A scan that finds no `jsonschema` dependency at all has stopped matching (the real tree
# has several), so it fails rather than passing.
MANIFESTS="$(manifest_scan | sort)"
if [[ -z "$MANIFESTS" ]]; then
  echo "audit-shape-schema-compile: FAIL — the manifest scan found NO jsonschema dependency under" >&2
  echo "  $MANIFEST_ROOT. The real tree has several, so finding zero means the scan broke." >&2
  fail=1
else
  BAD_MANIFESTS="$(printf '%s\n' "$MANIFESTS" | awk -F'\t' '$2 == "bad"')"
  if [[ -n "$BAD_MANIFESTS" ]]; then
    fail=1
    {
      echo "audit-shape-schema-compile: FAIL — a jsonschema dependency lacks default-features = false:"
      printf '%s\n' "$MANIFESTS" | awk -F'\t' '$2 == "bad" {printf "  %s  (%s)\n", $1, $3}'
      echo "  The default features compile in the http and file resolvers, the second layer behind"
      echo "  shape_schema's refusing retriever. Every dependency edge must say"
      echo "  { version = \"..\", default-features = false }: one edge without it turns the"
      echo "  resolvers back on for the whole build through feature unification."
    } >&2
  fi
fi

if [[ "$fail" == "0" ]]; then
  echo "audit-shape-schema-compile: OK — jsonschema entry points unchanged ($(printf '%s\n' "$CURRENT" | grep -c .) files), $(printf '%s\n' "$MANIFESTS" | grep -c .) jsonschema manifest edges, all default-features = false."
  exit 0
fi
exit 1
