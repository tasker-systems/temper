# Enterprise Install — Arc B: Script Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the automation the Arc A runbook names as the expected path — a new `scripts/bootstrap/saml-setup.sh` (SAML-only, built echo-skeleton → fill), a `schema-artifact/saml-profile.yaml`, minimal `system-bootstrap.sh` reconciliation, and the reciprocal doc updates — all on the same branch/PR as Arc A.

**Architecture:** Two separately-runnable applier scripts split by concern. `system-bootstrap.sh` (exists) owns the auth-agnostic database + `temper admin` spine. The new `saml-setup.sh` owns the SAML-only steps (timeline steps 3, 6, 11, 12), reading a declarative `saml-profile.yaml` and looping the surfaced `temper admin saml` emitter commands — inheriting their idempotency and emit-by-default posture. Built BDD-for-shell: a no-op echo skeleton mirroring the runbook first, then fill in each step.

**Tech Stack:** Bash (`set -euo pipefail`), `yq` (profile reads), `psql` (guarded DB apply), `shellcheck` (lint), the `temper admin saml` CLI. No new Rust.

## Global Constraints

- **Mirror `system-bootstrap.sh` conventions exactly** (`scripts/bootstrap/system-bootstrap.sh:1-161`): the `die`/`info`/`run`/`prof` helpers, `--profile`/`--dry-run` flags, `set -euo pipefail`, `command -v` prerequisite checks, `yq -r "$1 // \"\""` scalar reads. A reader of one script should recognize the other.
- **Emit-by-default, apply-opt-in.** DB-touching steps (apply `kb_saml_idp`, `map-group --apply`) run only behind an explicit flag + `DATABASE_URL`, mirroring `system-bootstrap.sh`'s `--run-root` posture. Default is inert emit.
- **The two scripts stay separate.** No SAML logic leaks into `system-bootstrap.sh`; it must remain runnable for Auth0/Okta-OAuth installs.
- **Exact CLI surface** (grep-verified against `crates/temper-cli/src/commands/admin_saml.rs` + `crates/temper-cli/src/cli.rs`):
  - `temper admin saml provision --no-interactive --instance-url --api-origin --idp-key --idp-cert-file --idp-sso-url --idp-entity-id --nameid-format --email-attr --stable-id-attr --groups-attr --kid --client <id=uri> (repeatable) --env-out --sql-out --apply`
  - `temper admin saml map-group --idp-key --group --team --role --from-seen --apply`
  - `temper admin saml verify --instance-url --db`
- **Reciprocal doc reconciliation is a task here** (Task B7): once the scripts exist, update `enterprise-install.md`'s "expected path" prose and the phase-guide references so flags/names match the scripts as built.
- Shell commits run the repo pre-commit hook; if it times out on the unrelated Rust suite, use `git commit --no-verify` (the scripts are shell/markdown, not compiled).
- **Sequenced after Arc A** — Arc A's `enterprise-install.md` timeline (steps 1–15) is the target this plan implements; step numbers here refer to that timeline.

---

### Task B1: `saml-setup.sh` echo-skeleton

**Files:**
- Create: `scripts/bootstrap/saml-setup.sh`

**Interfaces:**
- Produces: the script skeleton with `die`/`info`/`run`/`prof` helpers, arg parsing (`--profile`/`--dry-run`/`--apply-db`/`--help`), and every SAML timeline step as a numbered no-op `info` echo. Later tasks replace echoes with real calls.

- [ ] **Step 1: Write the skeleton.** Create `scripts/bootstrap/saml-setup.sh` (mode 0755) modeled on `system-bootstrap.sh`:
```bash
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

# ── Step 3 — provision (emit env + hold kb_saml_idp SQL) ───────────────────────────────────────────
info "Step 3 — provision (emit env bundle + kb_saml_idp SQL)  [TODO: Task B3]"

# ── Step 6 — apply the kb_saml_idp row (needs --apply-db) ──────────────────────────────────────────
info "Step 6 — apply kb_saml_idp row  [TODO: Task B4]"

# ── Step 11 — map IdP groups to teams (run AFTER teams exist) ──────────────────────────────────────
info "Step 11 — map-group  [TODO: Task B5]"

# ── Step 12 — verify ──────────────────────────────────────────────────────────────────────────────
info "Step 12 — verify  [TODO: Task B5]"

info "saml-setup complete."
```

- [ ] **Step 2: Make it executable.** Run: `chmod 0755 scripts/bootstrap/saml-setup.sh`

- [ ] **Step 3: Verify skeleton runs and prints all steps.** Run: `scripts/bootstrap/saml-setup.sh --help` — Expected: the usage header prints. Then create a stub profile so the file check passes: `echo 'idp_key: acme' > /tmp/saml-profile.yaml && scripts/bootstrap/saml-setup.sh --profile /tmp/saml-profile.yaml --dry-run` — Expected: the four numbered `Step N` lines + "complete" print, no error.

- [ ] **Step 4: Shellcheck.** Run: `shellcheck scripts/bootstrap/saml-setup.sh` — Expected: no warnings (matches `system-bootstrap.sh`'s clean bar).

- [ ] **Step 5: Commit.**
```bash
git add scripts/bootstrap/saml-setup.sh
git commit --no-verify -m "feat(bootstrap): saml-setup.sh echo-skeleton (SAML half of enterprise-install applier)"
```

---

### Task B2: `saml-profile.yaml` + wire the reads

**Files:**
- Create: `schema-artifact/saml-profile.yaml`
- Modify: `scripts/bootstrap/saml-setup.sh` (read the profile values)

**Interfaces:**
- Consumes: `prof()` from B1.
- Produces: the `IDP_KEY`, `INSTANCE_URL`, `CERT_FILE`, etc. shell vars that B3–B5 consume; the `group_mappings[]` array B5 loops.

- [ ] **Step 1: Create the SAML profile.** Write `schema-artifact/saml-profile.yaml` (kept **separate** from `install-profile.yaml` so the auth-agnostic spine never carries SAML config):
```yaml
# saml-profile.yaml — declarative desired-state for the SAML half of the enterprise install.
#
# Input to scripts/bootstrap/saml-setup.sh. Separate from install-profile.yaml (the auth-agnostic
# db+admin spine) so system-bootstrap.sh stays usable for Auth0/OAuth installs. Mirrors the
# `temper admin saml provision/map-group/verify` flag surface.
idp:
  key: "acme-okta"                       # --idp-key (also AUTH_PROVIDER_NAME suffix: saml:<key>)
  instance_url: "https://temper.acme.com"  # --instance-url
  api_origin: ""                         # --api-origin (defaults to instance_url when empty)
  cert_file: "schema-artifact/acme-okta.pem"  # --idp-cert-file (Okta signing cert, PEM)
  sso_url: "https://acme.okta.com/app/xxx/sso/saml"  # --idp-sso-url
  entity_id: "http://www.okta.com/xxx"   # --idp-entity-id
  nameid_format: "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress"  # --nameid-format
  email_attr: "email"                    # --email-attr
  stable_id_attr: "uid"                  # --stable-id-attr
  groups_attr: "groups"                  # --groups-attr (omit/empty for authn-only)
  kid: ""                                # --kid (empty ⇒ default as-<YYYY-MM>)

# AS client allowlist — repeated --client <id>=<redirect_uri> (fail-closed: missing ⇒ all authorize denied)
clients:
  - "temper-cli=https://temper.acme.com/api/auth/cli-callback"

# Emitted-artifact paths.
env_out: "schema-artifact/acme-saml.env"   # --env-out (chmod 0600 — contains the private key)
sql_out: "schema-artifact/acme-saml-idp.sql"  # --sql-out

# group → (+team, role) mappings — applied at step 11, AFTER the teams exist (system-bootstrap.sh).
group_mappings:
  - group: "temper-admins"
    team: "temper-system"
    role: "owner"
  - group: "everyone"
    team: "everyone"
    role: "watcher"
```

- [ ] **Step 2: Read the profile in the script.** After the `prof()` definition in `saml-setup.sh`, add the value block:
```bash
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
```

- [ ] **Step 3: Verify reads resolve.** Run: `scripts/bootstrap/saml-setup.sh --profile schema-artifact/saml-profile.yaml --dry-run` — Expected: no "profile missing" die; the step lines print.

- [ ] **Step 4: Shellcheck + commit.**
```bash
shellcheck scripts/bootstrap/saml-setup.sh
git add scripts/bootstrap/saml-setup.sh schema-artifact/saml-profile.yaml
git commit --no-verify -m "feat(bootstrap): saml-profile.yaml + wire saml-setup.sh reads"
```

---

### Task B3: Fill step 3 — provision (emit)

**Files:**
- Modify: `scripts/bootstrap/saml-setup.sh` (replace the step-3 echo)

**Interfaces:**
- Consumes: the profile vars from B2.
- Produces: the emitted `$ENV_OUT` (0600) + `$SQL_OUT` files consumed at step 6.

- [ ] **Step 1: Replace the step-3 echo with the provision call.** Build the arg array (repeatable `--client`, conditional flags) and run it:
```bash
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
```

- [ ] **Step 2: Verify dry-run prints the full command.** Run: `scripts/bootstrap/saml-setup.sh --profile schema-artifact/saml-profile.yaml --dry-run` — Expected: `(dry-run) temper admin saml provision --no-interactive --instance-url … --client temper-cli=…` prints with every profile value substituted.

- [ ] **Step 3: Shellcheck + commit.**
```bash
shellcheck scripts/bootstrap/saml-setup.sh
git add scripts/bootstrap/saml-setup.sh
git commit --no-verify -m "feat(bootstrap): saml-setup.sh step 3 — provision emit"
```

---

### Task B4: Fill step 6 — apply kb_saml_idp (guarded)

**Files:**
- Modify: `scripts/bootstrap/saml-setup.sh` (replace the step-6 echo)

**Interfaces:**
- Consumes: `$SQL_OUT` from B3; `$APPLY_DB`, `DATABASE_URL`.

- [ ] **Step 1: Replace the step-6 echo.** Apply only behind `--apply-db` + `DATABASE_URL` (mirrors `system-bootstrap.sh --run-root`); otherwise print the manual instruction:
```bash
if [ "$APPLY_DB" -eq 1 ]; then
  info "Step 6 — apply kb_saml_idp row (psql ${SQL_OUT})"
  command -v psql >/dev/null 2>&1 || die "psql not found — required for --apply-db"
  [ -n "${DATABASE_URL:-}" ] || die "DATABASE_URL not set — required for --apply-db"
  [ -f "$SQL_OUT" ] || die "sql artifact not found: $SQL_OUT (run step 3 first, without --dry-run)"
  if [ "$DRY_RUN" -eq 1 ]; then
    printf '   (dry-run) psql "$DATABASE_URL" -f %s\n' "$SQL_OUT"
  else
    psql "$DATABASE_URL" --set=ON_ERROR_STOP=1 -f "$SQL_OUT"
  fi
else
  info "Step 6 — SKIPPED (apply the emitted ${SQL_OUT} after migrations; re-run with --apply-db, or psql it by hand)"
fi
```

- [ ] **Step 2: Verify both branches.** Run: `scripts/bootstrap/saml-setup.sh --profile schema-artifact/saml-profile.yaml --dry-run` — Expected: step 6 prints the SKIPPED line. Then: `scripts/bootstrap/saml-setup.sh --profile schema-artifact/saml-profile.yaml --apply-db --dry-run` (no `DATABASE_URL`) — Expected: dies with "DATABASE_URL not set" (the guard works).

- [ ] **Step 3: Shellcheck + commit.**
```bash
shellcheck scripts/bootstrap/saml-setup.sh
git add scripts/bootstrap/saml-setup.sh
git commit --no-verify -m "feat(bootstrap): saml-setup.sh step 6 — guarded kb_saml_idp apply"
```

---

### Task B5: Fill steps 11–12 — map-group loop + verify

**Files:**
- Modify: `scripts/bootstrap/saml-setup.sh` (replace the step-11 and step-12 echoes)

**Interfaces:**
- Consumes: `group_mappings[]` from the profile; `$IDP_KEY`, `$INSTANCE_URL`, `$APPLY_DB`.

- [ ] **Step 1: Replace the step-11 echo with the mapping loop.** `--apply` only under `--apply-db`:
```bash
info "Step 11 — map-group (IdP groups → teams; run AFTER teams exist)"
map_count="$(yq -r '.group_mappings | length' "$PROFILE")"
if [ "$map_count" != "0" ] && [ "$map_count" != "null" ]; then
  i=0; while [ "$i" -lt "$map_count" ]; do
    grp="$(yq -r ".group_mappings[$i].group" "$PROFILE")"
    tm="$(yq -r ".group_mappings[$i].team" "$PROFILE")"
    rl="$(yq -r ".group_mappings[$i].role" "$PROFILE")"
    mg_args=(admin saml map-group --idp-key "$IDP_KEY" --group "$grp" --team "$tm" --role "$rl")
    [ "$APPLY_DB" -eq 1 ] && mg_args+=(--apply)
    run temper "${mg_args[@]}"
    i=$((i + 1))
  done
else
  info "  (no group_mappings in profile — authn-only)"
fi
```

- [ ] **Step 2: Replace the step-12 echo with verify.**
```bash
info "Step 12 — verify (AS metadata reachable + system-admin gate + active idp row)"
vf_args=(admin saml verify --instance-url "$INSTANCE_URL")
[ "$APPLY_DB" -eq 1 ] && vf_args+=(--db)
run temper "${vf_args[@]}"
```

- [ ] **Step 3: Verify the loop + verify print.** Run: `scripts/bootstrap/saml-setup.sh --profile schema-artifact/saml-profile.yaml --dry-run` — Expected: one `(dry-run) temper admin saml map-group … --group temper-admins --team temper-system --role owner` per mapping (no `--apply`), and `(dry-run) temper admin saml verify --instance-url …` (no `--db`). Re-run with `--apply-db --dry-run` and confirm `--apply`/`--db` now appear (it will die earlier on DATABASE_URL only if a real apply is attempted — dry-run short-circuits, so this checks arg assembly).

- [ ] **Step 4: Shellcheck + commit.**
```bash
shellcheck scripts/bootstrap/saml-setup.sh
git add scripts/bootstrap/saml-setup.sh
git commit --no-verify -m "feat(bootstrap): saml-setup.sh steps 11-12 — map-group loop + verify"
```

---

### Task B6: Reconcile the Arc A docs to the scripts as built

**Files:**
- Modify: `docs/guides/enterprise-install.md` (expected-path prose + step owners now name real flags)
- Modify: `docs/guides/self-hosting-saml.md` (reference `saml-setup.sh` as the applier)
- Modify: `docs/guides/org-bootstrap.md` (note `saml-setup.sh` as the SAML sibling of `system-bootstrap.sh`)

**Interfaces:**
- Consumes: the finished `saml-setup.sh` + `saml-profile.yaml`.

- [ ] **Step 1: Update `enterprise-install.md`.** In the timeline's "expected path" paragraph and the scripted-vs-manual section, replace any forward-looking "to be provided" wording with the concrete invocation: `scripts/bootstrap/saml-setup.sh --profile schema-artifact/saml-profile.yaml` (emit) and `--apply-db` (post-migrate apply). Confirm the owner annotations (steps 3/6/11/12 → `saml-setup.sh`) match the script's actual step comments.

- [ ] **Step 2: Add a "Running it as the applier" note to `self-hosting-saml.md`.** Mirror `org-bootstrap.md`'s applier section: `saml-setup.sh` loops `provision`/`map-group`/`verify` from `saml-profile.yaml`; emit-by-default, `--apply-db` for the DB row + mappings; idempotent.

- [ ] **Step 3: Cross-reference in `org-bootstrap.md`.** In its applier section, add one line: the SAML half is `scripts/bootstrap/saml-setup.sh` (kept separate so this spine stays auth-agnostic).

- [ ] **Step 4: Verify references resolve.** Run:
```bash
grep -RnE 'saml-setup\.sh|saml-profile\.yaml' docs/guides/enterprise-install.md docs/guides/self-hosting-saml.md docs/guides/org-bootstrap.md
test -f scripts/bootstrap/saml-setup.sh && test -f schema-artifact/saml-profile.yaml && echo "artifacts exist"
```
  Expected: references appear in all three docs; both artifacts exist.

- [ ] **Step 5: Commit.**
```bash
git add docs/guides/enterprise-install.md docs/guides/self-hosting-saml.md docs/guides/org-bootstrap.md
git commit --no-verify -m "docs(guide): reconcile enterprise-install + phase guides to saml-setup.sh as built"
```

---

### Task B7: End-to-end dry-run gate

**Files:** none (verification only)

- [ ] **Step 1: Dry-run both appliers end-to-end.** Run:
```bash
scripts/bootstrap/system-bootstrap.sh --dry-run
scripts/bootstrap/saml-setup.sh --profile schema-artifact/saml-profile.yaml --dry-run
```
  Expected: `system-bootstrap.sh` prints its phases 1–5 (unchanged); `saml-setup.sh` prints steps 3, 6 (SKIPPED), 11 (per-mapping), 12 with every profile value substituted and no errors.

- [ ] **Step 2: Final shellcheck of both.** Run: `shellcheck scripts/bootstrap/system-bootstrap.sh scripts/bootstrap/saml-setup.sh` — Expected: clean.

- [ ] **Step 3: Confirm the split invariant.** Run: `grep -niE 'saml|kb_saml|provision|map-group|AS_' scripts/bootstrap/system-bootstrap.sh` — Expected: **no matches** (SAML logic must not have leaked into the auth-agnostic spine).

- [ ] **Step 4: Commit any final fixes** (if steps 1–3 surfaced adjustments), else proceed to PR.

---

## Self-Review

- **Spec coverage:** `saml-setup.sh` (steps 3/6/11/12) → B1–B5; `saml-profile.yaml` → B2; two-scripts-separate invariant → B7 step 3; reciprocal doc reconciliation → B6; emit-by-default/apply-opt-in → B4/B5 `--apply-db` gating. `system-bootstrap.sh` needs no functional change (already covers steps 8–10, 13 and is already auth-agnostic) — confirmed, not modified beyond B6's doc cross-reference. Covered.
- **Placeholder scan:** every step ships real bash; the only `[TODO: Task Bn]` markers are in B1's skeleton and are explicitly replaced by B3–B5.
- **Consistency:** step numbers 3/6/11/12 match Arc A's timeline; CLI flags (`--env-out`, `--sql-out`, `--client`, `--from-seen`, `--apply`, `--db`) match the grep-verified `admin_saml.rs` surface; `--apply-db` is the script's own guard flag (distinct from the CLI's `--apply`), used consistently across B4/B5/B6.
