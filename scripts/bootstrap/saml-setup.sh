#!/usr/bin/env bash
#
# saml-setup.sh — the SAML-only half of the enterprise-install applier.
#
# Owns timeline steps 3, 6, 11, 12 of docs/guides/enterprise-install.md: provision (generate the AS
# signing key + reconcile secret, emit the env bundle + kb_saml_idp SQL), apply the kb_saml_idp row,
# map IdP groups to teams, and verify. It is the SAML sibling of system-bootstrap.sh (the auth-
# agnostic db+admin spine) — kept separate so system-bootstrap.sh stays usable for Auth0/OAuth.
#
# Idempotency + emit-by-default are INHERITED from `temper admin saml *`. DB-touching steps run only
# with --apply-db + DATABASE_URL; default is inert emit (safe to run anytime).
#
# Canonical rationale: docs/guides/enterprise-install.md + docs/guides/self-hosting-saml.md
#
# Usage:
#   scripts/bootstrap/saml-setup.sh [--profile <path>] [--apply-db] [--dry-run]
set -euo pipefail

PROFILE="schema-artifact/saml-profile.yaml"
APPLY_DB=0
DRY_RUN=0

die()  { printf 'error: %s\n' "$*" >&2; exit 1; }
info() { printf '\033[1m==>\033[0m %s\n' "$*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --profile)  PROFILE="${2:?--profile needs a path}"; shift 2 ;;
    --apply-db) APPLY_DB=1; shift ;;
    --dry-run)  DRY_RUN=1; shift ;;
    -h|--help)  sed -n '2,20p' "$0"; exit 0 ;;
    *)          die "unknown argument: $1" ;;
  esac
done

command -v yq     >/dev/null 2>&1 || die "yq not found — install it (brew install yq)"
command -v temper >/dev/null 2>&1 || die "temper not found on PATH"
[ -f "$PROFILE" ] || die "saml profile not found: $PROFILE"

run() {
  if [ "$DRY_RUN" -eq 1 ]; then printf '   (dry-run) %s\n' "$*"; REPLY=""; return 0; fi
  REPLY="$("$@")"
}
prof() { yq -r "$1 // \"\"" "$PROFILE"; }

# ── Profile values ───────────────────────────────────────────────────────────────────────────────
IDP_KEY="$(prof '.idp.key')";           [ -n "$IDP_KEY" ] || die "profile missing .idp.key"
INSTANCE_URL="$(prof '.idp.instance_url')"; [ -n "$INSTANCE_URL" ] || die "profile missing .idp.instance_url"
API_ORIGIN="$(prof '.idp.api_origin')"
CERT_FILE="$(prof '.idp.cert_file')"
SSO_URL="$(prof '.idp.sso_url')"
ENTITY_ID="$(prof '.idp.entity_id')"
NAMEID="$(prof '.idp.nameid_format')"
EMAIL_ATTR="$(prof '.idp.email_attr')"
STABLE_ID_ATTR="$(prof '.idp.stable_id_attr')"
GROUPS_ATTR="$(prof '.idp.groups_attr')"
KID="$(prof '.idp.kid')"
ENV_OUT="$(prof '.env_out')"
SQL_OUT="$(prof '.sql_out')"

# ── Step 3 — provision (emit env + hold kb_saml_idp SQL) ───────────────────────────────────────────
info "Step 3 — provision (emit env bundle → ${ENV_OUT}, kb_saml_idp SQL → ${SQL_OUT})"
prov_args=(admin saml provision --no-interactive
  --instance-url "$INSTANCE_URL"
  --idp-key "$IDP_KEY"
  --idp-cert-file "$CERT_FILE"
  --idp-sso-url "$SSO_URL"
  --idp-entity-id "$ENTITY_ID"
  --nameid-format "$NAMEID"
  --email-attr "$EMAIL_ATTR"
  --stable-id-attr "$STABLE_ID_ATTR"
  --env-out "$ENV_OUT"
  --sql-out "$SQL_OUT")
[ -n "$API_ORIGIN" ]   && prov_args+=(--api-origin "$API_ORIGIN")
[ -n "$GROUPS_ATTR" ]  && prov_args+=(--groups-attr "$GROUPS_ATTR")
[ -n "$KID" ]          && prov_args+=(--kid "$KID")
# Repeatable client allowlist:
client_count="$(yq -r '.clients | length' "$PROFILE")"
i=0; while [ "$i" -lt "$client_count" ]; do
  prov_args+=(--client "$(yq -r ".clients[$i]" "$PROFILE")"); i=$((i + 1))
done
run temper "${prov_args[@]}"
info "  env bundle + idp SQL emitted — set the env on api+mcp (timeline step 4) before deploy."

# ── Step 6 — apply the kb_saml_idp row (needs --apply-db) ──────────────────────────────────────────
if [ "$APPLY_DB" -eq 1 ]; then
  info "Step 6 — apply kb_saml_idp row (--apply-db set)  [TODO: Task B4]"
else
  info "Step 6 — apply kb_saml_idp row (skipped — pass --apply-db)  [TODO: Task B4]"
fi

# ── Step 11 — map IdP groups to teams (run AFTER teams exist) ──────────────────────────────────────
info "Step 11 — map-group  [TODO: Task B5]"

# ── Step 12 — verify ──────────────────────────────────────────────────────────────────────────────
info "Step 12 — verify  [TODO: Task B5]"

info "saml-setup complete."
