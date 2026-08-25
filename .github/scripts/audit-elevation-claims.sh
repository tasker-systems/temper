#!/usr/bin/env bash
# audit-elevation-claims.sh — bind a surface's elevation claim to the gate it describes, and red
# CI when either end moves.
#
# WHY THIS EXISTS
# ---------------
# "Every prose claim about who may perform an act agrees with the predicate that act actually
# enforces" has been MEASURED three times (2026-08-01 read surface, 2026-08-05 write/elevation
# across 116 claims, 2026-08-23 re-verification) and ENFORCED zero times. Every prior artifact is a
# photograph. Two things the photographs establish, and this guard is shaped by both:
#
#   * M2 — a claim BORN wrong. `temper admin subscription` shipped 2026-08-19 calling itself
#     "operator-only" over a gate that admits an owner-or-maintainer: three days after the register
#     named that exact conflation, eighteen days after a 116-claim sweep of the same surface. No
#     sweep prevents the NEXT one.
#   * M1 — the gate moved underneath prose that was right when written. Archaeology over 3077
#     commits dated every over-claim against the commit that widened its gate: M1 dominates 6 of 8,
#     and dominates by consequence — both instances that shipped downstream are M1. The widening
#     commits label themselves ("two-sided bind/unbind gate", "democratize genesis", "relax
#     context-share RBAC", "team-owner registration authority"). Each touched the gate and left
#     every surface string describing it untouched.
#
# A guard that reads only strings cannot see M1 at all: when a gate widens, the string does not
# change, the baseline does not change, and CI stays green — which is precisely how six of eight got
# in. So a baseline of claims alone would ratchet the birth case and be blind to the dominant one.
#
# Hence TWO triggers over ONE baseline:
#
#   trigger 1  the CLAIM SURFACE changed — a new or moved elevation claim is not in the reviewed
#              baseline. Catches M2 at the commit that introduces it.
#   trigger 2  a GATE FINGERPRINT changed — an authority's arms or a gate fn's body moved. Reds
#              every claim BOUND to that gate, by name, so the widening commit is the one that
#              re-reads the prose. Catches M1 at the commit that causes it.
#
# WHY NOT THE TWO OPTIONS THE DESIGN TASK POSED
# ---------------------------------------------
# "Tests grow an assertion about the prose": the green e2e cases (`bind_cogmap_e2e.rs` case (c),
# `context_share_e2e.rs` case (c)) have been continuously proving over-claims false while they
# ship, so the behavior is already covered — what is missing is the LINK from prose to gate. A
# behavioral test is the wrong place to assert about a string it does not import, it reaches only
# acts that have an e2e test, and it is blind to the two classes that actually shipped: utoipa TAG
# descriptions and `//!` module rustdoc. It also needs `test-db`, so it does not run in cheap CI.
#
# "A lint reads gates and strings independently": a lint cannot evaluate a gate — that needs type
# and flow analysis — so it degenerates into a baseline, i.e. trigger 1 alone, i.e. blind to M1.
#
# The mechanism is neither: it is a BINDING (claim file -> gate) plus the two triggers above.
#
# WHAT IT DOES NOT DO
# -------------------
# It does not decide whether a claim is TRUE. It cannot; that is a reading, and a reviewer does it.
# What it guarantees is that the reading HAPPENS — at the commit that introduces a claim, and again
# at the commit that moves a gate a reviewed claim is bound to.
#
# Unbound claim files (gate column `-`) are claims nobody has yet bound to a gate. They still
# ratchet under trigger 1; they are simply outside trigger 2. The OK line PRINTS that count rather
# than letting silence read as coverage.
#
# USAGE
#   .github/scripts/audit-elevation-claims.sh            # verify (CI mode)
#   .github/scripts/audit-elevation-claims.sh --list     # print current claims + gate fingerprints
#   UPDATE_BASELINE=1 .github/scripts/audit-elevation-claims.sh   # emit a new baseline after review

set -euo pipefail
cd "${REPO_ROOT_OVERRIDE:-$(git rev-parse --show-toplevel)}"

# ── the claim surface ────────────────────────────────────────────────────────────────────────
#
# Elevation vocabulary. Deliberately broad: a false positive costs a reviewer one line in the
# baseline, a false negative costs what 2026-08-19 cost.
VOCAB='admin-gated|admin-only|admin only|operator-only|operator only|system-admin|system admin|instance admin|instance-admin|requires admin|require admin|superuser|elevated privilege'

# Only text a READER meets: rustdoc (`///`, `//!`) and utoipa/MCP `description = "…"` literals.
# Plain `//` implementation comments are excluded — they are notes to the next contributor, not a
# claim the system makes about itself. `crates/temper-services/src/authz/` is excluded as a claim
# surface ON PURPOSE: those files ARE the gate, so their prose is the thing claims are checked
# AGAINST. They enter this guard through the fingerprints below instead.
claim_files() {
  ls crates/temper-cli/src/cli.rs \
     crates/temper-cli/src/commands/*.rs \
     crates/temper-api/src/openapi.rs \
     crates/temper-api/src/handlers/*.rs \
     crates/temper-mcp/src/service.rs \
     crates/temper-mcp/src/tools/*.rs \
     crates/temper-services/src/services/*.rs 2>/dev/null
}

current_claims() {
  # shellcheck disable=SC2046
  grep -nE "$VOCAB" $(claim_files) 2>/dev/null \
    | grep -E ':[0-9]+:[[:space:]]*(///|//!)|description = "' \
    | grep -vE ':[0-9]+:[[:space:]]*//[^/!]' \
    | awk -F: '{print $1}' \
    | sort | uniq -c \
    | awk '{printf "claim %s %s\n", $2, $1}' \
    | sort \
    || true
}

# ── the gates ────────────────────────────────────────────────────────────────────────────────
#
# A fingerprint is taken over CODE ONLY — every comment and blank line is stripped first. That is
# what makes trigger 2 mean "the gate moved" and not "someone improved a doc comment": rewriting
# the prose above a gate must NOT red the claims bound to it, or the guard trains people to
# re-baseline reflexively and stops being read.
fingerprint() {
  sed -E 's://!.*$::; s:///.*$::; s://.*$::' \
    | sed -E 's/[[:space:]]+$//' \
    | grep -v '^[[:space:]]*$' \
    | shasum -a 256 | cut -c1-12
}

# A gate is not always one file. `MachineAuthority`'s arms live in `services/machine_authz.rs` and
# `GrantAuthority`'s in `access_service.rs`, while their resolvers live in `authz/` — so a
# fingerprint over the authz module alone would cover the resolver and MISS a new admitting arm,
# which is the widening shape this guard exists to catch. Each gate therefore names its PARTS:
#
#   file:<path>              the whole module, comments stripped
#   block:enum:<N>:<path>    just enum N { … }, brace-counted
#   block:fn:<N>:<path>      just fn N(…) { … }, brace-counted
#
# Blocks rather than whole files for the split parts, so unrelated edits to a large service module
# do not red claims bound to the gate that happens to live in it.
#
# The `fn` gates are named here because each was READ, not because it matched a pattern:
# `require_cogmap_write_admin` is the one whose no-op branch made four cogmap surfaces false, and
# `require_manage_on_team` is the bar three `admin` subcommands were said to exceed.
read -r -d '' GATES <<'EOF' || true
audit_gate|file:crates/temper-services/src/authz/audit_gate.rs
connection|file:crates/temper-services/src/authz/connection.rs
context_admin|file:crates/temper-services/src/authz/context_admin.rs
grant|file:crates/temper-services/src/authz/grant.rs;block:enum:GrantAuthority:crates/temper-services/src/services/access_service.rs
machine|file:crates/temper-services/src/authz/machine.rs;block:enum:MachineAuthority:crates/temper-services/src/services/machine_authz.rs
read_gates|file:crates/temper-services/src/authz/read_gates.rs
subscription|file:crates/temper-services/src/authz/subscription.rs
two_sided|file:crates/temper-services/src/authz/two_sided.rs
require_cogmap_write_admin|block:fn:require_cogmap_write_admin:crates/temper-services/src/services/access_service.rs
is_system_admin|block:fn:is_system_admin:crates/temper-services/src/services/access_service.rs
require_manage_on_team|block:fn:require_manage_on_team:crates/temper-services/src/services/team_service.rs
can_manage|block:fn:can_manage:crates/temper-services/src/services/team_service.rs
EOF

# Extract one `fn`/`enum` block by brace counting from its declaration. Anchored on the declaring
# keyword so an identically-named call site or a `use` cannot start the capture.
extract_block() {
  local kind="$1" name="$2" file="$3"
  awk -v kind="$kind" -v nm="$name" '
    BEGIN { anchor = "(^|[^a-zA-Z0-9_])" kind "[[:space:]]+" nm "[[:space:]]*[({<]" }
    !inside && $0 ~ anchor { inside = 1 }
    inside {
      print
      n = gsub(/\{/, "{"); m = gsub(/\}/, "}")
      depth += n - m
      if (seen_open || n > 0) { seen_open = 1; if (depth <= 0) exit }
    }
  ' "$file"
}

# Every part of one gate, concatenated in declared order, then hashed once. A gate has ONE
# fingerprint however many files it spans.
gate_material() {
  local parts="$1" part
  while IFS= read -r part; do
    [[ -z "$part" ]] && continue
    case "$part" in
      file:*)  cat "${part#file:}" ;;
      block:*) local rest="${part#block:}"
               extract_block "${rest%%:*}" "$(cut -d: -f2 <<< "$rest")" "$(cut -d: -f3- <<< "$rest")" ;;
    esac
  done <<< "$(tr ';' '\n' <<< "$parts")"
}

current_gates() {
  local spec name parts
  while IFS= read -r spec; do
    [[ -z "$spec" ]] && continue
    name="${spec%%|*}"; parts="${spec#*|}"
    printf 'gate %s %s\n' "$name" "$(gate_material "$parts" | fingerprint)"
  done <<< "$GATES"
}

# ── the reviewed baseline ────────────────────────────────────────────────────────────────────
#
# `claim <path> <count> <gate[,gate…]>` — the gate column is the BINDING, and it is the whole point
# of the guard. `-` means "nobody has bound this file's claims to a gate yet": it still ratchets
# under trigger 1, and it is counted out loud on the OK line so the gap cannot read as coverage.
#
# The bindings below were read against the gate on 2026-08-23, for the files the elevation review
# actually opened. Everything else is honestly `-`.
#
# `gate <name> <fingerprint>` — code-only hash; see `fingerprint`.
read -r -d '' BASELINE <<'EOF' || true
claim crates/temper-api/src/handlers/access.rs 11 is_system_admin
claim crates/temper-api/src/handlers/cognitive_maps.rs 4 require_cogmap_write_admin
claim crates/temper-api/src/handlers/connections.rs 1 connection
claim crates/temper-api/src/handlers/slack_disconnect.rs 1 -
claim crates/temper-api/src/handlers/teams.rs 1 -
claim crates/temper-cli/src/cli.rs 12 -
claim crates/temper-cli/src/commands/admin_connection.rs 2 connection
claim crates/temper-cli/src/commands/admin_machine.rs 3 machine
claim crates/temper-cli/src/commands/admin_saml.rs 1 -
claim crates/temper-cli/src/commands/admin_slack.rs 1 -
claim crates/temper-cli/src/commands/admin_subscription.rs 2 subscription
claim crates/temper-cli/src/commands/admin.rs 3 -
claim crates/temper-cli/src/commands/cogmap.rs 3 require_cogmap_write_admin
claim crates/temper-cli/src/commands/context_cmd.rs 2 context_admin
claim crates/temper-mcp/src/service.rs 2 -
claim crates/temper-mcp/src/tools/cognitive_maps.rs 1 require_cogmap_write_admin
claim crates/temper-mcp/src/tools/contexts.rs 2 context_admin,two_sided
claim crates/temper-services/src/services/access_service.rs 5 is_system_admin
claim crates/temper-services/src/services/cogmap_service.rs 2 require_cogmap_write_admin
claim crates/temper-services/src/services/connection_service.rs 9 connection
claim crates/temper-services/src/services/context_service.rs 5 context_admin
claim crates/temper-services/src/services/machine_authz.rs 7 machine
claim crates/temper-services/src/services/machine_client_service.rs 4 machine
claim crates/temper-services/src/services/machine_registration_service.rs 5 machine
claim crates/temper-services/src/services/slack_disconnect_service.rs 2 -
claim crates/temper-services/src/services/subscription_service.rs 6 subscription
claim crates/temper-services/src/services/subscription_test_support.rs 1 subscription
claim crates/temper-services/src/services/team_service.rs 2 require_manage_on_team,can_manage
gate audit_gate 2765b1223b30
gate connection 815137c3936d
gate context_admin 14529ac51ace
gate grant 3cfec3045c9a
gate machine 9257313b8605
gate read_gates f619f1101959
gate subscription b5f813e35ccc
gate two_sided a44fcf6881bf
gate require_cogmap_write_admin ac8abe5dae96
gate is_system_admin 1f8215393b50
gate require_manage_on_team 9dc74ce6502d
gate can_manage b48bac6a803e
EOF

CURRENT="$(current_claims; current_gates)"

if [[ "${1:-}" == "--list" ]]; then
  echo "$CURRENT"
  exit 0
fi

if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  # Carry each reviewed binding forward by path, so re-baselining after a gate widening does not
  # silently blank the gate column — the binding is the expensive part and must survive the cheap
  # operation.
  while IFS= read -r line; do
    case "$line" in
      claim\ *)
        path="$(awk '{print $2}' <<< "$line")"
        bind="$(awk -v p="$path" '$1=="claim" && $2==p {print $4}' <<< "$BASELINE")"
        printf 'claim %s %s %s\n' "$path" "$(awk '{print $3}' <<< "$line")" "${bind:--}"
        ;;
      *) printf '%s\n' "$line" ;;
    esac
  done <<< "$CURRENT"
  echo "^^^ copy into BASELINE. For every CHANGED line, read the claim against the gate it names before accepting." >&2
  exit 0
fi

NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort)"
NORM_CURRENT="$(printf '%s\n' "$CURRENT" | sort)"

# The gate column is a REVIEWER's binding, not an observation, so nothing in the repo reproduces it
# and it must not enter the diff. Project it away for the comparison and keep `$NORM_BASELINE`
# intact — trigger 2's message is built entirely out of that column.
CMP_BASELINE="$(awk '$1=="claim" {print $1, $2, $3; next} {print}' <<< "$NORM_BASELINE")"
DIFF_FILE="$(mktemp)"; trap 'rm -f "$DIFF_FILE"' EXIT

if diff <(printf '%s\n' "$CMP_BASELINE") <(printf '%s\n' "$NORM_CURRENT") > "$DIFF_FILE" 2>&1; then
  bound="$(awk '$1=="claim" && $4!="-" {n+=$3} END {print n+0}' <<< "$NORM_BASELINE")"
  unbound="$(awk '$1=="claim" && $4=="-" {n+=$3} END {print n+0}' <<< "$NORM_BASELINE")"
  gates="$(awk '$1=="gate"' <<< "$NORM_BASELINE" | wc -l | tr -d ' ')"
  echo "audit-elevation-claims: OK — $((bound + unbound)) elevation claims, $bound bound to one of $gates gates, $unbound not yet bound."
  exit 0
fi

# Which trigger fired? A changed `gate` line is M1 and deserves the louder message, because it is
# the one that names prose nobody has looked at.
CHANGED_GATES="$(grep -E '^[<>] gate ' "$DIFF_FILE" | awk '{print $3}' | sort -u || true)"

echo "audit-elevation-claims: FAIL" >&2
echo >&2

if [[ -n "$CHANGED_GATES" ]]; then
  cat >&2 <<'MSG'
TRIGGER 2 — a GATE MOVED. This is mechanism M1, which dominates 6 of 8 measured over-claims:
the prose was right when it was written and the gate changed underneath it. Every claim bound to
a changed gate below is now unverified prose. Re-read each one against the new gate IN THIS
COMMIT — that is the whole reason this fires here rather than in the next audit.

MSG
  for g in $CHANGED_GATES; do
    echo "  gate: $g" >&2
    awk -v g="$g" '$1=="claim" && $4 ~ ("(^|,)" g "(,|$)") {printf "        %s (%s claims)\n", $2, $3}' \
      <<< "$NORM_BASELINE" >&2
  done
  echo >&2
fi

if grep -qE '^[<>] claim ' "$DIFF_FILE"; then
  cat >&2 <<'MSG'
TRIGGER 1 — the CLAIM SURFACE changed. A new or moved elevation claim is not in the reviewed
baseline. Read it against the gate the act actually enforces before accepting it: `temper admin
subscription` shipped in exactly this shape on 2026-08-19, calling itself "operator-only" over an
owner-or-maintainer gate, three days after the register named that conflation.

MSG
fi

echo "diff (baseline -> current):" >&2
cat "$DIFF_FILE" >&2
echo >&2
echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-elevation-claims.sh" >&2
exit 1
