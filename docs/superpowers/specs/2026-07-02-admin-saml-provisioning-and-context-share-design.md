# `temper admin saml` provisioning + `temper context share` — design spec

**Status:** Design (plan/medium). Grounded against `main` 2026-07-02, after SAML Phase 2 (PR #234) merged + deployed.
**Task:** `guided-scripted-self-hosting-provisioning-tool…-019f2353-23f9…` (`@me/temper`).
**Prior art it composes with:** the org-provisioning bootstrap (all 7 chunks shipped) — `schema-artifact/install-profile.yaml` + `scripts/bootstrap/system-bootstrap.sh` (Chunk 7, PR #210) + `docs/guides/org-bootstrap.md`.

## 1. Problem

Standing up a self-hosted **SAML** instance is a bespoke, manual, error-prone sequence
(`docs/guides/self-hosting-saml.md`): generate an EdDSA signing key, hand-write a large env
surface consistently across **two** Vercel functions (the TypeScript AS in `packages/temper-cloud`
and the Rust `temper-api`), and hand-write SQL with correct UUIDs for `kb_saml_idp` and
`kb_saml_group_mappings`. SAML Phase 2 (PR #234) added more of these hand-assembled steps. The
values that MUST agree across the two functions — audience, issuer, provider name, and the shared
`INTERNAL_RECONCILE_SECRET` — are today kept consistent only by operator discipline; a subtle
mismatch fails closed and silently.

The org-provisioning side is already solved: `temper team create`, `temper admin settings`,
`temper admin promote`, `temper cogmap create`/`bind` are surfaced, and `system-bootstrap.sh` loops
them from a declarative `install-profile.yaml`. **SAML is the remaining pure-config gap** — and
unlike org-bootstrap, most of it *cannot* become a surfaced command (env vars are Vercel platform
config; the first `kb_saml_idp` row and first admin are pre-auth DB writes). So the SAML surface is
fundamentally an **emitter**: generate keys → emit a consistent env bundle → emit SQL.

A second, adjacent gap surfaced during the T6 steward deploy (2026-07-02): **`kb_team_contexts` has
no CLI/API writer**. Wiring a team's ingest corpus needs a context to be team-owned *or*
shared-into the team; the share row had to be hand-`INSERT`ed. This is the read-reach sibling of
`kb_team_cogmaps` and belongs to the same "provision a usable org" story, so it is folded into this
task as a second limb.

## 2. Grounded current state (anchors)

### 2.1 The SAML surface is SQL + env only — no `temper` command exists

- `kb_saml_idp` (`migrations/20260701000006_saml_as_tables.sql:21-33`): PK `idp_key`, `is_active`,
  `idp_cert`, `idp_sso_url`, `idp_entity_id`, `sp_entity_id`, `acs_url`, `nameid_format`,
  `email_attr`, `stable_id_attr`; `groups_attr` added in `20260702000001_saml_group_provisioning.sql`.
  "Single active IdP" = flip `is_active` to rotate.
- `kb_saml_group_mappings` (`20260702000001:10-18`): `(idp_key, group_value, team_id, role)`,
  PK `(idp_key, group_value, team_id)`, `role team_role`. Role-max collapse is **server-side reconcile
  behavior**, not the emitter's concern.
- `kb_saml_seen_groups` (`20260702000001:26-33`): `(idp_key, group_value, first_seen, last_seen)` —
  discovery capture upserted on each reconcile; "the mapping table need not be pre-populated."
- **AS is TypeScript**: `packages/temper-cloud/src/oauth/keys.ts:29-49` reads `AS_SIGNING_KEY_PKCS8`
  and imports it via `jose` `importPKCS8(pem, "EdDSA")` + node `createPublicKey(pem)` — standard
  PKCS#8 PEM. `reconcile.ts:26` reads `INTERNAL_RECONCILE_SECRET`.
- **Reconcile endpoint is Rust**: `crates/temper-api/src/handlers/internal_saml.rs:14`
  (`POST /internal/saml/reconcile`, `routes.rs:180`), gated by
  `middleware/internal_auth.rs` (constant-time-compared `INTERNAL_RECONCILE_SECRET`);
  `temper-services/src/config.rs:65` also reads the secret.
- The full env contract is `self-hosting-saml.md §4` (AS: `AS_ISSUER/AS_AUDIENCE/AS_SIGNING_KEY_PKCS8/
  AS_SIGNING_KID/AS_CLIENTS/AS_ACCESS_TTL_SECONDS/AS_REFRESH_TTL_SECONDS`; api: `JWKS_URL/AUTH_ISSUER/
  AUTH_AUDIENCE/AUTH_PROVIDER_NAME`; shared: `INTERNAL_RECONCILE_SECRET`; AS-only:
  `INTERNAL_RECONCILE_URL`). No `temper` command touches any of this.

### 2.2 `kb_team_contexts` is a read-reach grant with no writer

- `kb_team_contexts` (`20260624000001_canonical_schema.sql:267-272`): `(context_id, team_id)` PK.
  "GRANT-like in semantics (inherits DOWN the teams DAG via `team_ancestors`)." Sibling of
  `kb_team_cogmaps` in shape.
- It is a **READ path only** — enters `resources_visible_to` / `context_visible_to`
  (`canonical_functions.sql:147-160`, service header `context_service.rs:3-11`); the write axis
  comment (`canonical_functions.sql`) is explicit: "Context-share is deliberately NOT a write path."
- Only scenario/test loaders write it today (task field evidence + `context_service.rs` has
  `create` but no `share`).
- Existing gate precedent: the structural sibling `temper cogmap bind` (`kb_team_cogmaps`,
  org-provisioning Chunk 5) is gated `is_system_admin`.

### 2.3 The `temper admin` group is the operator namespace

- `AdminAction` (`crates/temper-cli/src/cli.rs:604-637`): `Settings`, `Promote`,
  `Requests { subcommand }` — nesting precedent for a `Saml { subcommand }` group.
- `ContextAction` (`cli.rs:506-525`): `Add`, `Remove`, `Create { --owner }`, `List` — the home for
  `Share`/`Unshare`.
- A successful `temper admin settings` read already *proves* `is_system_admin` for the caller
  (the read is admin-gated) — the basis for the `verify` verb's admin check.

### 2.4 The bracketing order (why SAML wraps org-bootstrap)

The end-to-end flow interleaves; SAML steps sit on **both sides** of org-bootstrap:

1. `admin saml provision` → set AS+API env on Vercel → deploy → apply `kb_saml_idp`
   *(SAML login must work before anyone can authenticate)*.
2. First admin logs in via SAML → JIT provisions their `kb_profiles` row
   (`internal_saml.rs`) — only now does a profile id exist.
3. org-bootstrap: SQL root step (needs that profile id) → `temper team create` … (existing
   `system-bootstrap.sh`).
4. `admin saml map-group` → `kb_saml_group_mappings` *(the teams from step 3 must already exist —
   the FK is `team_id REFERENCES kb_teams`)*.
5. `admin saml verify`.

## 3. Settled decisions (this brainstorm, 2026-07-02)

1. **Home = the shipped `temper admin` group, not a separate binary/crate.** One operator mental
   model (`temper admin …`). The blast-radius worry is answered structurally: an *emitter* is inert
   (writes nothing, grants nothing); the only privileged path (`--apply`) is gated by possession of
   `DATABASE_URL` + DB credentials, exactly like the `admin promote`/`settings` commands the CLI
   already ships. Keeping it in Rust retains typed consistency + testability.
2. **Emit-by-default, opt-in `--apply`.** Env is *always* emit-only (the tool cannot set Vercel env)
   — stdout or `--env-out .env.saml`. SQL (`idp`, `map-group`) emits by default; `--apply` runs it
   against `DATABASE_URL`.
3. **Consistency-by-construction via one typed struct.** A single `SamlProvisionConfig` is the sole
   source of truth; every shared value is *derived*, so `AS_AUDIENCE == AUTH_AUDIENCE`,
   `AUTH_ISSUER == AS_ISSUER`, `AUTH_PROVIDER_NAME == saml:<idp_key>`, and the one
   `INTERNAL_RECONCILE_SECRET` cannot drift across the two functions. Env + SQL rendering are pure
   functions of the struct (typed structs over `json!()`/string concat — code-quality tenet).
4. **Rust-native keygen** (ed25519 → PKCS#8 PEM), so operators need no `openssl`. Compatible with the
   TS AS's `importPKCS8`/`createPublicKey` (§2.1). A cross-runtime format contract is a tested risk (§7).
5. **Three focused verbs** — `provision` / `map-group` / `verify` — mapping to the bracketing phases;
   each independently re-runnable.
6. **Context-share gate = `is_system_admin` (interim).** Mirrors its structural sibling `cogmap bind`;
   fail-closed; consistent with the provisioning framing. Explicitly interim — a later RBAC arc may
   relax it to context-owner (+ team maintainer). (See `project_authorial_rbac_undefined_contexts_cogmaps`.)

## 4. Design

### 4.1 Limb 1 — `temper admin saml { provision, map-group, verify }`

**`provision`** — the before-first-login bracket.
- Interactive prompts *or* `--no-interactive` + flags (mirrors `temper init`): instance URL,
  `idp_key`, IdP cert/PEM, SSO URL, IdP + SP entity ids, ACS URL, nameid format, email/stable-id
  attrs, optional `groups_attr`, `AS_CLIENTS` (cli + ui redirect URIs), TTL overrides, `--kid`.
- Generates: ed25519 signing key (PKCS#8 PEM), `AS_SIGNING_KID` (default `as-YYYY-MM`, overridable),
  `INTERNAL_RECONCILE_SECRET` (≥32 random bytes, base64).
- Renders from `SamlProvisionConfig`:
  - the complete env bundle (AS-side + api-side + shared), consistent by construction;
  - the `kb_saml_idp` INSERT (all columns typed from input).
- Output: env to stdout (or `--env-out .env.saml`); SQL to stdout (or `--apply` → run against
  `DATABASE_URL`). The private key is written only where the operator directs (never echoed into logs
  unbidden; `--env-out` writes it to the file, with a mode-0600 note).

**`map-group`** — the after-teams-exist bracket.
- `temper admin saml map-group --idp <key> <group_value> <+team/slug> --role <role>` (repeatable /
  batch). Resolves `+team/slug → team_id` via the authenticated API (`temper team list` path) so no
  DB creds are needed to *emit*; emits the `kb_saml_group_mappings` INSERT with the resolved UUID.
- `--from-seen` reads `kb_saml_seen_groups` (via DB or an admin endpoint) to list groups the IdP has
  actually asserted, so mappings can be added reactively.
- `--apply` runs the INSERT. Role-max collapse stays server-side (reconcile), not here.

**`verify`** — post-setup confidence, closing the T6 silent-403 gap.
- Instance probes (no DB creds): AS metadata (`/.well-known/oauth-authorization-server`) + `/oauth/jwks`
  reachable ⇒ AS mode on; a successful authenticated `admin settings` read ⇒ `is_system_admin` true
  for the caller (**the exact field-evidence gap**: on prod, `gating_team_slug=''` made
  `is_system_admin` false for everyone, silently).
- DB probe (`--db`/`DATABASE_URL`): exactly one active `kb_saml_idp` row.
- Reports each check pass/fail with the remediation pointer. (The reconcile-secret parity across
  functions is env-only and not externally checkable — noted, not asserted.)

### 4.2 Limb 2 — `temper context share` / `unshare`

- CLI: `ContextAction::Share { context: <ref>, team: <+slug> }` / `Unshare { … }`
  (`cli.rs:506`).
- API: `POST /api/contexts/{id}/teams` + `DELETE …/{team_id}` — mirrors `cogmap bind`'s
  `POST /api/cognitive-maps/{id}/teams`.
- Service: `context_service::share`/`unshare` — auth-before-write (`is_system_admin`) then
  insert/delete the `(context_id, team_id)` row (plain write, no event emission, per the context
  "infrastructure, no events" precedent, `context_service.rs:10-11`).
- Client: `ContextsClient::share`/`unshare`.

## 5. The unified SoP (docs)

- `self-hosting-saml.md`: make the three verbs the happy path; retain the manual SQL/env as the
  documented fallback/reference (acceptance criterion).
- `org-bootstrap.md`: add the interleave-ordering note (§2.4) so the operator sees where the SAML
  brackets wrap the existing bootstrap.

## 6. Build roadmap (beats)

| Beat | Scope | Depends on | Gate |
|------|-------|------------|------|
| **A** | Context-share surface: API + service (`is_system_admin`) + client + `context share`/`unshare` CLI | — | e2e access-semantics (share widens a team's read-reach; unshare reverses; non-admin `Forbidden`) |
| **B** | `SamlProvisionConfig` + keygen + env/SQL rendering (pure, no I/O) | — | unit/snapshot: shared values equal by construction; PEM label = `PRIVATE KEY` |
| **C** | `admin saml provision` wiring: interactive + `--no-interactive` + emit / `--apply` / `--env-out` | B | e2e/unit: emitted bundle round-trips; `--apply` writes one active `kb_saml_idp` row |
| **D** | `admin saml map-group` (+ `--from-seen`, slug→team_id resolve, emit/`--apply`) | B, (A for team refs) | e2e: mapping INSERT resolves the right `team_id`; `--from-seen` lists seen groups |
| **E** | `admin saml verify` (instance probes + DB probe) | C | e2e: passes on a provisioned instance; fails loudly on the `gating_team_slug=''` case |
| **F** | Docs: `self-hosting-saml.md` happy path + `org-bootstrap.md` interleave note | A–E | doc review |

Beats A and B are independent and parallelizable; C–E stack on B; F is the capstone.

## 7. Open questions / risks

- **Keygen cross-runtime format contract.** The Rust-emitted PKCS#8 PEM must be accepted by the TS
  AS's `importPKCS8`/`createPublicKey`. Mitigation: Beat B asserts the PEM structure; a
  round-trip check (Rust emits → `jose` imports) is the strongest guard — decide in planning whether
  that lands as a `temper-cloud` test fixture or a documented manual verify.
- **`--apply` for `kb_saml_idp`** must respect "single active row" — insert-active should optionally
  deactivate the prior active row (rotate) vs. plain insert. Resolve in Beat C (default: refuse if an
  active row exists unless `--rotate`).
- **`--from-seen` read path** — **resolved: DB query (`--db`), not a new API endpoint.** This level of
  provisioning is deliberately not an API-surface concern right now; a discovery convenience does not
  justify a new endpoint.
- **Secret handling** — the private key + reconcile secret are sensitive; the tool must never log
  them and `--env-out` should chmod 0600. Confirm in Beat C.

## 8. Deferred / out of scope

- **Multi-IdP `idp_key` scoping on `kb_team_members`** — reconcile currently reads all non-native
  memberships as this IdP's; a 2nd *active* IdP needs a discriminator. `kb_saml_idp.idp_key` and
  `kb_saml_group_mappings.idp_key` already exist, but the membership table does not carry it. No
  action until a 2nd IdP is real (carry-forward from Phase 2 §12).
- **Context-share RBAC beyond `is_system_admin`** — owned by the dedicated RBAC arc.
- **Auto-setting Vercel env** — out of scope (platform-specific; emit is the contract).

## 9. Acceptance criteria

- An operator goes from fresh Neon+Vercel to a working SAML instance (SSO login + group provisioning
  + first admin) via `temper admin saml` + the existing org-bootstrap — interactively or fully
  scripted — without hand-editing SQL or env.
- Generated secrets/keys are strong; the AS↔API shared values are consistent by construction (proven
  by a test, not discipline).
- `temper context share` gives a team read-reach into a shared context (`is_system_admin`-gated),
  reversible via `unshare`.
- Both self-hosting guides reference the verbs as the happy path (manual steps retained as fallback);
  `verify` catches the `gating_team_slug` silent-403 class of failure.
