# Enterprise Install — Arc A: Ground-Up Runbook Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Author `docs/guides/enterprise-install.md` — the single ground-up SAML/Okta-primary operator runbook that flattens the five phase guides into one operable timeline with one consolidated env matrix.

**Architecture:** A new operator guide that *links to* the existing phase guides (`self-hosting.md`, `self-hosting-saml.md`, `org-bootstrap.md`, `vercel-eve.md`) rather than duplicating their prose. Four parts: consolidated env matrix, linear timeline (with owner annotations + emit/apply split), traps callout, scripted-vs-manual seam list. It presents the tooling (`system-bootstrap.sh`, `saml-setup.sh`) as the expected path with manual steps as the documented fallback.

**Tech Stack:** Markdown (mdBook-compatible), markdownlint. No code. Verification is grep-against-source (every referenced `temper` command/flag must exist) + sequence-match against the e2e tests.

## Global Constraints

- **SAML/Okta is the primary auth path.** Temper's native AS fronting Okta's SAML app. Auth0 and Okta-OAuth are noted variants only (link to `self-hosting.md` / `self-hosting-okta.md`).
- **Do not duplicate phase-guide prose.** Cite/link `self-hosting.md`, `self-hosting-saml.md`, `org-bootstrap.md`, `vercel-eve.md`, `packages/temper-ui/.env.example` as sources of truth for raw values.
- **Tooling is the expected path; manual is the documented fallback.** (Spec goal #5.)
- **Eve is a forward-pointer only** — surface `vercel-eve.md` exists, is incomplete, has its own env contract; not a sequenced step.
- **Every `temper` command + flag referenced MUST exist** — verified by grep against `crates/temper-cli/src/cli.rs` and `crates/temper-cli/src/commands/admin_saml.rs`. No invented flags.
- **The timeline MUST NOT diverge** from the sequences exercised in `tests/e2e/tests/org_bootstrap_e2e.rs` and `tests/e2e/tests/admin_surface_e2e.rs` (`root_bootstrap_first_admin`).
- **The two scripts referenced** (`scripts/bootstrap/system-bootstrap.sh`, `scripts/bootstrap/saml-setup.sh`) — the first exists today; the second is delivered by Arc B in the **same PR**. Reference both as the expected path; Arc B reconciles the exact flag names.
- Commit after each task. Docs-only commits may use `git commit --no-verify` (the repo pre-commit runs the full Rust clippy/docs suite, which is irrelevant to markdown and times out — see the session note).

---

### Task A1: Scaffold + orientation

**Files:**
- Create: `docs/guides/enterprise-install.md`

**Interfaces:**
- Produces: the file all later tasks append sections to; the H1 `# Enterprise Install — Ground Up` and the section anchors `#environment-matrix`, `#the-timeline`, `#traps`, `#scripted-vs-manual`.

- [ ] **Step 1: Create the file with title + orientation section.** Write the H1 and an orientation section containing:
  - A one-paragraph framing: "This is the spine for a first real enterprise install — it flattens the phase guides into one sequence. Each detailed step links to its phase guide; this document is the order and the joins, not the detail."
  - A **"What you end up with"** table (adapt from `org-bootstrap.md:36-43`, extended): deployed API+MCP behind Okta-SAML SSO · a first system admin · instance settings · an everyone-team every member auto-joins · an org-identity telos-charter cogmap born+bound · (optional) the UI · (deferred) the eve steward.
  - A **"Four phases"** list: (B) backend deploy + auth, (C) org bootstrap, (D) agents [deferred], with the note that (A) installing the `temper` binary is a prerequisite (link `install.md`).
  - A **Prerequisites** block: an `embed`-capable `temper` binary (`org-bootstrap.md:49-52`); `psql` + `DATABASE_URL_UNPOOLED` for the DB steps; Okta admin access; a Vercel project; a Neon project.

- [ ] **Step 2: Verify structure.** Run: `grep -nE '^#|^##' docs/guides/enterprise-install.md` — Expected: H1 + the orientation subsections present, no other sections yet.

- [ ] **Step 3: Markdownlint.** Run: `npx markdownlint-cli2 docs/guides/enterprise-install.md` (or the repo's configured linter — check `cargo make lint` in tasker-book conventions; if none configured here, skip). Expected: clean.

- [ ] **Step 4: Commit.**
```bash
git add docs/guides/enterprise-install.md
git commit --no-verify -m "docs(guide): scaffold enterprise-install runbook + orientation"
```

---

### Task A2: Part 1 — the consolidated environment matrix

**Files:**
- Modify: `docs/guides/enterprise-install.md` (append the `## Environment matrix` section)

**Interfaces:**
- Consumes: the file from A1.
- Produces: the `#environment-matrix` section referenced by the timeline (Task A3).

- [ ] **Step 1: Write the three-column variable matrix.** A table with columns `Variable` | `temper-cloud (api+mcp)` | `temper-ui` | `eve` | `Notes`, grouped by concern. Populate from these verified sources (do not invent — each cell traces to a source):

  **api+mcp** (from `self-hosting.md:143-161` + `self-hosting-saml.md` AS block):
  `DATABASE_URL`, `DATABASE_URL_UNPOOLED` (deploy step), `AUTH_ISSUER`, `JWKS_URL`, `AUTH_AUDIENCE`, `AUTH_PROVIDER_NAME`, `MCP_AUDIENCE`, `MCP_CLIENT_ID`, `MCP_BASE_URL`, `BLOB_READ_WRITE_TOKEN`, `SQLX_OFFLINE` (build), `ENABLE_SWAGGER` (opt), `PORT` (opt), `CORS_ORIGINS` (situational), and the SAML AS block: `AS_ISSUER`, `AS_AUDIENCE`, `AS_SIGNING_KEY_PKCS8`, `AS_SIGNING_KID`, `AS_CLIENTS`, `AS_ACCESS_TTL_SECONDS`, `AS_REFRESH_TTL_SECONDS`, `INTERNAL_RECONCILE_SECRET`, `INTERNAL_RECONCILE_URL`.

  **temper-ui** (from `self-hosting.md:288-300` + `packages/temper-ui/.env.example`):
  `API_BASE_URL`, `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET`, `OIDC_AUDIENCE` (situational), `OIDC_PUBLIC_CLIENT` (SAML AS = public PKCE), `APP_URL`, `SESSION_SECRET`, `DATABASE_URL`, `STOREFRONT_ENABLED` (opt).

  **eve** (from `vercel-eve.md:144-151`) — mark the whole column **DEFERRED**:
  `TEMPER_MCP_URL`, `TEMPER_API_URL`, `TEMPER_SELF_COGMAP_ID`, `TEMPER_CONNECT_CONNECTOR` (prod), `TEMPER_TOKEN` (dev), `TEMPER_MCP_AUDIENCE` (opt).

- [ ] **Step 2: Write the "must-match by construction" sub-table.** Verbatim the join table from the spec:

| Join | Values that must be equal |
|------|---------------------------|
| Audience | `AS_AUDIENCE` = `AUTH_AUDIENCE` = `MCP_AUDIENCE` = UI `OIDC_AUDIENCE` |
| Issuer | `AS_ISSUER` = `AUTH_ISSUER`; UI `OIDC_ISSUER` resolves the same issuer |
| Provider label | `AUTH_PROVIDER_NAME` = `saml:<idp-key>` |
| Reconcile secret | `INTERNAL_RECONCILE_SECRET` identical on the AS and API env (same Vercel project) |
| Database | `DATABASE_URL` (pooled) shared api/mcp/ui; `DATABASE_URL_UNPOOLED` migrations only |

  Add a sentence: `temper admin saml provision` renders the `AS_*` + reconcile block so these are consistent by construction — it is the reason the SAML env is emitted, not hand-written.

- [ ] **Step 3: Verify every variable exists in a cited source.** Run these greps; each var name must appear in at least one source file:
```bash
grep -RnE 'AS_ISSUER|AS_AUDIENCE|AS_SIGNING_KEY_PKCS8|AS_CLIENTS|INTERNAL_RECONCILE_SECRET' docs/guides/self-hosting-saml.md
grep -nE 'API_BASE_URL|OIDC_ISSUER|OIDC_PUBLIC_CLIENT|SESSION_SECRET|APP_URL' packages/temper-ui/.env.example docs/guides/self-hosting.md
grep -nE 'TEMPER_MCP_URL|TEMPER_SELF_COGMAP_ID|TEMPER_CONNECT_CONNECTOR' docs/guides/vercel-eve.md
```
  Expected: every referenced variable resolves. If any does not, correct the matrix to match the source (source wins).

- [ ] **Step 4: Commit.**
```bash
git add docs/guides/enterprise-install.md
git commit --no-verify -m "docs(guide): enterprise-install consolidated env matrix + must-match joins"
```

---

### Task A3: Part 2 — the linear timeline

**Files:**
- Modify: `docs/guides/enterprise-install.md` (append `## The timeline`)

**Interfaces:**
- Consumes: the env matrix section (A2) — the timeline references it at the "set env" step.
- Produces: the `#the-timeline` section; the canonical step numbering Arc B's scripts mirror 1:1.

- [ ] **Step 1: Write the emit/apply framing paragraph.** Explain that `temper admin saml provision` is an inert emitter run early *only because it generates the ed25519 signing key + reconcile secret that must be in the env before deploy*; it emits two artifacts landing at different times — the **env bundle** (`--env-out`, used pre-deploy) and the **`kb_saml_idp` INSERT** (`--sql-out`, applied post-migrate, since the table does not exist until migrations run).

- [ ] **Step 2: Write the numbered timeline table.** Columns `#` | `Step` | `Owner` | `Detail link`. Content (verbatim step list + owners):
```
 1 Provision Neon (PG17, vector + pg_uuidv7, pooled/unpooled)      manual                       self-hosting.md#provision-neon
 2 Register Okta SAML app; capture cert / SSO URL / entity ids /
   group attribute statement                                      manual                       self-hosting-saml.md + Okta note below
 3 temper admin saml provision → generate keys, --env-out bundle,
   --sql-out kb_saml_idp SQL (inert; early for the env keys)       saml-setup.sh (emit)         self-hosting-saml.md
 4 Set Vercel env (matrix + emitted bundle) on api + mcp           manual                       #environment-matrix
 5 Deploy backend; sqlx migrate run against DATABASE_URL_UNPOOLED  manual                       self-hosting.md#run-migrations
 6 Apply kb_saml_idp row (--apply, or psql the --sql-out file)     saml-setup.sh (apply)        self-hosting-saml.md
 7 First admin signs in via SAML → JIT kb_profiles row             manual                       self-hosting-saml.md
 8 SQL root step: gating team + first admin;
   VERIFY is_system_admin(<uuid>) = true                          system-bootstrap.sh --run-root  org-bootstrap.md#0
 9 temper admin settings (instance name, gating team, mode)        system-bootstrap.sh          org-bootstrap.md#1
10 temper team create everyone --auto-join-role watcher            system-bootstrap.sh          org-bootstrap.md#2
11 temper admin saml map-group (after teams exist)                 saml-setup.sh (emit/apply)   self-hosting-saml.md
12 temper admin saml verify                                        saml-setup.sh                self-hosting-saml.md
13 Telos-charter: cogmap create → reconcile → bind +everyone       system-bootstrap.sh          org-bootstrap.md#3-5
14 (optional) UI deploy: confidential OIDC client, API_BASE_URL,
   SESSION_SECRET                                                  manual                       self-hosting.md#deploy-the-ui-optional
15 Verify: health, temper login, resource round-trip              manual                       self-hosting.md#verify
   → team-self-cognition + eve steward: DEFERRED                   —                            vercel-eve.md
```

- [ ] **Step 3: Write the "expected path" paragraph.** State that the happy path is: `temper admin saml provision` (step 3) → do the platform steps 4–5 → run `saml-setup.sh` for steps 6/11/12 and `system-bootstrap.sh --run-root` for steps 8–10, 13 — the numbered breakdown above is the reference an operator reads to understand what each script does or falls back to when running by hand. Note the two scripts are separate so `system-bootstrap.sh` (steps 8–10, 13) works unchanged for Auth0/Okta-OAuth installs (which swap steps 2–3, 6, 11–12 for the Auth0 app registration in `self-hosting.md`).

- [ ] **Step 4: Write the short "Okta SAML app" note.** A callout: in Okta, create a SAML 2.0 app; capture the **SSO URL** (→ `idp_sso_url`), the **signing certificate** PEM (→ `idp_cert_file`), the **IdP entity id** (→ `idp_entity_id`), and add a **group attribute statement** exposing the user's groups (→ `groups_attr`, e.g. `groups`). Defer the generic SAML-IdP side to `self-hosting-saml.md`. Note the SP ACS/entity values Temper's AS expects come from `self-hosting-saml.md`.

- [ ] **Step 5: Verify every command + flag exists.** Run:
```bash
grep -nE 'Provision|MapGroup|Verify|auto-join-role|AutoJoinRole' crates/temper-cli/src/cli.rs
grep -nE 'env_out|sql_out|apply|from_seen|instance_url|groups_attr|idp_cert_file' crates/temper-cli/src/commands/admin_saml.rs
grep -nE 'admin settings|cogmap create|cogmap reconcile|cogmap bind|team create' docs/guides/org-bootstrap.md
```
  Expected: `--env-out`, `--sql-out`, `--apply`, `--auto-join-role`, `map-group --from-seen`, `verify --db` all resolve. Fix any timeline command that names a flag not present in source.

- [ ] **Step 6: Verify sequence-match against the e2e tests.** Run:
```bash
grep -nE 'admin settings|team create|cogmap (create|reconcile|bind)|system_access|gating_team_slug|is_system_admin' tests/e2e/tests/org_bootstrap_e2e.rs tests/e2e/tests/admin_surface_e2e.rs
```
  Expected: the ordered commands in steps 8–13 match the order the e2e tests drive. If the timeline diverges from the test, the test wins — correct the timeline.

- [ ] **Step 7: Commit.**
```bash
git add docs/guides/enterprise-install.md
git commit --no-verify -m "docs(guide): enterprise-install linear timeline + Okta SAML-app note"
```

---

### Task A4: Part 3 (traps) + Part 4 (scripted-vs-manual/deferred)

**Files:**
- Modify: `docs/guides/enterprise-install.md` (append `## Traps` and `## Scripted vs. manual, and what's deferred`)

**Interfaces:**
- Consumes: the timeline (A3) — traps reference specific steps.

- [ ] **Step 1: Write the Traps callout.** One box, each item citing its source:
  - `is_system_admin` reads gating-team **ownership**, not `kb_profiles.system_access`; `gating_team_slug` NULL/empty ⇒ silent 403 for everyone (`self-hosting-saml.md:80-82`, `l0-content-delivery.md:71-88`; field evidence 2026-07-02). Set both halves at step 8; verify the gate returns true.
  - `API_BASE_URL` self-proxy loop → `508 Loop Detected`; must be the API's own distinct origin (`self-hosting.md:277`).
  - `AS_CLIENTS` missing rejects every `/oauth/authorize`; unset `INTERNAL_RECONCILE_SECRET` silently disables group provisioning (`self-hosting-saml.md:195,230-233`).
  - `embed`-feature binary required for `cogmap create`/`reconcile` (`org-bootstrap.md:49-52`).
  - Migrations are a deploy step, not startup — run `sqlx migrate run` against the unpooled URL, back up first (`self-hosting.md:66-73`, `DEPLOYING.md`).

- [ ] **Step 2: Write the "Scripted vs. manual, and what's deferred" section.** A short table: which timeline steps are automated today (steps 8–10, 13 via `system-bootstrap.sh`; steps 3/6/11/12 via `saml-setup.sh` once Arc B lands), which stay manual (1–2, 4–5, 7, 14–15), and what's deferred (eve/M2M, plan/diff applier semantics, SCIM Phase 3, cogmap-write-by-team-role). Frame it as: tooling is the expected path; these are the remaining manual edges and the roadmap tail.

- [ ] **Step 3: Verify each trap + seam cites a real line.** Run:
```bash
grep -n 'Loop Detected' docs/guides/self-hosting.md
grep -nE 'AS_CLIENTS|INTERNAL_RECONCILE_SECRET' docs/guides/self-hosting-saml.md
grep -nE "is_system_admin|gating_team_slug" docs/guides/l0-content-delivery.md
```
  Expected: each citation resolves.

- [ ] **Step 4: Commit.**
```bash
git add docs/guides/enterprise-install.md
git commit --no-verify -m "docs(guide): enterprise-install traps callout + scripted-vs-manual seam list"
```

---

### Task A5: Cross-links from the phase guides + final gate

**Files:**
- Modify: `docs/guides/self-hosting.md` (top-of-file pointer)
- Modify: `docs/guides/self-hosting-saml.md` (top-of-file pointer)
- Modify: `docs/guides/org-bootstrap.md` (top-of-file pointer)
- Modify: `docs/guides/enterprise-install.md` (final read pass)

**Interfaces:**
- Consumes: the complete `enterprise-install.md`.

- [ ] **Step 1: Add a back-pointer to each phase guide.** At the top of each of `self-hosting.md`, `self-hosting-saml.md`, `org-bootstrap.md`, add a one-line note-block: `> **Doing a full ground-up enterprise install?** This guide is one phase. For the single end-to-end sequence (deploy → SAML → org → agents) see [enterprise-install.md](./enterprise-install.md).` Match each file's existing note-block style (they use `> **...**` blockquotes).

- [ ] **Step 2: Final read pass of `enterprise-install.md`.** Confirm: the "expected path" voice is consistent (scripts primary, manual fallback); every internal anchor link resolves; the eve column/step is marked DEFERRED everywhere it appears; no phase-guide prose is duplicated (only linked).

- [ ] **Step 3: Verify links resolve.** Run:
```bash
grep -oE '\]\(\./[a-z0-9-]+\.md' docs/guides/enterprise-install.md | sed 's/](\.\///' | sort -u | while read f; do test -f "docs/guides/$f" && echo "OK $f" || echo "MISSING $f"; done
```
  Expected: all `OK`, no `MISSING`.

- [ ] **Step 4: Commit.**
```bash
git add docs/guides/enterprise-install.md docs/guides/self-hosting.md docs/guides/self-hosting-saml.md docs/guides/org-bootstrap.md
git commit --no-verify -m "docs(guide): cross-link phase guides to the enterprise-install spine"
```

---

## Self-Review

- **Spec coverage:** Part 1 matrix → A2; Part 2 timeline (emit/apply split, owner annotations, Okta note) → A3; Part 3 traps → A4; Part 4 seam list → A4; "expected path" voice (goal #5) → A3 step 3 + A5 step 2; eve forward-pointer → A2 (deferred column) + A3 (deferred step) + A5. Covered.
- **Placeholder scan:** the matrix/timeline/traps content is enumerated verbatim from verified sources; no "TBD"/"add appropriate…".
- **Consistency:** step numbers 1–15 are used identically in A3, A4, and Arc B's plan; command flags (`--env-out`, `--sql-out`, `--apply`, `--auto-join-role`, `--from-seen`, `--db`) match the grep-verified source.
