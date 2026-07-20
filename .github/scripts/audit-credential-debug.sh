#!/usr/bin/env bash
# audit-credential-debug.sh — pin the set of credential-bearing types that DERIVE `Debug`.
#
# WHY THIS EXISTS
# ---------------
# Several types in this repo carry a live credential and hand-write `Debug` specifically to redact
# it — `MintOutcome` and `NewGrant` (slack_grant_vault_service.rs), `SlackMintResponse`
# (handlers/slack_mint.rs), `VaultKey`, `BrokerToken`, `SlackLinkConfig`. That is a CONVENTION, and
# it is enforced by nothing. Every one of those redactions is one `#[derive(Debug)]` away from
# being undone, silently, by someone who has never heard of it.
#
# The blast radius is not "a scary-looking log line". A single `?value` or `{:?}` on a
# derive-Debug'd token type inside a `tracing::` macro writes an act-as-the-human access token —
# carrying that human's FULL reach — into the platform log, where it is retained, indexed, and
# readable by anyone with log access. Credentials in logs are not revoked by rotating the code.
#
# This does NOT forbid deriving Debug on a credential type — plenty of the baselined entries below
# are deliberate (a CLI-facing DTO whose Debug never reaches a log sink). It PINS the set, so a NEW
# credential-bearing type with a derived Debug fails CI until someone answers: does this type's
# Debug ever reach a log sink, and if it might, should it redact like MintOutcome does?
#
# Scope: production `src` trees only, `#[cfg(test)]` modules excluded — a test fixture struct
# holding a token is not a log-leak path, and baselining them would bury the signal in noise.
#
# USAGE
#   .github/scripts/audit-credential-debug.sh          # verify (CI mode)
#   .github/scripts/audit-credential-debug.sh --list   # print current credential types
#   UPDATE_BASELINE=1 .github/scripts/audit-credential-debug.sh   # rewrite baseline after review
#
# SCAN_ROOT may be overridden to point at a fixture tree (see test-audit-credential-debug.sh).

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

SCAN_ROOT="${SCAN_ROOT:-crates}"

# Field names that hold a live credential in plaintext. Deliberately NOT the bare word `token`:
# `token_count`, `token_hash`, `id_token_id` and friends are not credentials, and a pattern that
# matches them produces a baseline nobody reads. Hashes/digests are excluded for the same reason —
# a stored hash is not the secret.
CREDENTIAL_FIELDS='access_token|refresh_token|id_token|client_secret|hmac_secret|api_key|private_key|password|plaintext|[a-z_]*_secret'

# Reviewed baseline: <file>\t<type> for each credential-bearing type deriving Debug. Each was
# reviewed for whether its Debug can reach a log sink. NONE of these are known leak paths today —
# but they are the population from which a leak would come, so the set is frozen rather than
# ignored. A new entry means a new type whose credential could be formatted into a log.
#
# REVIEWED 2026-07-20 (T4 agent-half CI guard sweep) — initial baseline, 7 entries; two were
# retired the same day by the fixes noted below, leaving 5:
#   temper-client/auth.rs TokenResponse        — same shape, client-side; private to the module.
#   temper-core/types/machine.rs IssuedMachineCredential — one-time client_secret, returned once.
#   temper-services/auth/secret.rs MintedSecret — plaintext + hash pair from mint_secret().
#   temper-core/types/config.rs LlmConfig      — api_key field; Debug'd in config-dump paths.
#   temper-cli/src/saml/mod.rs SamlProvisionConfig — reconcile_secret + signing_key_pem, CLI-side.
#
#   RETIRED 2026-07-20 (T4 security-audit fixes) — two entries left this baseline by being FIXED,
#   which is the outcome the guard exists to produce:
#     temper-services/config.rs ApiConfig — now hand-writes a redacting `Debug`. It had derived one
#       over `internal_reconcile_secret`, `embed_dispatch_secret` and `slack_mint_secret`, the keys
#       behind all three signature gates audit-signature-secrets.sh keeps separate; anything that
#       formatted it with `{:?}` printed all three. The irony the initial baseline recorded — that
#       `SlackLinkConfig`'s hand-written impl cites "wherever this or THE ENCLOSING `ApiConfig` is
#       formatted" as its reason, while ApiConfig itself was left derived — is now resolved.
#     temper-auth/token.rs TokenResponse — now hand-writes a redacting `Debug`. It is bound on the
#       Slack mint path holding BOTH the rotated refresh token and an act-as-the-human access token.
#   Note temper-client/auth.rs TokenResponse is a DIFFERENT type of the same name and remains
#   baselined: it is private to its module and client-side.
#
# The redacting hand-written impls (MintOutcome, NewGrant, SlackMintResponse, VaultKey,
# BrokerToken, SlackLinkConfig, and now ApiConfig and temper-auth's TokenResponse) correctly do NOT
# appear here — they do not derive Debug. That
# asymmetry is the guard's real signal: the convention exists, and this is who is outside it.
read -r -d '' BASELINE <<'EOF' || true
crates/temper-cli/src/saml/mod.rs	SamlProvisionConfig
crates/temper-client/src/auth.rs	TokenResponse
crates/temper-core/src/types/config.rs	LlmConfig
crates/temper-core/src/types/machine.rs	IssuedMachineCredential
crates/temper-services/src/auth/secret.rs	MintedSecret
EOF

# Emit <file>\t<type> for every struct/enum that (1) derives Debug and (2) has a credential field.
#
# Item extent is tracked by BRACE DEPTH, not by a column-0 `}`. That distinction is not cosmetic: a
# struct declared inside a `mod` closes on an INDENTED brace, so a column-0 rule never ends it and
# the scan keeps attributing later structs' fields to it. The first version of this script did
# exactly that and reported `DisconnectEventRow` — a type whose four fields are payload/references/
# producing_anchor_* and contain no credential at all — as a credential type. A guard that
# misattributes fields across type boundaries reports types that are fine and would, in the
# mirror-image case, stay silent about one that is not.
#
# `#[cfg(test)]` modules are excised by the same depth tracking.
extract() {
  find "$SCAN_ROOT" -name '*.rs' -path '*/src/*' -print0 2>/dev/null \
  | xargs -0 awk '
    function braces(s,   i, c, n) {
      n = 0
      for (i = 1; i <= length(s); i++) { c = substr(s, i, 1); if (c == "{") n++; else if (c == "}") n-- }
      return n
    }
    # An attribute is closed when its brackets balance. Needed because rustfmt wraps a long
    # derive list across lines — the continuation lines do not start with `#[`, so a rule keyed
    # on that prefix alone drops them and a wrapped `#[derive(\n Debug,\n)]` reads as no Debug
    # at all. That is the silent-miss direction, which is the one that matters here.
    function attr_closed(s,   i, c, n) {
      n = 0
      for (i = 1; i <= length(s); i++) { c = substr(s, i, 1); if (c == "[") n++; else if (c == "]") n-- }
      return (n <= 0)
    }
    FNR==1 { skip=0; skipdepth=0; pending=0; derives=""; ty=""; inty=0; depth=0; attropen=0 }

    # --- #[cfg(test)] module excision, by depth ---
    /^[[:space:]]*#\[cfg\(test\)\]/ { pending=1; next }
    pending && /[[:space:]]*mod[[:space:]]+[A-Za-z_]/ {
      pending=0; skip=1; skipdepth=braces($0)
      if (skipdepth <= 0) skip=0
      next
    }
    pending { pending=0 }
    skip { skipdepth += braces($0); if (skipdepth <= 0) skip=0; next }

    # Accumulate attribute lines so a multi-line #[derive(...)] is seen whole.
    attropen { derives = derives $0; if (attr_closed(derives)) attropen = 0; next }
    /^[[:space:]]*#\[/ { derives = derives $0; attropen = (attr_closed(derives) ? 0 : 1); next }

    /^[[:space:]]*(pub[[:space:]]*(\([^)]*\))?[[:space:]]*)?(struct|enum)[[:space:]]+[A-Za-z_]/ {
      ty = $0
      sub(/.*(struct|enum)[[:space:]]+/, "", ty)
      sub(/[^A-Za-z0-9_].*/, "", ty)
      has_debug = (index(derives, "Debug") > 0)
      derives = ""
      depth = braces($0)
      inty = (depth > 0)
      hit = 0
      next
    }

    inty {
      line = $0
      sub(/[[:space:]]*\/\/.*/, "", line)
      # Field position only: `name: Type` — not a method body or a doc mention.
      if (!hit && match(line, /^[[:space:]]*(pub[[:space:]]*(\([^)]*\))?[[:space:]]*)?[a-z_][a-z0-9_]*[[:space:]]*:/)) {
        fld = line
        sub(/[[:space:]]*:.*/, "", fld)
        sub(/^[[:space:]]*(pub[[:space:]]*(\([^)]*\))?[[:space:]]*)?/, "", fld)
        if (fld ~ CRED && has_debug) { print FILENAME"\t"ty; hit = 1 }
      }
      depth += braces($0)
      if (depth <= 0) inty = 0
      next
    }
    { derives = "" }
  ' CRED="^($CREDENTIAL_FIELDS)$" | sort -u
}

CURRENT="$(extract)"

if [[ "${1:-}" == "--list" ]]; then
  printf '%s\n' "$CURRENT"
  exit 0
fi
if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  printf '%s\n' "$CURRENT"
  echo "^^^ copy into BASELINE after confirming each new type's Debug cannot reach a log sink" >&2
  echo "    (or giving it a redacting hand-written impl, as MintOutcome/NewGrant have)." >&2
  exit 0
fi

NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -u)"
if diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$CURRENT") >/tmp/credential-debug.diff 2>&1; then
  echo "audit-credential-debug: OK — credential-bearing types deriving Debug unchanged ($(printf '%s\n' "$CURRENT" | grep -c .) types)."
  exit 0
fi

cat >&2 <<'MSG'
audit-credential-debug: FAIL — the set of credential-bearing types deriving `Debug` changed.

A derived `Debug` prints the credential verbatim. One `?value` or `{:?}` on such a type inside a
`tracing::` macro writes a live token into the platform log — retained, indexed, and not revoked
by fixing the code afterwards. Before accepting a NEW entry, confirm:
  1. This type's `Debug` cannot reach a log sink, OR
  2. It hand-writes a redacting `Debug`, as MintOutcome / NewGrant / SlackMintResponse do.

diff (baseline -> current):
MSG
cat /tmp/credential-debug.diff >&2
echo >&2
echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-credential-debug.sh" >&2
exit 1
