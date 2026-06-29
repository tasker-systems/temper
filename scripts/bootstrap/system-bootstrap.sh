#!/usr/bin/env bash
#
# system-bootstrap.sh — take a blank-but-stable self-hosted temper install to a usable org.
#
# This is the thin EXTERNAL applier for the org-provisioning bootstrap (Chunk 7). It is deliberately
# NOT a `temper` subcommand: it loops the now-surfaced idempotent `temper` commands (+ the one
# irreducible SQL root step) declared in an install-profile.yaml. Idempotency is inherited from the
# primitives — re-running converges rather than duplicating. There is no state backend; plan/diff
# (Terraform-like) semantics are deferred.
#
# The canonical step-by-step rationale lives in the SoP runbook this script automates:
#   docs/guides/org-bootstrap.md
#
# Usage:
#   scripts/bootstrap/system-bootstrap.sh [--profile <path>] [--run-root] [--dry-run]
#
#   --profile <path>   install-profile.yaml (default: schema-artifact/install-profile.yaml)
#   --run-root         also run the irreducible SQL root step (needs DATABASE_URL + DB creds).
#                      Omit to treat the root step as a manual prerequisite (see the runbook).
#   --dry-run          print the commands without executing them.
#
# Prerequisites:
#   - `temper` on PATH, authenticated as the first admin (TEMPER_TOKEN or `temper auth login`).
#   - `yq` (https://github.com/mikefarah/yq) to read the profile.
#   - `psql` + DATABASE_URL ONLY when --run-root is used.
#   - An `embed`-capable `temper` binary (the default install bundles it) — cogmap create/reconcile
#     embed the charter client-side.
set -euo pipefail

PROFILE="schema-artifact/install-profile.yaml"
RUN_ROOT=0
DRY_RUN=0

die()  { printf 'error: %s\n' "$*" >&2; exit 1; }
info() { printf '\033[1m==>\033[0m %s\n' "$*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --profile)  PROFILE="${2:?--profile needs a path}"; shift 2 ;;
    --run-root) RUN_ROOT=1; shift ;;
    --dry-run)  DRY_RUN=1; shift ;;
    -h|--help)  sed -n '2,30p' "$0"; exit 0 ;;
    *)          die "unknown argument: $1" ;;
  esac
done

command -v yq     >/dev/null 2>&1 || die "yq not found — install it (brew install yq)"
command -v temper >/dev/null 2>&1 || die "temper not found on PATH"
[ -f "$PROFILE" ] || die "install profile not found: $PROFILE"

# Run a command, honoring --dry-run. Captures stdout to the caller via the global REPLY.
run() {
  if [ "$DRY_RUN" -eq 1 ]; then
    printf '   (dry-run) %s\n' "$*"
    REPLY=""
    return 0
  fi
  REPLY="$("$@")"
}

# Read a scalar from the profile; empty string when null/absent.
prof() { yq -r "$1 // \"\"" "$PROFILE"; }

# ── Profile values ───────────────────────────────────────────────────────────────────────────────
INSTANCE_NAME="$(prof '.instance_name')"
GATING_TEAM="$(prof '.root.gating_team_slug')"
ACCESS_MODE="$(prof '.root.access_mode')"
FIRST_ADMIN="$(prof '.root.first_admin_profile_id')"
EVERYONE_SLUG="$(prof '.auto_join_team.slug')"
EVERYONE_NAME="$(prof '.auto_join_team.name')"
AUTO_JOIN_ROLE="$(prof '.auto_join_team.auto_join_role')"
ORG_ID="$(prof '.org_identity.id')"
GENESIS_MANIFEST="$(prof '.org_identity.genesis_manifest')"
LANDMARKS_MANIFEST="$(prof '.org_identity.landmarks_manifest')"

[ -n "$EVERYONE_SLUG" ]      || die "profile missing .auto_join_team.slug"
[ -n "$GENESIS_MANIFEST" ]   || die "profile missing .org_identity.genesis_manifest"
[ -f "$GENESIS_MANIFEST" ]   || die "genesis manifest not found: $GENESIS_MANIFEST"

# ── Phase 0 — the irreducible SQL root step (operator-with-DB-credentials) ────────────────────────
# Set gating_team_slug + access_mode and promote the first admin. The `system_access='admin'` UPDATE
# fires the auto-join trigger, which mints the admin as OWNER of the gating team — so is_system_admin
# resolves true. Mirrors `root_bootstrap_first_admin` in tests/e2e/tests/admin_surface_e2e.rs.
if [ "$RUN_ROOT" -eq 1 ]; then
  info "Phase 0 — SQL root step (gating + first admin)"
  command -v psql >/dev/null 2>&1 || die "psql not found — required for --run-root"
  [ -n "${DATABASE_URL:-}" ] || die "DATABASE_URL not set — required for --run-root"
  [ -n "$GATING_TEAM" ]  || die "profile missing .root.gating_team_slug (needed for --run-root)"
  [ -n "$FIRST_ADMIN" ] && [ "$FIRST_ADMIN" != "REPLACE-WITH-FIRST-ADMIN-PROFILE-UUID" ] \
    || die "set .root.first_admin_profile_id in the profile before --run-root"
  ACCESS_MODE_SQL="${ACCESS_MODE:-open}"
  if [ "$DRY_RUN" -eq 1 ]; then
    printf '   (dry-run) psql <DATABASE_URL> -f - <<root.sql (gating=%s mode=%s admin=%s)\n' \
      "$GATING_TEAM" "$ACCESS_MODE_SQL" "$FIRST_ADMIN"
  else
    psql "$DATABASE_URL" --set=ON_ERROR_STOP=1 \
      --set=gating="$GATING_TEAM" --set=mode="$ACCESS_MODE_SQL" --set=admin="$FIRST_ADMIN" <<'SQL'
INSERT INTO kb_teams (slug, name) VALUES (:'gating', :'gating')
  ON CONFLICT (slug) DO NOTHING;
UPDATE kb_system_settings SET gating_team_slug = :'gating', access_mode = :'mode' WHERE id = 1;
UPDATE kb_profiles SET system_access = 'admin' WHERE id = :'admin'::uuid;
SQL
  fi
else
  info "Phase 0 — SKIPPED (run the SQL root step manually first; see docs/guides/org-bootstrap.md)"
fi

# ── Phase 1 — instance settings (admin-gated, surfaced) ───────────────────────────────────────────
if [ -n "$INSTANCE_NAME" ] || [ -n "$GATING_TEAM" ] || [ -n "$ACCESS_MODE" ]; then
  info "Phase 1 — admin settings"
  settings_args=(admin settings --format json)
  [ -n "$INSTANCE_NAME" ] && settings_args+=(--instance-name "$INSTANCE_NAME")
  [ -n "$GATING_TEAM" ]   && settings_args+=(--gating-team "$GATING_TEAM")
  [ -n "$ACCESS_MODE" ]   && settings_args+=(--access-mode "$ACCESS_MODE")
  run temper "${settings_args[@]}"
fi

# ── Phase 2 — the everyone auto-join team (admin-gated --auto-join-role) ───────────────────────────
info "Phase 2 — team create ${EVERYONE_SLUG} (auto-join ${AUTO_JOIN_ROLE:-none})"
team_args=(team create "$EVERYONE_SLUG" --format json)
[ -n "$EVERYONE_NAME" ]   && team_args+=(--name "$EVERYONE_NAME")
[ -n "$AUTO_JOIN_ROLE" ]  && team_args+=(--auto-join-role "$AUTO_JOIN_ROLE")
run temper "${team_args[@]}"

# ── Phase 3 — genesis the org-identity cogmap ─────────────────────────────────────────────────────
info "Phase 3 — cogmap create (genesis) from ${GENESIS_MANIFEST}"
create_args=(cogmap create --manifest "$GENESIS_MANIFEST" --format json)
[ -n "$ORG_ID" ] && create_args+=(--id "$ORG_ID")
run temper "${create_args[@]}"
if [ "$DRY_RUN" -eq 1 ]; then
  COGMAP_ID="${ORG_ID:-<minted-at-apply-time>}"
elif [ -n "$ORG_ID" ]; then
  COGMAP_ID="$ORG_ID"
else
  COGMAP_ID="$(printf '%s' "$REPLY" | yq -r '.cogmap_id')"
  [ -n "$COGMAP_ID" ] && [ "$COGMAP_ID" != "null" ] || die "could not read cogmap_id from create output"
  info "minted org-identity cogmap id: ${COGMAP_ID} — pin it in the profile for idempotent re-runs"
fi

# ── Phase 4 — reconcile the org-identity landmark content (idempotent) ────────────────────────────
if [ -n "$LANDMARKS_MANIFEST" ] && [ -f "$LANDMARKS_MANIFEST" ]; then
  info "Phase 4 — cogmap reconcile ${COGMAP_ID} from ${LANDMARKS_MANIFEST}"
  run temper cogmap reconcile "$COGMAP_ID" --manifest "$LANDMARKS_MANIFEST" --format json
elif [ -n "$LANDMARKS_MANIFEST" ]; then
  die "landmarks manifest not found: $LANDMARKS_MANIFEST"
fi

# ── Phase 5 — bind the cogmap to its audience teams ───────────────────────────────────────────────
bind_count="$(yq -r '.org_identity.bind_teams | length' "$PROFILE")"
if [ "$bind_count" != "0" ] && [ "$bind_count" != "null" ]; then
  i=0
  while [ "$i" -lt "$bind_count" ]; do
    team="$(yq -r ".org_identity.bind_teams[$i]" "$PROFILE")"
    info "Phase 5 — cogmap bind ${COGMAP_ID} +${team}"
    run temper cogmap bind "$COGMAP_ID" "+${team}" --format json
    i=$((i + 1))
  done
fi

info "Bootstrap complete — org is usable: everyone-team provisioned, org-identity map born + bound."
