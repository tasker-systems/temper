# Enterprise Install — Ground-Up Operator Runbook (+ script hardening)

**Status:** Design · **Date:** 2026-07-02 · **Author:** Cole (j-cole-taylor)

## Problem

Standing up a real self-hosted Temper instance today means reassembling a coherent
sequence from **five separate guides**, each competent in isolation but never joined:

- `self-hosting.md` — backend deploy (Neon, Vercel, Auth0, the base env contract, optional UI).
- `self-hosting-saml.md` — the native SAML AS, `kb_saml_idp`, group mapping, the `AS_*`/reconcile env block.
- `self-hosting-okta.md` — the Okta-OAuth delta on the base guide.
- `org-bootstrap.md` — the in-app provisioning spine (root SQL → admin settings → everyone-team → cogmap create/reconcile/bind).
- `vercel-eve.md` — the steward agent deploy, with its own env contract.

An operator doing a first real enterprise install must hold `self-hosting.md` open, mentally
patch in the SAML delta, **reassemble the environment contract from three separate tables**,
**interleave the SAML steps around the org-bootstrap steps**, then chain into
`team-self-cognition` → `vercel-eve`. That reassembly is exactly where the field failures live:
the empty-`gating_team_slug` silent-403 (every admin op fails, `auth status` shows
`profile_id: null`), and the `API_BASE_URL` self-proxy `508 Loop Detected`.

There is **no single ground-up timeline** and **no consolidated env matrix**. That connective
tissue is the gap this work closes.

**Field evidence (2026-07-02, prod steward deploy):** `is_system_admin(profile)` is TRUE iff the
profile is `owner` of the team named by `kb_system_settings.gating_team_slug`. On prod the owner
row existed but `gating_team_slug` was the empty string → `is_system_admin` false for everyone,
silently. The two halves drifted. A ground-up runbook must set both and **verify the gate returns
true** before proceeding.

## Goals

1. A single operator-followable **ground-up enterprise install runbook** (`docs/guides/enterprise-install.md`),
   SAML/Okta-primary, that links to the phase guides rather than duplicating them.
2. **One consolidated environment matrix** spanning all three deploy targets
   (`temper-cloud` api+mcp, `temper-ui`, `eve`) with the cross-target must-match joins made
   explicit by construction.
3. **One linear timeline** that flattens the SAML↔org-bootstrap interleave into numbered steps,
   each annotated with its owner (`[manual]` / `[system-bootstrap.sh]` / `[saml-setup.sh]`).
4. A **script-hardening arc** (Arc B) whose structure is dictated by the runbook: two
   separately-runnable scripts built BDD-for-shell (echo-skeleton mirroring the runbook →
   fill-in the automatable steps).
5. **The tooling is the expected path; manual is the documented fallback.** The runbook does not
   merely tag steps scripted-vs-manual as equals — it presents "run the script" as the happy path
   and the underlying step-by-step as the reference an operator falls back to (or reads to
   understand what the script does). Genuinely-unscriptable platform steps (Neon, Okta app, Vercel
   env, deploy) stay manual, but even those the tooling emits templates for where it can.

## Non-goals / deferred

- **Eve is a forward-pointer only.** The runbook surfaces that `vercel-eve.md` exists, is
  incomplete, carries its own env contract, and will get a script when the agent path is closer
  to ready. It is **not** a sequenced step. (App-principal M2M / `client_credentials` remains
  blocked; the `user`-bridge path is not documented in this runbook.)
- **No plan/diff (Terraform-like) applier semantics.** The scripts stay stateless + idempotent.
- **No SCIM / immediate deprovisioning** (SAML Phase 3); reconcile-on-login only.
- **No cogmap-write-by-team-role** — the interim `is_system_admin` gate stands.
- **No multi-region / HA Neon, no alternative messaging backends** — single Vercel + single Neon
  + single tenant, matching `self-hosting.md`.
- **No new CLI surface.** This arc is documentation (Arc A) + shell scripting over the existing
  surfaced commands (Arc B). `temper admin saml`, `temper cogmap`, `temper team`, `temper context`
  already exist (PR #237 and prior).

## Two arcs, one PR, two plans

**Arc A: the runbook.** It is the *target/spec* for Arc B.
**Arc B: script hardening**, structured 1:1 by Arc A's timeline.

The runbook is written **first** deliberately: it lets us generate no-op echo-the-steps skeletons
that mirror it, then fill in the real work per step — BDD-for-shell.

**Delivery: both arcs land in a single PR.** They are mutually reinforcing — authoring the scripts
(Arc B) will reciprocally sharpen the docs (Arc A): the runbook's final step list, command flags,
and "expected path" prose should match the scripts as built, so the two must reconcile before the
PR lands. Writing them in one PR keeps that reconciliation in one review.

**Two implementation plans, sequenced.** Arc A gets its own plan (write the runbook), Arc B gets
its own plan (author the two scripts + reconcile the docs). Arc A's plan executes first so Arc B
has a target; Arc B's plan closes the loop back into the docs. Both plans' output lands on the one
branch / one PR.

---

## Arc A — `docs/guides/enterprise-install.md`

A single SoP with four parts.

### Part 1 — Consolidated environment matrix (the artifact that doesn't exist today)

One table: rows = variables grouped by concern; columns = `temper-cloud (api+mcp)` / `temper-ui` /
`eve`. Then a short **"must-match by construction"** sub-table making the cross-target joins
explicit:

| Join | Values that must be equal |
|------|---------------------------|
| Audience | `AS_AUDIENCE` = `AUTH_AUDIENCE` = `MCP_AUDIENCE` = UI `OIDC_AUDIENCE` |
| Issuer | `AS_ISSUER` = `AUTH_ISSUER`; UI `OIDC_ISSUER` resolves the same issuer |
| Provider label | `AUTH_PROVIDER_NAME` = `saml:<idp-key>` |
| Reconcile secret | `INTERNAL_RECONCILE_SECRET` identical on the AS **and** the API env (same Vercel project) |
| Database | `DATABASE_URL` (pooled) shared api/mcp/ui; `DATABASE_URL_UNPOOLED` migrations only |

Source-of-truth for the raw variables (do not duplicate their prose — cite):
- api+mcp: `self-hosting.md` §"Environment variable contract" + the SAML `AS_*`/reconcile block in
  `self-hosting-saml.md`.
- ui: `self-hosting.md` §"Deploy the UI" env table + `packages/temper-ui/.env.example`.
- eve: `vercel-eve.md` §"Environment contract" — presented in the matrix but flagged **deferred**
  (surfaced, not sequenced).

The eve column is present so the operator sees the whole surface, with a clear "not yet — see
vercel-eve.md" marker.

### Part 2 — The linear timeline

Numbered steps, each tagged with its owner. This flattens the SAML↔org-bootstrap interleave into
one sequence. The **emit/apply split** for SAML is load-bearing: `temper admin saml provision` is
an inert emitter that runs early only because it *generates the ed25519 signing key + reconcile
secret that must be in the env before deploy*. It emits two artifacts landing at different times —
the **env bundle** (used at step 4, pre-deploy) and the **`kb_saml_idp` INSERT** (which has nowhere
to land until migrations exist, so it is applied at step 6, post-migrate).

```
 1 Provision Neon (PG17, vector + pg_uuidv7, pooled/unpooled)     [manual]
 2 Register Okta SAML app; capture cert / SSO URL / entity ids /
   group attribute statement                                     [manual]
 3 temper admin saml provision → GENERATE keys, EMIT env bundle,
   HOLD kb_saml_idp SQL  (inert; early only for the env keys)     [saml-setup.sh: emit]
 4 Set Vercel env (matrix + emitted bundle) on api + mcp          [manual]
 5 Deploy backend; sqlx migrate run against DATABASE_URL_UNPOOLED [manual]
 6 APPLY kb_saml_idp row (--apply or psql) — table now exists     [saml-setup.sh: apply]
 7 First admin signs in via SAML → JIT kb_profiles row            [manual]
 8 SQL root step: gating team + first admin;
   VERIFY is_system_admin(<uuid>) = true                          [system-bootstrap.sh --run-root]
 9 temper admin settings (instance name, gating team, mode)       [system-bootstrap.sh]
10 temper team create everyone --auto-join-role watcher           [system-bootstrap.sh]
11 temper admin saml map-group (after the teams exist)            [saml-setup.sh: emit/apply]
12 temper admin saml verify                                       [saml-setup.sh]
13 Telos-charter: cogmap create → reconcile → bind +everyone      [system-bootstrap.sh]
14 (optional) UI deploy: confidential OIDC client, API_BASE_URL,
   SESSION_SECRET                                                 [manual]
15 Verify: health, temper login, resource round-trip             [manual]
   → team-self-cognition + eve steward: pointer to
     vercel-eve.md, DEFERRED (not sequenced)
```

The owner annotations are the seam between Arc A and Arc B: `[system-bootstrap.sh]` steps are
auth-agnostic (they run identically for Auth0/Okta-OAuth installs), `[saml-setup.sh]` steps are
SAML-only, `[manual]` steps are platform/IdP config that cannot be scripted. The runbook presents
the two scripts as the **expected path** — "run `saml-setup.sh`, then `system-bootstrap.sh`, doing
these platform steps between them" — with the numbered breakdown as the reference an operator reads
to understand what each script does or falls back to when running by hand.

**Okta-as-SAML-IdP note:** the primary auth path is Temper's native AS fronting Okta's SAML app.
A short "Okta SAML app" callout covers the SSO URL, signing cert, and the group attribute
statement; the generic IdP-side detail defers to `self-hosting-saml.md`. (Okta-OAuth remains a
documented variant via `self-hosting-okta.md`, but SAML is the enterprise-default path here.)

### Part 3 — Traps callout

The fail-closed gates that bit in the field, collected in one box:

- **`is_system_admin` reads gating-team ownership, not `system_access`.** `gating_team_slug` is
  NULL/empty by default ⇒ silent 403 for everyone. Set both halves; verify the gate returns true
  (step 8). (Field evidence above.)
- **`API_BASE_URL` self-proxy loop.** Pointing the UI proxy at a shared public domain forwards to
  itself → `508 Loop Detected`. Must be the API's own distinct origin.
- **`AS_CLIENTS` / `INTERNAL_RECONCILE_SECRET` fail-closed.** Missing `AS_CLIENTS` rejects every
  `/oauth/authorize`; an unset reconcile secret silently disables group provisioning.
- **`embed`-feature binary required** for `cogmap create`/`reconcile` (client-side charter embed).
  A non-`embed` build errors; reinstall with `--features embed`.
- **Migrations are a deploy step, not a startup step.** The API does not auto-migrate; run
  `sqlx migrate run` against the unpooled URL, back up first.

### Part 4 — "Scripted vs. manual today / deferred"

The honest seam list that becomes Arc B's punch-list and the roadmap tail:
- env-bundle emission is not yet in a script (only `admin saml provision` emits the SAML block);
- SAML steps are not yet in an applier (Arc B: `saml-setup.sh`);
- `context share` for a team corpus is manual today;
- eve is deferred (M2M app-tokens blocked);
- plan/diff applier semantics deferred; SCIM Phase 3; cogmap-write-by-team-role deferred.

---

## Arc B — script hardening (BDD-for-shell, sequenced after Arc A)

Two **separately-runnable** scripts, split by concern so the database+admin spine stays usable for
non-SAML installs.

### `scripts/bootstrap/system-bootstrap.sh` (exists; auth-agnostic)

Owns the **database + `temper admin` spine**: the SQL root step (gating team + first admin),
`admin settings`, everyone-team, telos-charter (cogmap create/reconcile/bind). Already
auth-provider-agnostic — works unchanged for Auth0/Okta-OAuth. Arc B changes here are minimal:
optionally fold in `context share` (team corpus) and confirm the timeline's step numbering matches
the runbook. It stays the applier `org-bootstrap.md` already documents.

### `scripts/bootstrap/saml-setup.sh` (new; SAML-only)

Owns the **SAML-only** steps, authored BDD-for-shell:

1. **Echo-skeleton first** — every SAML step from the runbook becomes a numbered no-op echo (the
   "pending" scaffold), 1:1 with steps 3, 6, 11, 12.
2. **Fill in** the automatable steps by wrapping the existing surfaced commands:
   `temper admin saml provision` (emit env + hold SQL), apply `kb_saml_idp` (`--apply`),
   `temper admin saml map-group`, `temper admin saml verify`.
3. Genuinely-manual steps between them (set Vercel env, deploy, first login) stay as echoed
   operator instructions, clearly `[manual]`, so the script reads as the full timeline with the
   automatable parts live and the manual parts sign-posted.

The runbook holds the two scripts **separate** (interleaved only by step number in the prose), so
`system-bootstrap.sh` remains the shared spine and `saml-setup.sh` is a purely additive SAML layer.

## Validation

- **Arc A** is validated by the existing `tests/e2e/tests/org_bootstrap_e2e.rs` for its
  `system-bootstrap`-owned steps (root → admin settings → team → cogmap create/reconcile/bind), and
  by `tests/e2e/tests/admin_surface_e2e.rs` (`root_bootstrap_first_admin`) for the SQL root step.
  The runbook's timeline must not diverge from those exercised sequences.
- **Arc B** scripts stay **idempotent** (re-running converges, inherited from the surfaced
  commands' idempotency) and **dry-run-able** (`--dry-run` prints without executing), matching the
  existing `system-bootstrap.sh` contract.

## References

- Guides: `docs/guides/{self-hosting,self-hosting-saml,self-hosting-okta,org-bootstrap,vercel-eve,team-self-cognition-bootstrap,l0-content-delivery}.md`
- Applier + profile: `scripts/bootstrap/system-bootstrap.sh`, `schema-artifact/install-profile.yaml`,
  `schema-artifact/manifests/*.yaml`
- CLI surface: `crates/temper-cli/src/cli.rs` (`admin`, `cogmap`, `team`, `context`),
  `crates/temper-cli/src/commands/admin_saml.rs`
- Prior arcs: `docs/superpowers/specs/2026-07-02-admin-saml-provisioning-and-context-share-design.md`,
  `docs/superpowers/specs/2026-06-28-org-provisioning-bootstrap-surface-design.md`
- Deploy invariants: `DEPLOYING.md`
