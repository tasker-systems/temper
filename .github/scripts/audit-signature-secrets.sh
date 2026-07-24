#!/usr/bin/env bash
# audit-signature-secrets.sh — assert each internal signature gate reads a DISTINCT secret.
#
# WHY THIS EXISTS
# ---------------
# temper-api has three HMAC signature gates, each guarding a server-to-server surface that
# `require_auth` never sees (see audit-route-auth.sh's REVIEW_GROUPS table):
#
#   GATE                            SECRET FIELD                  GUARDS
#   ------------------------------  ----------------------------  ------------------------------
#   require_internal_signature      internal_reconcile_secret     /internal/saml/reconcile
#   require_slack_link_signature    hmac_secret (SlackLinkConfig) /internal/slack/link-state
#   require_slack_mint_signature    slack_mint_secret             /internal/slack/mint
#
# The KEYS MUST DIFFER, and that is the entire reason `slack_mint_internal_routes` is a third
# signature group rather than one more route on `slack_link_internal_routes`. The two capabilities
# are worth wildly different amounts: link-state answers "is this principal linked?" — a question.
# The mint route vends an act-as-the-human access token carrying that human's FULL reach
# (`resources_visible_to` takes a profile and nothing else; there is no narrowing behind it), and
# its signature gate is the ONLY thing enforcing "naming a principal must not be sufficient to mint
# its token" — `mint_access_token` authorizes nothing itself.
#
# So a shared key is not a tidiness problem. Possession of the cheap key would forge the expensive
# call: whoever can ask "is Alice linked?" could instead mint Alice's token. Collapsing two of these
# onto one config field is a one-line edit that no type checks, no route audit notices (all three
# layers are still mounted, so audit-route-auth.sh stays green), and — until this script — nothing
# in CI caught. It was defended only by an e2e test, i.e. only where someone remembered to look.
#
# This asserts the invariant STRUCTURALLY: pairwise-distinct secret fields across the gates. Note
# the distinctness check is COMPUTED, not baselined — UPDATE_BASELINE cannot silence it. The
# baseline exists only to catch a gate being added, removed, or repointed at a different field.
#
# IT CHECKS THE SOURCE, NEVER THE DEPLOYED VALUES — and it cannot. Two gates reading two
# differently-named env vars satisfy this script whatever those vars actually contain, so an
# operator who wires a new instance by copy-paste can set them to ONE value and collapse the whole
# privilege split with every gate here still green. That half is asserted at boot instead, by
# `check_secret_distinctness` (crates/temper-services/src/config.rs), which refuses to start when
# any two of the five shared secrets hold the same value — a wider set than the three gates below,
# because it also covers EMBED_DISPATCH_SECRET and SLACK_VAULT_ENC_KEY. The Slack mention agent
# holds its own copies in a SEPARATE deployment and asserts the same thing at its call sites
# (`assertSlackSecretsDistinct`, packages/agent-workflows/mention/agent/lib/link.ts).
#
# The three are one invariant at three altitudes: distinct FIELDS (here), distinct VALUES at boot,
# distinct VALUES in the other deployment. None of them subsumes another.
#
# USAGE
#   .github/scripts/audit-signature-secrets.sh          # verify (CI mode)
#   .github/scripts/audit-signature-secrets.sh --list   # print current gate -> secret mapping
#   UPDATE_BASELINE=1 .github/scripts/audit-signature-secrets.sh   # rewrite baseline after review
#
# MIDDLEWARE_FILE may be overridden to point at a fixture (see test-audit-signature-secrets.sh).

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

MIDDLEWARE_FILE="${MIDDLEWARE_FILE:-crates/temper-api/src/middleware/internal_auth.rs}"

# Reviewed baseline: <gate>\t<comma-separated secret fields it reads>. A change means a gate was
# added, removed, or repointed at a different config field — confirm the key separation above
# still holds (and that any new gate has its OWN key), then UPDATE_BASELINE=1.
read -r -d '' BASELINE <<'EOF' || true
require_internal_signature	internal_reconcile_secret
require_slack_link_signature	hmac_secret
require_slack_mint_signature	slack_mint_secret
EOF

# Map each signature gate to the secret field(s) its body reads.
#
# Keys on identifiers ending in `_secret` — that catches `internal_reconcile_secret`,
# `slack_mint_secret` and `hmac_secret` while ignoring the local binding `let secret = ...`, which
# every gate shares and which would otherwise collapse all three to a false match. Comment lines are
# stripped first so prose about "this secret" cannot contribute a field.
extract() {
  awk '
    /^(pub )?async fn require_[a-z_]*signature\(/ {
      fname=$0; sub(/.*async fn /,"",fname); sub(/\(.*/,"",fname); inside=1; next
    }
    inside && /^\}/ { inside=0; next }
    !inside { next }
    { line=$0; sub(/[[:space:]]*\/\/.*/,"",line) }
    {
      while (match(line, /[a-z][a-z0-9_]*_secret/)) {
        print fname"\t"substr(line, RSTART, RLENGTH)
        line = substr(line, RSTART+RLENGTH)
      }
    }
  ' "$MIDDLEWARE_FILE" | sort -u \
  | awk -F'\t' '{ if ($1!=p) { if (p!="") print p"\t"v; p=$1; v=$2 } else v=v","$2 } END { if (p!="") print p"\t"v }'
}

CURRENT="$(extract)"

if [[ "${1:-}" == "--list" ]]; then
  printf '%s\n' "$CURRENT"
  exit 0
fi
if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  printf '%s\n' "$CURRENT"
  echo "^^^ copy into BASELINE after confirming every gate still reads its OWN key." >&2
  exit 0
fi

fail=0

# (a) No gate may be left with no discoverable secret — that means the extraction lost the gate
# (renamed/reshaped) and every assertion below would pass vacuously over an empty set.
if [[ -z "$CURRENT" ]]; then
  echo "audit-signature-secrets: FAIL — no signature gates found in $MIDDLEWARE_FILE." >&2
  echo "  Either the gates moved, or the extraction no longer matches their shape. A guard that" >&2
  echo "  finds nothing must fail, not pass: an empty set satisfies every assertion vacuously." >&2
  exit 1
fi

# (b) THE INVARIANT: the gates' secret fields must be pairwise distinct. Computed from the file,
# never from the baseline — UPDATE_BASELINE cannot silence this one.
DUPES="$(printf '%s\n' "$CURRENT" | cut -f2 | tr ',' '\n' | sort | uniq -d)"
if [[ -n "$DUPES" ]]; then
  echo "audit-signature-secrets: FAIL — signature gates SHARE a secret:" >&2
  printf '%s\n' "$DUPES" | while IFS= read -r d; do
    [[ -n "$d" ]] || continue
    echo "  '$d' is read by: $(printf '%s\n' "$CURRENT" | grep -F "$d" | cut -f1 | tr '\n' ' ')" >&2
  done
  echo "  These keys must differ. link-state answers a question; mint vends an act-as-the-human" >&2
  echo "  token with that human's FULL reach. One shared key lets the cheap capability forge the" >&2
  echo "  expensive one. Give each gate its own config field." >&2
  fail=1
fi

# (c) The gate -> secret mapping must match the reviewed baseline.
NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -u)"
if ! diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$CURRENT") >/tmp/signature-secrets.diff 2>&1; then
  echo "audit-signature-secrets: FAIL — the gate -> secret mapping changed." >&2
  echo "Confirm each gate still reads its OWN key, and that any NEW gate does not reuse one." >&2
  echo "diff (baseline -> current):" >&2
  cat /tmp/signature-secrets.diff >&2
  echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-signature-secrets.sh" >&2
  fail=1
fi

if [[ "$fail" == "0" ]]; then
  echo "audit-signature-secrets: OK — $(printf '%s\n' "$CURRENT" | grep -c .) signature gates, all reading distinct secrets."
fi
exit "$fail"
