#!/usr/bin/env bash
# check-production-features.sh — no test feature reaches a shipped binary.
#
# The machine-principal invariants are held by types whose test-only doors are compiled out of
# production by Cargo features: `Principal::Bare`, `DbBackend::new`, `require_system_admin_by_id`
# and `test_support::human_principal_for` (which mints a `HumanPrincipal` from a bare id) exist only
# under `temper-services`' `test-harness` / `test-db`. Today only dev-dependencies enable those, and
# with resolver 2 a dev-dependency's features never reach a normal build. Nothing else holds that:
# the compile-fail fixture `backend_new_without_test_harness` proves the cfg exists in
# temper-services' own lane, not that the deployed binary's graph leaves it off. One normal
# dependency edge naming `features = ["test-db"]` would compile every one of those doors into
# production with every other gate green.
#
# So for each shipped package this resolves its NORMAL (and build) dependency graph with
# `cargo tree -e normal,build --target all` (features unified across every target, a superset of
# any one platform's release build), and fails if any
# workspace crate in it carries a feature whose name contains `test` — every test-gating feature in
# this workspace does (`test-db`, `test-harness`, `test-embed`, `test-support`,
# `integration-tests`, `artifact-tests`). It also fails if `temper-services` stops appearing under
# `temper-cloud`: a check that reads no services line has nothing to say about the doors above.
#
# Shipped packages: `temper-cloud` (the Vercel functions under api/*.rs) and `temper-cli` (the
# release binaries). Override with DEPLOYED_PACKAGES for a harness. CARGO_TREE_CMD substitutes the
# `cargo tree` invocation (the harness stubs it; it receives the package name as its argument).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

DEPLOYED="${DEPLOYED_PACKAGES:-temper-cloud temper-cli}"
REPO_ROOT="$(pwd)"

cargo_tree() {
  if [[ -n "${CARGO_TREE_CMD:-}" ]]; then
    "$CARGO_TREE_CMD" "$1"
  else
    cargo tree --locked -p "$1" -e normal,build --target all --prefix none -f '{p} [{f}]'
  fi
}

fail=0
for pkg in $DEPLOYED; do
  if ! tree="$(cargo_tree "$pkg")"; then
    echo "check-production-features: FAIL — cargo tree failed for ${pkg}" >&2
    fail=1
    continue
  fi
  # Workspace crates only: a `{p}` line carries the crate's source path, and a workspace crate's is
  # under this repository. Third-party feature names are not this check's question.
  local_lines="$(printf '%s\n' "$tree" | grep -F "(${REPO_ROOT}" | sed 's/ (\*)$//' | sort -u || true)"
  if ! printf '%s\n' "$local_lines" | grep -Eq "^${pkg} v"; then
    echo "check-production-features: FAIL — ${pkg}: its own line is not in the cargo tree output" >&2
    echo "  (the output format moved, or the package was renamed; this check read nothing)." >&2
    fail=1
    continue
  fi
  if [[ "$pkg" == "temper-cloud" ]] && ! printf '%s\n' "$local_lines" | grep -Eq '^temper-services v'; then
    echo "check-production-features: FAIL — temper-cloud's graph has no temper-services line;" >&2
    echo "  the doors this check guards live there, so a pass would say nothing." >&2
    fail=1
  fi
  offenders="$(printf '%s\n' "$local_lines" | awk '
    {
      if (match($0, /\[[^]]*\]$/)) {
        feats = substr($0, RSTART + 1, RLENGTH - 2)
        n = split(feats, f, ",")
        for (i = 1; i <= n; i++) if (f[i] ~ /test/) { print; next }
      }
    }' || true)"
  if [[ -n "$offenders" ]]; then
    echo "check-production-features: FAIL — ${pkg} compiles a test feature into a shipped build:" >&2
    printf '%s\n' "$offenders" | sed "s|${REPO_ROOT}/||; s/^/  /" >&2
    echo "  A test feature reaches production only through a normal dependency edge that names it." >&2
    echo "  Move that edge to [dev-dependencies], or gate it behind the dependent's own test feature." >&2
    fail=1
  fi
done

if [[ "$fail" == "0" ]]; then
  echo "check-production-features: OK — no test feature in the normal graph of: ${DEPLOYED}"
fi
exit "$fail"
