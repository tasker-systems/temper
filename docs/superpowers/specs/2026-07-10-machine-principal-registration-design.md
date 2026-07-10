# Machine-principal registration, rotation, and revocation

**Date:** 2026-07-10
**Goal:** `019f4910` — temper-rb, a native Ruby client for the temper API
**Beat:** G3 — a machine-principal auth path for SDK callers (`client_credentials`)
**Status:** design approved; Phase A is the current build target, Phase B is deferred

---

## Problem

Temper already authenticates machine principals. It does not *govern* them.

`normalize_machine` (`crates/temper-services/src/auth/normalize.rs`) detects an OAuth
`client_credentials` token by its `gty` claim, resolves the client id from `azp` (falling back to
stripping the `@clients` suffix off `sub`), and emits `AuthClaims` under the `auth0-m2m` provider
namespace. `resolve_machine_from_claims` (`crates/temper-services/src/services/profile_service.rs:120`)
then looks the client id up in `kb_profile_auth_links` and, **on a miss, creates a fresh agent
profile**. There is no allowlist. Any principal who can create an M2M application in the Auth0
tenant and grant it the temper API audience becomes a machine principal in temper, silently, on
first call.

Four consequences follow, and they are the substance of this design:

1. **Temper cannot enumerate its machine principals.** There is no table of them. The nearest thing
   is `SELECT ... FROM kb_profile_auth_links WHERE auth_provider = 'auth0-m2m'`, which records that a
   machine authenticated, not that anyone authorized it.

2. **Temper cannot revoke one.** Revocation means deleting the application in the Auth0 dashboard.
   That is an out-of-band action against a third-party control plane, invisible to temper's ledger,
   and unavailable at all to a self-hosted instance not fronted by Auth0.

3. **Temper does not record who authorized a machine, or when, or why.** Accountability terminates
   at "someone with Auth0 dashboard access." The chain from a written resource back to a human
   decision is broken at the machine boundary.

4. **Rotating the Auth0 *application* silently forks the machine's identity.** A new `client_id` is
   a new `(auth0-m2m, client_id)` link, which JIT-creates a *second* agent profile with its own
   emitter entities. Every event the machine wrote under the old client id remains attributed to a
   profile the new credential cannot act as. Nothing errors. The corruption is invisible until
   somebody asks "what has this agent written" and receives half an answer.

### What production actually contains (verified 2026-07-10, `temper-cloud/main`)

Read-only queries against the production database, run while writing this spec:

- **Exactly one `auth0-m2m` auth link exists**: client id `y23AQxuvzjYSb5n8lAUeuIgIXOftCWYu`, profile
  `agent-y23aqxuvzjysb5n8laueuigixoftcwyu`, linked `2026-07-03`. That client id is the same one
  pinned in `normalize.rs`'s known-answer test. It is the steward, and it is the only machine
  principal. The backfill's assumption holds.
- Its four per-surface emitter entities (`@cli`, `@mcp`, `@sdk`, `@web`) are all present.
- **Its cogmap write grant is on a specific cogmap**, `019f2391-…` / *Temper — self-cognition*:
  `kb_access_grants(subject_table='kb_cogmaps', can_read, can_write)`. Not a team-scoped grant.
- **It holds three team memberships**: `owner` of its own auto-provisioned personal team, `watcher`
  on `temper-system`, and `member` of `personal-j-cole-taylor` (hand-added for read reach).

That last pair of facts is the concrete vindication of treating `team_id` as *owner* and not as
*reach*: the steward's reach is three memberships and one cogmap grant. A single `team_id` column
would describe a third of it.

### The gate is the only line, not the second line

`kb_system_settings.access_mode` is **`open`** in production. `has_system_access` short-circuits to
`true` for every profile under that mode, and `trg_sync_system_membership` — firing on every
`kb_profiles` insert — then auto-joins the new profile to the `temper-system` gating team as
`watcher`. (`trg_sync_personal_team` likewise mints a personal team. Both are database triggers, so
they fire wherever the profile is created; the inversion below does not have to reproduce them.)

The consequence: **`require_system_access` passes for everyone in production today**, including any
profile JIT-created by a machine token. So the sequence available right now to anyone who can mint an
M2M token for the temper audience in the Auth0 tenant is: authenticate → get a profile → get emitter
entities → get system access → pass the gated router. There is no second line behind the missing
allowlist.

Two things follow. First, this raises Phase A from hygiene to the actual control. Second, the
`is_system_admin` check on the registration endpoints is **load-bearing, not defense-in-depth** —
placing those routes in the system-gated router buys nothing while `access_mode` is `open`.
`is_system_admin` resolves to *owner of the gating team*, which is exactly one profile
(`j-cole-taylor`), so the check is sound; it simply must not be omitted on the assumption that the
router already gated it.

Point 4 above is the sharpest of the four. Note the asymmetry it sits inside: rotating the *client secret* is
completely free — the `client_id` is unchanged, so the auth link, the agent profile, its emitter
entities, and its entire authorship history stay stitched together, and temper never needs to know
it happened. That property is already true today and this design preserves it. It is only
application-level rotation, where the `client_id` itself changes, that has no safe path.

### The onboarding cliff

Bringing the steward — `packages/agent-workflows/steward/`, the reference M2M caller, deployed and
ticking hourly — to production required provisioning an Auth0 M2M application and client-grant,
setting `TEMPER_M2M_{TOKEN_URL,CLIENT_ID,CLIENT_SECRET,AUDIENCE}`, and then, *separately*, granting
the JIT-provisioned agent profile a cogmap write grant and team membership for read reach.
Authentication alone bought nothing: without the grants, every call authenticated and then 403'd.

This is a runbook, executed by hand, whose failure modes are a 401 and a 403 that look like bugs.
Every first temper-rb user hits it. It is not documentation's job to fix.

### What this problem is not

The task note that opened this design asked whether temper needs `pgcrypto` and a secrets-management
table, motivated by proliferation of `EMBED_DISPATCH_SECRET`, `CRON_SECRET`,
`INTERNAL_RECONCILE_SECRET`, and the four `TEMPER_M2M_*` variables. Sorting those into their actual
classes dissolves most of the question, and the sorting is worth recording because the instinct to
re-conflate them is strong:

- **Bootstrap secrets** — `EMBED_DISPATCH_SECRET`, `CRON_SECRET`, `INTERNAL_RECONCILE_SECRET`. The
  API holds these to authenticate callers that are the same deployment. They **cannot** live in the
  database: they gate access to the process that would read the database, and `pgcrypto`'s
  decryption key would itself have to live in an environment variable. Moving them buys one env var
  for one env var, plus a database round-trip on every cron tick, plus a new fail-closed dependency.
  These stay in the environment. (`internal_auth.rs` is worth studying as the good case: an HMAC
  over `{timestamp}.{raw_body}`, so the secret never crosses the wire and a captured request is
  replay-proof. `EMBED_DISPATCH_SECRET` is a plain bearer comparison by contrast. That asymmetry is
  real, but it is a *scheme* difference, not a *storage* one, and it is out of scope here.)

- **Caller-held credentials** — `TEMPER_M2M_CLIENT_SECRET`. This lives with the caller: the
  steward's Vercel project today, a customer's Sidekiq deployment tomorrow. Temper never sees it,
  and must never see it. Auth0 holds a hash.

- **Issuer-held credential material** — nothing today. Under Phase B, temper would hold an argon2id
  **hash** of each client secret. A hash, not ciphertext: temper never needs the plaintext back, so
  a KDF strictly dominates encryption, and `kb_oauth_refresh_tokens.token_hash` already models this
  exactly. `pgcrypto` would be indicated only if temper had to *replay* a third-party secret it
  holds on someone else's behalf. It has no such need, and acquiring one should be resisted.

**No table stores a secret in either phase. `pgcrypto` is not adopted.** The problem is
registration, revocation, and identity continuity — a governance problem wearing a cryptography
costume.

---

## Design

Two phases. Phase A ships now and is sufficient to unblock temper-rb's `Credentials::ClientCredentials`.
Phase B is a separate task, deferred until after Phase A deploys.

The phases share one table, landed in its final shape by Phase A. Phase B adds columns; it does not
reshape or migrate what A wrote.

### Phase A — registration as a gate

Temper remains a **verifier**. Auth0 (or, for a self-hosted instance, whatever IdP fronts it) keeps
issuing tokens. What changes is that temper now decides *which* machine principals it will accept,
and records who decided.

#### The table

```sql
CREATE TABLE kb_machine_clients (
    id                      UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    client_id               TEXT        NOT NULL UNIQUE,
    issuer                  TEXT        NOT NULL DEFAULT 'auth0-m2m',
    label                   TEXT        NOT NULL,
    profile_id              UUID        NOT NULL REFERENCES kb_profiles(id),
    team_id                 UUID            NULL REFERENCES kb_teams(id),
    registered_by_profile_id UUID       NOT NULL REFERENCES kb_profiles(id),
    created                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at            TIMESTAMPTZ     NULL,
    revoked_at              TIMESTAMPTZ     NULL,
    revoked_by_profile_id   UUID            NULL REFERENCES kb_profiles(id)
);
```

- **`client_id`** is `UNIQUE`; its constraint index serves the authentication-path lookup. It is the
  IdP's client identifier, matching `AuthClaims.external_user_id` as produced by `normalize_machine`.

- **`issuer`** distinguishes an IdP-issued client from a temper-issued one. Phase A writes only
  `'auth0-m2m'`, matching `MACHINE_PROVIDER_TAG`. Phase B writes `'temper'`. It exists in A so that
  B is additive.

- **`profile_id`** is the agent profile this client acts as. `NOT NULL`: under Phase A the profile
  is created *at registration*, never on the authentication path (see "The inversion" below).

- **`team_id` is the machine's OWNER, and is never consulted for authorization.** It records which
  team a registration was performed on behalf of, so `provision` can report what it did and an
  operator can answer "whose machine is this." It is emphatically **not** the machine's *reach*.
  Reach is `kb_access_grants` plus team membership, both plural, and a useful agent will span more
  than one team — the steward, concretely, holds three memberships and one cogmap grant. A single FK
  reads like reach, and if anyone ever writes an authorization predicate against it, that predicate
  will be strictly narrower than `resources_visible_to` — the read-gate subset bug this codebase has
  already been bitten by. The column comment in the migration must say this. Nullable, because a
  machine need not belong to a team. It is **never** the agent's own auto-provisioned personal team,
  which `trg_sync_personal_team` creates for every profile and which carries no meaning here.

- **`last_seen_at`** is written on the authentication path and is the only field here that is not
  load-bearing for the gate. Its write is made **coarse**: update only when the stored value is
  `NULL` or older than five minutes. The common case is therefore a pure read, preserving the
  property that authentication does not mutate. Precision beyond five minutes has no consumer.

- **`revoked_at` / `revoked_by_profile_id`** are the revocation record. A revoked row is dead;
  reactivation is a new registration, not an `UPDATE`. Rows are never deleted — the ledger's value
  is that it retains what was once true.

Indexes: the `client_id` UNIQUE constraint index (authentication path), plus
`(profile_id)` and `(team_id) WHERE team_id IS NOT NULL` for the list/show surfaces.

#### The gate

`resolve_machine_from_claims` loses its create branch and becomes lookup-or-reject:

1. Look up `kb_machine_clients` by `client_id`.
2. Miss → reject: this client is not registered with this instance.
3. `revoked_at IS NOT NULL` → reject: this client was revoked, at that timestamp.
4. Hit → load `profile_id`'s profile, coarsely touch `last_seen_at`, return.

**The gate lives in `temper-services`, inside `resolve_machine_from_claims` — not in an Axum
middleware.** Both surfaces (temper-api and temper-mcp) route every machine principal through that
one function. Placing the check there makes gate drift between the surfaces *structurally
impossible* rather than a thing to remember, and temper-mcp inherits the gate with no diff. This is
the direct application of a lesson already paid for: any auth change that touches one surface must
touch both, so the correct fix is to make there be only one place.

#### The inversion

`create_agent_profile_and_link` (`profile_service.rs:298`) and `provision_profile_entities`
(`:340`) move out of the authentication path and into a new registration service. Registration
creates the agent profile, its `(auth0-m2m, client_id)` auth link, and its per-surface emitter
entities — all in one transaction with the `kb_machine_clients` row.

Two things fall out:

- **Authentication no longer writes.** A database write on the authentication path was always
  slightly wrong. Removing it is a correctness improvement independent of everything else here.
  (`last_seen_at` reintroduces a bounded, coarse write; see above. The distinction is that a
  five-minute-coarse touch cannot fail a request in a way that matters, whereas profile creation
  could.)

- **`provision` becomes one command.** Under a design that kept JIT provisioning, the agent profile
  would not exist until the machine's first authenticated call, so there would be nothing to grant
  cogmap write to and nothing to add to a team. Onboarding would remain a runbook with a "wait for a
  cron tick" step wedged into the middle of it. Inverting the order is what collapses the runbook.

`provision_profile_entities` is idempotent and concurrency-safe (`ON CONFLICT DO NOTHING` against
the unique index from `20260709000040`) and has tests asserting both. It is called from a new
caller, unchanged.

#### Error surfaces

The gate runs **after** JWT verification. A caller reaching it has already proven it holds a valid,
unexpired, correctly-audienced token signed by the trusted issuer. Telling that caller *why* it was
rejected therefore leaks nothing it does not already know, and it is the difference between a
five-minute fix and a lost afternoon.

| Condition | Response |
|---|---|
| Token fails signature / expiry / audience verification | `ApiError::Unauthorized` — flat, no detail |
| Valid token, `client_id` not in `kb_machine_clients` | `ApiError::Unauthorized`, message naming the client id as unregistered with this instance, and naming `temper admin machine provision` |
| Valid token, client registered but `revoked_at` set | `ApiError::Unauthorized`, message naming the client id and its revocation timestamp |

An opaque 401 covering all three is the status quo's worst ergonomic property and is explicitly
rejected. The messages name the remedy.

#### CLI surface

`temper admin machine <subcommand>`, alongside the existing `temper admin` surface
(`crates/temper-cli/src/commands/admin.rs`) and `temper admin saml` (`admin_saml.rs`), which it
follows for shape.

| Subcommand | Behavior |
|---|---|
| `provision --client-id <id> --label <l> [--owner-team <ref>] [--team <ref>[:role]]... [--cogmap <ref>[:rw]]...` | Create the agent profile, its auth link, its emitter entities, and the `kb_machine_clients` row; add each `--team` membership; apply each `--cogmap` grant. `--owner-team` records `team_id`. One transaction. |
| `rebind --client-id <new> --to <machine-ref> [--no-revoke-old]` | Register `<new>` against the **existing** agent profile of `<machine-ref>`, and revoke the old client's row. One transaction. `--no-revoke-old` leaves both credentials live for an overlap window. |
| `list [--include-revoked]` | Enumerate registered clients: client id, label, agent profile handle, owning team, last seen, revocation state. |
| `show <ref>` | One client in full, including its grants. |
| `revoke <ref>` | Set `revoked_at` / `revoked_by_profile_id`. Non-interactive, matching `resource delete`. |

**Reach is plural, and `provision` must model it as plural.** The steward's live configuration is the
worked example: a `can_read`/`can_write` grant on one specific cogmap (`kb_access_grants` with
`subject_table = 'kb_cogmaps'`), plus membership in a human's personal team for read reach. Neither
is inferable from an owning team, so neither `--cogmap` nor `--team` may be inferred from
`--owner-team`. `--team` and `--cogmap` are repeatable; `--team` takes an optional `:role` defaulting
to `member`; `--cogmap` takes an optional `:rw` defaulting to `rw` (a read-only agent is unusual but
expressible). The `temper-system` watcher membership and the personal team are trigger-created and
must not be passed.

**Revocation does not strip reach.** `revoke` sets `revoked_at` and nothing else: the agent profile
keeps its grants and memberships. This is deliberate, and it is what makes `rebind` work — the new
credential inherits the reach the old one had, because the reach hangs off the *profile*, not the
client. An operator who wants the reach gone too must say so explicitly, by revoking the grants and
memberships as separate actions. Anyone reading `revoke` as "deprovision this machine" will be
surprised; the CLI help text must say which one it is.

`rebind` is the answer to problem 4. A fresh `client_id` is pointed at the agent profile that
already exists, and the old row is revoked in the same transaction. Authorship history stays
continuous across an Auth0 application rotation, and there is a window of exactly zero in which both
credentials are live. An operator who wants an overlap window instead — both credentials valid while
the new one rolls out — passes `rebind --no-revoke-old` and runs `revoke` against the old client
afterward. Secret rotation, as established, requires no temper action at all.

#### API surface

`POST/GET /api/machine-clients`, `GET/DELETE /api/machine-clients/{id}`, and
`POST /api/machine-clients/{id}/rebind`.

Authorization is `is_system_admin` (`access_service.rs:43` → the SQL `is_system_admin`, i.e. *owner of
the gating team*), checked **explicitly in the handler**. Routes also sit in the system-gated router,
but as established above that router admits every profile while `access_mode = 'open'`, so the
explicit check is the only thing protecting these endpoints. It is not optional and not redundant.

**Phase A's authorization is system-admin only, deliberately** — see "Deferred" below. The path is
chosen so that Phase B widens the *predicate*, not the *route*.

Handlers stay thin: validate, dispatch one service call, serialize. SQL lives in a new
`machine_client_service` in `temper-services/src/services/`. Per the repository's persistence rule,
no `sqlx::query!()` appears in a handler, and writes route through the service.

#### Migration and deploy ordering

One additive migration, numbered with a gap above the current head (`20260709000050`) to leave room
for concurrent sibling sessions: **`20260711000010_machine_clients.sql`**.

It creates the table, its indexes, and its column comments, and then **backfills** the steward:

```sql
INSERT INTO kb_machine_clients (client_id, issuer, label, profile_id, registered_by_profile_id)
SELECT l.auth_provider_user_id, 'auth0-m2m', 'backfilled: ' || p.handle, l.profile_id, l.profile_id
  FROM kb_profile_auth_links l
  JOIN kb_profiles p ON p.id = l.profile_id
 WHERE l.auth_provider = 'auth0-m2m'
ON CONFLICT (client_id) DO NOTHING;
```

`registered_by_profile_id` is set to the agent's own profile: no human authorized these, and
recording a fabricated human would be a lie in the accountability ledger. The `label` marks them as
backfilled. Any operator auditing the table can see exactly which rows predate governance. This is
the honest encoding of "we did not know who authorized this."

**Ordering is migrate-then-deploy.** Migrate-ahead-of-deploy is inert: the table exists and nothing
reads it. Deploy-ahead-of-migrate 500s **every machine call** against a missing relation. The
migration must therefore land first, and the additive-only-on-`main` invariant holds it does.

Blast radius of the skew window, assessed against production: `temperkb.io`'s only machine principal
is the steward, on an hourly cron over our own corpus. A missed tick is picked up on the next one.
The enterprise instance has no machine principals yet — its Vercel Eve flow is still in internal
security review. The window is therefore safe to accept without a maintenance ritual.

#### Testing

- **`normalize_machine`** is untouched; its known-answer test pinning the real Auth0 M2M claim shape
  continues to guard the boundary.
- **Gate unit tests** (`#[sqlx::test]`, `test-db`): registered client resolves; unregistered client
  rejects; revoked client rejects; the rejection *messages* distinguish the two.
- **Inversion test:** authentication no longer creates a profile. Assert that a valid machine token
  for an unregistered client leaves `kb_profiles` unchanged. This is the bite test — under the old
  code it fails by finding a newly created row.
- **`rebind` test:** after rebinding, the new client id resolves to the *same* `profile_id`, the old
  row is revoked, and events written under the old client id remain attributed to that profile. This
  is the regression test for the silent identity fork.
- **`last_seen_at` coarseness:** two authentications inside five minutes produce one write.
- **Backfill test:** an `auth0-m2m` auth link with no machine-client row yields exactly one
  backfilled row, and re-running the migration is a no-op.
- **e2e**, per `feedback_access_semantics_changes_need_e2e_tier`: a `test-db`-green result is a false
  signal for a change to authentication semantics. `cargo make test-e2e` must exercise a machine
  principal end-to-end against a real Axum server. Both surfaces authenticate machines, so
  temper-mcp gets a case too.

---

### Phase B — temper as an issuer

Deferred to its own task, after Phase A deploys. Recorded here because A's schema is shaped by it.

Phase B is less speculative than it sounds. **Temper already runs an OAuth Authorization Server**:
`packages/temper-cloud/src/oauth/` mints Ed25519-signed JWTs (`mint.ts`, via `jose`), publishes a
JWKS (`keys.ts`, `metadata.ts`, keyed by `AS_SIGNING_KEY_PKCS8` / `AS_SIGNING_KID`), and rotates
single-use refresh tokens with a revocation chain (`kb_oauth_refresh_tokens`). It supports
`authorization_code` and `refresh_token`, and returns `unsupported_grant_type` for everything else
(`endpoints.ts`). Its client registry, `AS_CLIENTS`, is a public-client redirect-URI allowlist with
no secrets — appropriate for PKCE, insufficient for `client_credentials`.

Phase B adds a third grant to that surface.

**Motivation.** Auth0 bills and rate-limits per M2M application, so "one application per tenant
agent" does not scale. A self-hosted instance not fronted by Auth0 has no way to mint a machine
credential at all today. And revocation, under Phase A, is temper-side deny of an IdP-issued token
that remains valid at the IdP — correct, but not the same as never issuing it.

**Shape.**

- `kb_machine_clients` grows `secret_hash TEXT NULL` and `secret_rotated_at TIMESTAMPTZ NULL`.
  Rows with `issuer = 'temper'` carry a hash; `issuer = 'auth0-m2m'` rows carry `NULL` and continue
  to verify against Auth0's JWKS. The two verification paths coexist, keyed on `issuer`.
- The hash is **argon2id**. Never encryption: temper does not need the plaintext back, so a KDF
  strictly dominates, and there is no key to manage. `kb_oauth_refresh_tokens.token_hash` is the
  precedent already in the schema.
- **Two live secrets per client** for zero-downtime secret rotation: `secret_hash` plus
  `secret_hash_previous` with its own expiry. Issue the new secret, deploy it to the caller, retire
  the old one — no window in which the caller has no valid credential. Under Phase A this problem
  does not exist because Auth0 owns it.
- `POST /oauth/token` with `grant_type=client_credentials` authenticates
  `(client_id, client_secret)` against the hash and mints an access token whose claims carry
  `gty: "client-credentials"` and `azp: <client_id>` — **the same shape `normalize_machine` already
  detects**, verified against the known-answer test. The verifier does not change. The Phase A gate
  runs unmodified.
- The plaintext secret is returned **once**, at issuance, and never again. It is not stored.

**Also in Phase B, and independent of the grant: widen registration authorization from
`is_system_admin` to `is_system_admin OR is_team_owner(team_id)`.** This is a one-line change to a
predicate plus tests, with **no migration** — `team_id` already exists, having landed in Phase A.
The two halves of Phase B are independent and may ship in either order.

---

## Decisions

- **D1 — `pgcrypto` is not adopted, and no table stores a secret.** Bootstrap secrets cannot live in
  the database they gate. Caller-held secrets are the caller's. Issuer-held material is a KDF hash.
  The problem was governance, not cryptography.

- **D2 — Registration is a gate, not a ledger.** An unregistered `client_id` is rejected even with a
  perfectly valid IdP token. A ledger that is not enforced is a log, and the event ledger already
  exists. Revocation, rotation-with-continuity, and accountability are delivered *only* by the
  fail-closed check.

- **D3 — Registration creates the agent profile; authentication does not.**
  `resolve_machine_from_claims` becomes lookup-or-reject. This removes a write from the
  authentication path and is the precondition for `provision` being a single command.

- **D4 — The gate lives in `temper-services`, not in middleware.** One function, both surfaces, no
  drift by construction.

- **D5 — Phase A takes the multi-tenant *schema* but keeps the single-admin *authorization*.**
  `team_id` lands now, nullable, recorded by `provision --team`. The widening of the registration
  predicate from
  system-admin to team-owner is deferred: it is where the security surface actually changes, it is
  where an enterprise security review will spend its attention, and that review is still open. It
  costs nothing to defer, because it requires no migration.

- **D6 — `team_id` is the machine's owner, never its reach.** Enforced by column comment and by this
  document. Reach remains `kb_access_grants` plus team membership.

- **D7 — Post-verification rejections are specific.** Unregistered and revoked are distinguishable to
  a caller that has already proven it holds a valid token. Unverifiable tokens get a flat 401.

- **D8 — `rebind` preserves identity across application rotation.** A new `client_id` binds to the
  existing agent profile; the old row is revoked in the same transaction. Secret rotation continues
  to require no temper action whatsoever.

- **D9 — `last_seen_at` is coarse (five minutes).** It is the only non-load-bearing column, and its
  coarseness preserves D3's "authentication does not write" in the common case.

- **D10 — Reach is plural and always explicit.** `provision` takes repeatable `--team` and `--cogmap`
  and infers neither from `--owner-team`. The steward's live configuration — one cogmap-scoped grant
  plus three team memberships, two of them trigger-created — is the proof that no single-valued
  inference is correct.

- **D11 — `revoke` denies authentication and nothing else.** Grants and memberships hang off the
  agent *profile*, not the client, and survive revocation. This is what lets `rebind` inherit reach.
  Stripping reach is a separate, explicit act.

- **D12 — The `is_system_admin` check on the registration endpoints is load-bearing.** Production runs
  `access_mode = 'open'`, under which `require_system_access` admits every profile. The router is not
  a gate; the explicit check is.

- **D13 — Backfilled rows are honestly labeled.** `registered_by_profile_id` is the agent's own
  profile, not a fabricated human. The ledger records that nobody authorized these.

## Rejected

- **A `client_id → arbitrary existing profile` binding**, letting a machine credential act as a
  human's profile. This collapses the human/machine distinction `normalize_machine` exists to
  preserve, and it means a leaked client secret impersonates a *person* rather than a bounded agent.
  `rebind` permits binding only to an existing **agent** profile — one already reached through a
  machine auth link — which is the narrow, safe case.

- **Storing secrets encrypted with `pgcrypto`.** The decryption key would live in an environment
  variable beside the ciphertext, and temper never needs a machine secret in plaintext. See D1.

- **Keeping JIT provisioning with an advisory (fail-open) registry.** Delivers none of rotation,
  revocation, or accountability; produces a table that describes what already happened.

- **Enforcing the gate in an Axum middleware.** temper-mcp does not share temper-api's middleware
  stack. See D4.

## Deferred

- **Phase B in full** — the `client_credentials` grant on the existing AS, and the team-owner
  widening. Its own task, after Phase A deploys.

- **Harmonizing `EMBED_DISPATCH_SECRET`'s bearer comparison with `INTERNAL_RECONCILE_SECRET`'s
  HMAC-over-body scheme.** A real asymmetry, correctly identified, entirely orthogonal to machine
  principals. It deserves its own task and should not ride along.

## Open questions and risks

- **RESOLVED — the backfill set is exactly the steward.** Verified against `temper-cloud/main` on
  2026-07-10: one `auth0-m2m` auth link, whose client id matches `normalize.rs`'s known-answer test.
  No unauthorized machine has ever authenticated. Re-verify immediately before running the migration;
  the window between now and then is small but not zero.

- **RESOLVED — `provision` cannot infer the cogmap.** The steward's grant is on a specific cogmap,
  not on a team, so `--cogmap` is explicit and repeatable. See "CLI surface."

- **`access_mode = 'open'` is a separate exposure, and Phase A does not close it.** Under `open`,
  every profile — human or machine, invited or not — passes `require_system_access`. Phase A means an
  unregistered machine can no longer *get* a profile, which removes the machine path into that
  exposure. It does nothing about the human path. Whether `open` is still the right production
  setting is a real question, and it is not this task's to answer. It should be raised on its own.

- **`rebind`'s transaction spans a profile that may have in-flight requests** authenticating under
  the old `client_id`. Those requests resolve the old row, which is being revoked. Postgres'
  read-committed default means an in-flight authentication either sees the pre-revocation row or the
  post-revocation one; both are safe (the latter 401s, and the caller retries with the new
  credential). No lock is needed, but the test should assert the interleaving.

- **temper-rb's `Credentials::ClientCredentials` (D12 of the gem design) is unaffected by the phase
  split.** The gem mints against a token URL either way, and the contract it depends on — the env
  surface, the machine-identity-first precedence, `expires_at` plus a 60-second skew, and
  re-mint-on-401 — is settled by the steward's `temper-auth.ts` and is orthogonal to who issues the
  token. Phase A does add one thing the gem must surface: a rejected-because-unregistered 401 is
  `Permanent`, not `Transient` (D13), and must not be retried by Sidekiq.
