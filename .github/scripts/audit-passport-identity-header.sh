#!/usr/bin/env bash
# audit-passport-identity-header.sh — forbid the edge-injected Passport identity header in CODE.
#
# WHY THIS EXISTS
# ---------------
# Vercel Passport, when fronting an origin, injects a Vercel-signed identity JWT as the
# `x-vercel-oidc-passport-token` request header, and the edge strips any client-supplied value.
# The header is therefore genuinely trustworthy AS TO ITS OWN CLAIMS — it authentically names the
# user who passed the edge challenge. That authenticity is exactly what makes it hazardous here.
#
# Temper's central architectural property is that no surface constructs a principal: every human
# identity arrives through the one seam (`temper-services::auth`), and every authorization
# decision runs through `admit`. Vercel team membership is not Temper standing. Trusting the
# header would admit principals `admit` never approved, bypass `Denied` / `Revoked` entirely, and
# give "who is this caller" two answers that can disagree — on ONE of two surfaces only, since
# Passport fronts the UI origin while the API origin never sees the header. That asymmetry is the
# seam-drift shape this repository has already paid for once (a deactivated account kept MCP
# access), and the cautionary twin is in production today: `webhook_intake`'s anti-decoy
# `client_id` assertion (see audit-route-auth.sh) exists because a verifier stopping at "a valid
# Vercel OIDC token naming our project" would accept the deployment's own ambient identity as a
# forged webhook. Same failure mode, one header over.
#
# The rule itself lives in internal/auth/authorization-seam.md ("Edge-injected identity"). This
# script is its enforcement half — the difference between a documented intention and an enforced
# one is a distinction this repo learned at cost: the auth typestates were "sealed by intent, not
# enforcement" until the trybuild proof landed.
#
# WHAT IT CATCHES
# ---------------
# The literal header name, case-insensitively (HTTP headers are case-insensitive; axum and fetch
# both normalize, but a reader could spell it any way), in any file that is not markdown and not
# explicitly allowlisted below. Markdown is exempt because prose NAMING the header is where the
# rule lives; the failure mode is code READING it. The ambient `x-vercel-oidc-token` — the
# webhook attestation header — is a different, shorter string and does not match this needle;
# reading THAT one is legitimate and load-bearing.
#
# ALLOWLIST
# ---------
# One filesystem path per line, optionally followed by `# the decision that admits it`. Today it
# is EMPTY: the Passport header is read nowhere, including for observability. If a future change
# wants it logged for observability — correlating "reached the edge" with "authenticated to
# Temper" — the logging field is named in the logging contract FIRST, and THEN the file lands
# here citing that decision. An allowlist entry with no citation is a finding, not a lane.
#
# USAGE
#   .github/scripts/audit-passport-identity-header.sh          # verify (CI mode, tracked files)
#   SCAN_ROOT=<dir> .github/scripts/audit-passport-identity-header.sh   # scan a fixture tree
#     (see test-audit-passport-identity-header.sh)

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

NEEDLE='x-vercel-oidc-passport-token'

# Reviewed allowlist — see ALLOWLIST above. Empty as of 2026-08-30: the header reaches no code.
read -r -d '' ALLOWLIST <<'EOF' || true
EOF

is_allowlisted() {
  local path="$1" entry
  while IFS= read -r entry; do
    entry="${entry%%#*}"
    entry="${entry%%[[:space:]]}"
    entry="${entry##[[:space:]]}"
    [ -n "$entry" ] || continue
    [ "$path" = "$entry" ] && return 0
  done <<< "$ALLOWLIST"
  return 1
}

# CI mode scans TRACKED files only — build output and node_modules are not claims anyone made.
# SCAN_ROOT mode (harness fixtures) scans every file under the given root except .git.
# (Plain string + while-read, not mapfile: local runs use macOS bash 3.2, where mapfile does
# not exist — a guard that only runs on Linux CI is a guard nobody can verify locally.)
if [[ -n "${SCAN_ROOT:-}" ]]; then
  FILES_LIST="$(find "$SCAN_ROOT" -type f -not -path '*/.git/*' | sort)"
else
  FILES_LIST="$(git ls-files | sort)"
fi

HITS=""
while IFS= read -r f; do
  [ -n "$f" ] || continue
  # Markdown is the lane the rule itself lives in — prose may NAME the header.
  case "$f" in
    *.md|*.markdown) continue ;;
  esac
  # These two files must spell the needle to test for it.
  case "$f" in
    .github/scripts/audit-passport-identity-header.sh|\
.github/scripts/test-audit-passport-identity-header.sh) continue ;;
  esac
  if is_allowlisted "$f"; then continue; fi
  if grep -qi "$NEEDLE" "$f" 2>/dev/null; then
    HITS="${HITS}${f}
"
  fi
done <<< "$FILES_LIST"

if [[ -z "$HITS" ]]; then
  echo "audit-passport-identity-header: OK — the Passport identity header appears in no non-markdown file outside the (empty) allowlist."
  exit 0
fi

FORMATTED_HITS=""
while IFS= read -r h; do
  [ -n "$h" ] || continue
  FORMATTED_HITS="${FORMATTED_HITS}  ${h}
"
done <<< "$HITS"

cat >&2 <<MSG
audit-passport-identity-header: FAIL — edge-injected identity header named in code:

$(printf '%s' "$FORMATTED_HITS")

An edge-injected identity header is never an input to authentication or authorization. Identity
comes from the seam (temper-services::auth); standing comes from admit. The rule:
internal/auth/authorization-seam.md, "Edge-injected identity".

If this use is observability — recording who reached the edge, nothing more — then (1) name the
log field in the logging contract, and (2) add the file to the allowlist in this script CITING
that decision. Any other use is the finding this guard exists for: remove it.
MSG
exit 1
