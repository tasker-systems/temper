# Security Audit — Surfaces-In AuthN / AuthZ / Credential-Flow

- **Date:** 2026-07-18
- **Scope:** every point of entry (temper-api, temper-mcp, TypeScript OAuth AS / SAML SP),
  the shared auth seam (`temper-services::auth`), and the credential flows behind them
  (OAuth device PKCE, SAML, machine `client_credentials`, minted M2M, Slack link + grant vault,
  internal HMAC gates, embed-cron secrets).
- **Method:** surfaces-in, code-traced (file:line evidence), adversarial. Front-door layering
  and the machine-reach containment were traced by hand; the per-endpoint matrices were produced
  by parallel adversarial subagents and cross-checked against the specs and the shipped SQL.
- **Prompt:** a prior plan was found shipping a PR carrying a *known* self-promotion of
  capabilities via grant mechanics, flagged only as a fast-follow. This is a from-scratch
  re-verification of the trust boundary.
- **Temper goal:** `security-audit-surfaces-in-authn-authz-credential-flow-pen-test-019f764f…`

## Bottom line

- **AuthN (Phase 1): the front door is guarded.** Every entry point verifies the caller's
  identity before privileged work, or is deliberately unauthenticated with a compensating
  control that holds. **No AuthN holes.**
- **AuthZ self-promotion (Phase 2): a live vector EXISTS on `main` — F-0 below.** A `can_grant`
  holder can confer capabilities it does not itself possess — including to *itself* — because
  `grant_capability` (and the direct `insert_grant` callers) bound the *authority to grant* but
  **not the capability being conferred** (no attenuation). A `read+grant` principal can
  self-escalate to `write+delete+grant`. This is the triggering finding's class; it is
  corrected in draft **PR #482** (Task 5b.3). **This audit's first pass MISSED it** — see the
  correction note below.
- **What is clean:** the *authority-acquisition* axis (visibility can never become grant
  authority), the machine-**registration** reach containment (`AuthorizedReach`, type-enforced
  — but note it shares F-0's attenuation blind spot for the `can_write` it confers, also swept
  up by #482's SQL chokepoint), admin/bootstrap gating, and the uniform resource/edge RBAC gate.
- **Findings:** one live self-escalation (F-0), four items for explicit decision, one forward
  tripwire. Ranked in [§5](#5-findings-ranked).

> ### Correction note (2026-07-18, post-review)
> This audit's first pass reported "no live self-promotion vector." **That was wrong for
> `main`.** The grant trace checked whether a caller *without* grant authority could *acquire*
> it (correctly: no) and never checked whether a caller *with* grant authority could confer
> **more capability than it holds** (the amplification / self-escalation axis — F-0). The two
> are different questions; the second is the actual hole and the one this audit was commissioned
> to find. The miss was surfaced by cross-checking against PR #482, whose live probe
> demonstrated the self-escalation. Recorded here rather than quietly edited away, because a
> methodology blind spot that hid the exact target class is itself a finding: **grant audits
> must trace capability attenuation (conferred ⊆ held), not only authority acquisition.**

The user's hypothesis — "AuthN is OK because events require an entity id" — is **not** why it
holds (reads and non-event CRUD bypass event emission). It holds because both Rust surfaces
funnel through the single `authenticate_token` seam by construction (crate-privacy on
`classify`/`Principal`/`authenticate` + a per-tool / per-route chokepoint), independent of
whether a path emits an event.

---

## 1. AuthN — front-door matrix

### The shared seam (`temper-services::auth`)
`authenticate_token` is the one Level-1 gate for both Rust surfaces:
`classify` (a **closed 3-variant sum** Machine/Human/Refuse — no fall-open default arm) →
human email ladder (human only) → `resolve_from_claims` → `gate_resolved_profile` (rejects
`is_active = false`). Machines resolve via `resolve_machine_from_claims`, **lookup-or-401**:
an unregistered or revoked `client_id` never authenticates (`kb_machine_clients` gate).
`classify` refuses machine-shaped `@clients` subjects lacking the `client-credentials` grant,
and machine grants with no derivable client id — both former fall-open paths.

| Surface / route group | AuthN mechanism | Enforcement site | Verdict |
|---|---|---|---|
| API `public` (`/health`) | none | — | by-design unauth |
| API `auth_only` (profile/access self-service) | JWT → seam | `require_auth` (routes.rs:308) | SECURE |
| API `gated` (all data routes) | JWT → seam, then system-access | `require_auth` outermost + `require_system_access` (routes.rs:312-320) | SECURE — e2e `no_auth_returns_401` on `GET /api/resources` proves the layer survives `split_for_parts()` |
| API `/api/embed/{dispatch,warm}` | `EMBED_DISPATCH_SECRET` bearer | self-check, constant-time + length guard, fail-closed on unset/empty (embed.rs:41-70) | SECURE |
| API `/api/embed/admin/reembed` | JWT + `is_system_admin` | embed.rs:186-188 | SECURE |
| API `/api/auth/slack/callback` | PKCE + single-use state + lookup-only resolve | atomic `UPDATE … WHERE consumed_at IS NULL … RETURNING` (slack_link_service.rs:60-72); `authenticate_token_existing_only`; independent JWKS re-verify | SECURE |
| API `/internal/saml/reconcile` | HMAC `INTERNAL_RECONCILE_SECRET` | fail-closed, 30s replay window, constant-time; payload `provider` **ignored** (config authoritative) | SECURE |
| API `/internal/slack/link-state` | HMAC `SLACK_LINK_SECRET` (separate key) | fail-closed (internal_auth.rs) | SECURE |
| MCP `/mcp` transport (57 tools) | JWT (sig+issuer+audience) then seam | `require_mcp_auth` (router.rs:66) + `ensure_profile_from_parts` → `authenticate_token` + `require_system_access`, **first line of every tool** (service.rs:78,85; 57↔57 verified) | SECURE |
| MCP `.well-known/*`, `/oauth/register`, `/mcp/health` | none | static metadata / pre-registered client_id, no DB, no user data | by-design unauth |
| TS `/oauth/token` (3 grants) | authz_code PKCE+single-use; refresh single-use rotation (hashed at rest); **client_credentials hashed secret + `timingSafeEqual`** | endpoints.ts / flow.ts / machine-clients.ts | SECURE |
| TS `/oauth/authorize` | client_id + redirect_uri allowlist; PKCE S256 + state required | clients.ts (`AS_CLIENTS` unset → `{}` → deny-all) | SECURE (public by design) |
| TS `/oauth/saml/acs` | SAML Response+Assertion signature validation pinned true + assertion-id replay guard + relay_state binding | config.ts / sp.ts / replay.ts | SECURE (public by design) |
| TS `/oauth/saml/{login,metadata}`, `/.well-known/*`, `/oauth/jwks`, `/api/auth/cli-callback` | none | discovery/metadata (no secrets); cli-callback relays code to `localhost:<port>` only, inert without PKCE verifier | by-design public |

### AuthN caveats (not holes)
- `/internal/saml/reconcile` identity resolution trusts the co-deployed AS to have validated
  the SAML assertion, plus secret secrecy — inherent to the server-to-server design; the HMAC
  gate enforces only-the-AS-can-call. It is the one place payload fields
  (`external_user_id`/`email`) drive profile resolution.
- `packages/temper-cloud/src/middleware.ts::authenticateRequest` (a bearer-JWT guard) is
  **dead code** — no entry point imports it. Orphan, not a hole; delete to avoid a future
  reader wiring a route to it assuming it is live.

---

## 2. AuthZ 2a — admin / machine / connection actions

**No self-elevation.** The two predicates, traced to SQL:
- `is_system_admin(profile)` = `owner` role on the configured gating team
  (canonical_functions.sql:1409-1425). **When `gating_team_slug IS NULL` (seed default) it is
  `false` for everyone** — fail-closed. There is **no first-admin self-serve HTTP path**; the
  first admin is minted by an operator against the DB. `has_system_access` returns `true`
  under `access_mode='open'` (prod), which is precisely why the per-endpoint service checks are
  **load-bearing, not defense-in-depth**.
- `machine_authz::authorize` = `is_system_admin` OR `Owner` of the machine's team; teamless
  fails closed.

Every admin / credential-minting endpoint (`promote`, admin `requests`/`settings`,
machine-clients `provision`/`issue`/`revoke`/`rebind`/`rotate-secret`, connections
`provision`/`revoke`/`credential`/`webhook-events`/`tool-manifest`/`reach`) gates **before**
any write. The Phase-B2 spec defenses are all **enforced in shipped code**:

- **D3/D4 — reach containment is TYPE-ENFORCED.** `apply_reach` takes `AuthorizedReach`, a
  struct whose fields are private to `machine_authz` with no public constructor; the only way
  to obtain one is `authorize_registration`, which runs `contain_reach` on the non-admin path.
  So `apply_reach` is **structurally uncallable without having passed containment**
  (machine_authz.rs:61-112). Containment calls the human predicates — `can_manage` for team
  grants, `profile_can_grant` for cogmap grants — so a machine can only receive reach ⊆ what
  the caller could confer on a human. Proxy-escalation is closed.
- **D4a** — machine team-role capped below `owner` on any team (machine_authz.rs:135-137).
- **D6** — minted grants keep `can_grant=false, can_delete=false` (non-re-delegable).
- **D7** — `add_member` refuses `role=Owner` (team_service.rs:191), mirrored in `change_role`.
- **D9** — `rebind` stays `is_system_admin`-only and refuses a revoked source (the one
  endpoint that transplants inherited reach; team-ownership cannot bound it).

---

## 3. AuthZ 2a — grants / share / role

Two axes, and they must be traced separately:

- **Authority acquisition (CLEAN).** A caller *without* grant authority cannot *acquire* it.
  `can_administer_grant = is_system_admin OR can(…,'grant',…)`, and the `'grant'` SQL arm is
  **owner-or-explicit-`can_grant` only** — read/write can **never** be laundered into grant
  authority (the `derived_access_profile` `'grant'` arm has no read/write fallthrough;
  `ELSE false`).
- **Capability attenuation (BROKEN on `main` — F-0).** A caller *with* grant authority can
  confer **more capability than it holds**, including to itself: `grant_capability` inserts the
  request's capability bits verbatim with no `conferred ⊆ held` check. This is the live
  self-escalation, fixed in draft #482. It is the axis the first pass failed to check.

Cross-surface: the MCP surface exposes `cogmap_*`/`resource_grant|revoke`/`*_context`/
invitation `accept|decline` and routes them through the **same shared service** as the HTTP
handlers — the "handler-only authZ bypassed by MCP" drift **does not exist** here (so F-0 is
equally reachable from MCP, not mitigated by surface). Team
`add_member`/`change_role`/`remove_member`, `promote_admin`, machine provisioning, and
`invitation create` are HTTP-only. Team role grants are attenuation-safe by a different
mechanism — `role=Owner` is hard-refused on `add_member`/`change_role` regardless of caller.

Invitation `accept` uses a 128-bit CSPRNG bearer token (not a guessable UUID); role is fixed
at creation and capped below owner.

---

## 4. AuthZ 2b — resource / edge RBAC on data mutation

Every resource/edge/facet/ingest write on **both** surfaces funnels through
`DbBackend::check_can_modify_next` → `can_modify_resource` **before** any write, keyed on the
**JWT-derived** `profile_id` (never caller input; no mutating tool has a `profile_id`/`on_behalf`
field). `can_modify_resource` requires the resource be live AND owned/originated, or a direct
write-grant, or a reachable-team write-grant, or the container-write cascade — ownership /
membership / capability, not mere authentication. Reads gate on `resources_visible_to`.
Async writers (steward delta/advance, dispatch tick) run with the **principal's own** access,
not ambient authority; the embed drain is a CRON-secret system backfill writing only derived
vectors for already-authorized rows.

Two by-design authority asymmetries surfaced — see F-1, F-2 below.

---

## 5. Findings (ranked)

F-0 is a live self-escalation on `main`. F-1…F-4 are risk items for an explicit decision.
Ordered by severity.

### F-0 — capability amplification / self-escalation via the grant path (HIGH — live on `main`, fixed in draft #482)
`grant_capability` (access_service.rs:189-214) gates on `can_administer_grant` (admin OR
`can_grant` on the subject), then calls `insert_grant` with the capability bits **taken
verbatim from the request** (`can_read/can_write/can_delete/can_grant`). Nothing checks that
the caller *holds* the capabilities it is conferring, and `principal_id` may be the caller's
own profile. **A principal with `read+grant` on a subject can call `grant_capability` with
`principal_id = self, can_write=true, can_delete=true, can_grant=true` and escalate itself to
full control.** The only backstop is the DB coherence CHECK (`write|delete|grant ⇒ read`),
which is not attenuation. The same blind spot exists on every direct `insert_grant` caller
(`connection_service::grant_reach`, `machine_registration_service::apply_reach`, cogmap
genesis). Fix (shipped in draft PR #482, Task 5b): a SQL-resident chokepoint that attenuates —
"delegated administrators may confer only what they hold; system admins stay unrestricted" —
plus bounding `grant_reach`'s grantee team. **Recommend: land #482.** Until it merges, this is
the live form of the exact class that triggered the audit.

### F-2 — create-into-context is gated on context **READ**, not context **WRITE** (medium)
`create_resource_inner` re-enforces the container write-gate for **cogmap** homes only
(db_backend.rs:1211-1212); create-into-context authority rests on the surface's
`resolve_context_ref` → `context_visible_to`, which delegates to `context_readable_by_profile`
(the READ set). Read is strictly broader than `context_authorable_by_profile` (WRITE): it
inherits up the team-enclosure chain and includes the `watcher` role and read-only grants.
**Net: a read-only context member (watcher, transitive-ancestor member, or read-only grant)
can create a resource homed in that context**, which every context reader then sees. This
re-opens the exact read-wider-than-write axis that migration `20260712000010` closed for the
*modify* path. Recommend: gate create-into-context on `context_authorable_by_profile`, matching
the cogmap path — or explicitly sign off that read-authority-to-place-content is intended.

### F-3 — `promote_admin` authorization lives in the **handler**, not the service (low, latent)
`access.rs:222` checks `is_system_admin` before calling `access_service::promote_admin`, whose
own doc-comment says "Auth is enforced by the caller." This is **the exact drift shape this
audit was commissioned to find.** Not currently exploitable — no MCP tool or other caller
reaches `promote_admin`. But any future second caller (an MCP admin tool, an internal endpoint)
that calls the service directly would grant system-admin with **no** authorization. Recommend:
move the `is_system_admin` gate inside `promote_admin`, consistent with every other grant op,
so authority is enforced at the write, not at one call site.

### F-1 — edge asserts authorize the **source** only, never the **target** (low, by-design)
`assert`/`retype`/`reweight`/`fold` gate `check_can_modify_next(source)`; the target endpoint
is never authorized (db_backend.rs:1755+). A caller who can modify A can attach an outbound
edge A→B to any B by id, including resources they cannot read or modify. Blast radius is
contained: the edge is homed on A's home, does not mutate B, and `edges_visible_to` requires
**both** endpoints visible to any viewer (so an edge to an invisible target is invisible even
to its creator). Documented as production parity. Recommend an explicit sign-off that
one-sided edge authorship is intended.

### F-4 — `change_role`/`remove_member` don't compare caller rank to target rank (low)
A **maintainer** may demote or remove an **owner** (except the last owner, which is guarded)
— team_service.rs:387-529. Downward/griefing, not self-elevation (a maintainer still cannot
make themselves owner). Flagged only because it lets a lower role act on a higher one; if the
intent is "only owners manage owners," it is under-gated.

### T-1 — forward tripwire: T4 `mint_access_token` caller (not yet built)
The Slack grant vault's `mint_access_token` enforces **no** authorization itself — it mints an
act-as-the-human token for whatever `slack_principal_id` it is handed
(slack_grant_vault_service.rs:182-185). **There is no production caller today** (only tests);
the T4 "act-as-human" mention path is unbuilt. Whoever implements T4 **must** derive the
principal from the HMAC-verified server-to-server Slack event, never a client-supplied field —
otherwise it is act-as-any-user token theft. Gate the T4 PR review on this invariant. (The
*store* side is already safe: the callback derives the principal from the consumed, HMAC-gated,
single-use link intent.)

## 6. Credential-flow (Phase 3) — verified so far
- **Slack grant vault crypto:** XChaCha20-Poly1305, fresh nonce per seal, **AAD binds
  `principal‖field`** so a row/field ciphertext transplant fails the tag (tested end-to-end),
  fail-closed decrypt, redacted `Debug` on key + token types. **Sound.**
- **OAuth `/token`:** PKCE verified constant-time before the code is burned; codes client-scoped
  + single-use; refresh single-use rotation, hashed at rest; client_credentials constant-time.
- **SAML ACS:** Response + Assertion signatures required, assertion-id replay guard, relay_state
  one-time binding.
- **Internal HMAC gates + embed secret:** constant-time, fail-closed on unset/empty, 30s replay
  window; two internal gates use **separate** secrets so neither principal can forge the other.
- **Machine classification:** closed sum, refuses incoherent machine-shaped tokens.

### Remaining for a full Phase 3 adversarial pass
- Cross-surface audience/issuer parity under JWKS rotation + algorithm-confusion (the
  `auth_seam_parity_e2e` concern), M2M token replay window, and a consolidated per-flow threat
  model. Both surfaces read the one audience off the shared `AuthConfig` (a known prior
  divergence, now unified) — worth an explicit adversarial confirmation.
