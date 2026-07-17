# T2 — the Slack↔temper account-link flow

Design for task `019f6344-bace-7200-8d36-2c7da0d4267c` (Phase 1 · T2). Grounding research:
`019f6343-52b4-7f61-8561-cb30601b9681`. Depends on T1 (`019f6344-80ce-7142-a116-5e39a04eeb3e`,
done — `@temper` is live in Slack and resolves `slack:<team>:<user>`).

A Slack user proves their **temper** identity once in a browser; temper writes the directory row
`slack:<team>:<user> → profile`. This is the identity-binding half of approach (B).

## Boundary

| Task | Owns |
|---|---|
| T1 | the mention shell + `slack:<team>:<user>` resolution — **done** |
| **T2** | **the link: server-side OAuth redirect + the `auth_provider='slack'` row** |
| T3 | the per-user grant vault: encrypt, store, refresh |
| T4 | reads under proven identity |
| T5 | writes + in-thread HITL confirm |

T2 *obtains* the grant and hands the refresh token to T3's vault behind a seam. It does not
persist it. Identity (the row) and secret (the vault) stay in separate tables —
`kb_profile_auth_links` has no secret column and must not grow one.

## Flow

```
Slack: @temper …                 (principal slack:T0BH…:U0BH… from attributes.user_id)
  → agent: POST /internal/slack/link-state        [HMAC, ≤30s skew]
      → SELECT kb_profile_auth_links ⋈ kb_profiles  WHERE (slack, principal)
      ├─ row exists ⇒ ← { status: "linked", handle }         NO intent minted
      │    → agent: ctx.thread.postEphemeral(user_id, linkedPrompt(handle)) → drop
      └─ no row ⇒
           → generate_pkce_pair(); INSERT kb_slack_link_intents
             (state_nonce, code_verifier, slack_principal_id, expires_at)
           ← { status: "unlinked", authorize_url }  (IdP url, state=nonce, S256, offline_access)
  → agent: ctx.thread.postEphemeral(user_id, authorize_url)   ← slack.ts:36 changes here
  → user: browser → IdP  (Auth0 Universal Login | AS → SAML)
  → IdP → GET /api/auth/slack/callback?code=&state=
      → UPDATE … WHERE state_nonce=$1 AND consumed_at IS NULL
                  AND expires_at > now() RETURNING code_verifier, slack_principal_id
        (0 rows ⇒ unknown | expired | replayed — one indistinguishable rejection)
      → exchange_code(code, verifier, redirect_uri) → { access_token, refresh_token }
      → resolve profile LOOKUP-ONLY from the token's sub   (refuse if absent)
      → upsert kb_profile_auth_links
        ON CONFLICT (auth_provider, auth_provider_user_id) DO UPDATE SET profile_id
      → [T3 seam: refresh_token → vault]
      ← 200 text/html  "✅ Linked as @j-cole-taylor"
```

## Decisions

### D1 — the grant is T2's own PKCE grant, not a Management-API mint

The callback's `code`→token exchange requests `offline_access` and therefore **already returns a
refresh token**, in its own independent grant family. That is exactly what the research's
correction #2 demands: each Slack link is its own grant, never an export of the user's local CLI
grant (sharing one RT across sessions trips Auth0 RT-rotation reuse-detection and kills the whole
family — `temper-client/src/auth.rs:586-610`).

The research points T3 at Unit D's Auth0 **Management API** mint
(`docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md:84-88`). That blueprint was written
for a different problem — minting a session for a user who is **not** in a browser
(`temper auth create-cloud-session`). T2 has a browser and a consent screen, which is the natural
moment to obtain `offline_access`.

Consequences: no M2M Management credentials in the server, and the `parse_jwt_claims` latent bug
(`temper-client/src/auth.rs:166` — silently yields `None` when `sub` is `auth0|…` rather than a
profile UUID, flagged at `2026-04-19-cloud-mode-auth0-design.md:159-164`) **never fires**, because
that shape only arrives on the Management-API path. T2's "grant received" is therefore the real
thing, not a placeholder: T3 becomes "encrypt, store, refresh".

### D2 — home: Rust, in temper-api

**The TypeScript `packages/temper-cloud/src/oauth/` surface is the *server* half of OAuth; T2 is the
*client* half.** That surface implements the Authorization Server role — `/oauth/authorize`,
`/oauth/token`, `/oauth/jwks`, SAML login/acs — and `kb_oauth_flow` is the AS's own bookkeeping
(`pending_saml` → `code_issued` → `consumed`), tracking flows *it* authorizes. T2 needs the
opposite: build an authorize URL, hold a PKCE verifier across a redirect, receive a callback,
exchange a code. That is client-side state, for which `flow.ts` has no slot.

This holds in **both** deployments. The AS surface is live and non-legacy — `metadata.ts:93-95`
serves "BOTH instance types (SAML/AS instances that set `AS_ISSUER`, and the legacy Auth0-fronted
instance that doesn't)" — enterprise installs run AS/SAML mode; temperkb.io is the Auth0-fronted
one. On an enterprise install T2 is a client of the **local** AS; on temperkb.io, a client of Auth0.
Either way, a client.

The decisive reason to build in Rust: **the access token returned by the exchange is a temper JWT** —
same issuer, same audience temper-api already validates. So the callback resolves the profile
through the sanctioned front door rather than reaching around it. `resolve_from_claims` is
`pub(crate)` *as a security property* (`temper-services/src/auth/mod.rs:160-164`: "a surface cannot
hand this function claims it built itself, which is what makes a forged `AuthClaims` inert"). A
TypeScript callback would have to cross back into Rust to write the auth-link row, re-opening
exactly that seam. Rust keeps it shut.

**Rejected:** minting in-process in AS mode to avoid the HTTP self-hop. It would bypass the AS's own
code/PKCE validation and fork the flow per mode — two paths, one of which only ever runs on customer
installs, where we would notice breakage last. temper-api exchanges against the instance's own
`/oauth/token` and the AS validates PKCE via `verifyPkceS256` exactly as it does for the CLI. One
path, both modes.

### D3 — the profile is resolved LOOKUP-ONLY; absent ⇒ refuse

**Connecting Slack is not a registration route.** `authenticate_token`
(`temper-services/src/auth/mod.rs:95`) is a *login* path: `resolve_human_from_claims`
(`profile_service.rs:117`) step 5 (`:151-155`) creates a brand-new profile and provisions its
entities and default context. Its own doc says it: "the machine path has a gate; the human path has
auto-provisioning."

The cost is not a stray row — **it is unapproved reach, conferred by a database trigger.**
`trg_sync_system_membership` is `AFTER INSERT OR UPDATE OF system_access ON kb_profiles`
(`migrations/20260624000002_canonical_functions.sql:79-81`) and calls
`ensure_auto_join_memberships` (`migrations/20260629000002_auto_join_team_generalization.sql:41,91-95`).
That migration's stated invariant: enrollment gates on `has_system_access`, and "In `open` mode
(**default**) that is true for everyone → **every profile auto-joins every auto-join team**"
(`:13-16`). Production runs `access_mode = 'open'`, so that gate is vacuous there.

So the chain would have been: stray Slack click → `authenticate_token` → INSERT `kb_profiles` →
trigger → membership in every auto-join team at its `auto_join_role`. **No line of the callback
would "do" it, and there is no way to create the profile without it** — the enrollment is a trigger,
not a decision.

The governing rule is therefore: **reach must be backed by an approved auth flow.** It resolves
coherently across both modes without special-casing the policy:

- **AS/SAML mode** — the profile is JIT-created upstream at SAML ACS (`resolve_federated_human`,
  `auth/mod.rs:139`, which "resolves-or-JITs the profile the minted token will later resolve to"),
  and auto-join fires. **SAML backs it** — an employee completed enterprise SSO. Legitimate; we
  leave it alone. Note this happens two hops *before* our callback, so it is not ours to prevent.
- **Auth0 mode** — lookup-only, so no profile is ever born from a Slack click. Anyone wanting one
  goes through the front door, where Auth0 backs it.

Cold start costs one extra round trip: sign in at temperkb.io once, then reconnect. Accepted.

Implementation: a lookup-only human resolution in temper-services — a **narrowing** of the existing
path, mirroring the machine arm, which is already lookup-or-reject by design
(`resolve_machine_from_claims`, `profile_service.rs:169`). Not a new write path.

### D4 — rebind is a feature, not a threat

**To bind a Slack principal to profile P you must authenticate as P.** The token's `sub` is the only
thing naming the temper side. So a principal can only ever be bound **to the authenticator's own
profile**; there is no move that binds it to someone else's. Profile takeover does not exist here.

The direction of harm inverts accordingly: if B links A's Slack principal to B's profile, B has not
taken anything from A — B has handed *A* the keys to *B's* profile. Whoever completes the login bears
the risk. That is self-harm, not attack, and not the design's job to prevent.

Re-link is therefore just: someone authed as B and claimed principal U. If they control U it is
legitimate — the account-move case, which should work. Many links → one profile is the intent:
a person's `auth0`/SAML link and their `slack` link converge on one profile, and
`UNIQUE(auth_provider, auth_provider_user_id)` is what makes it safe (each link owned by at most
one profile). The upsert is
`ON CONFLICT (auth_provider, auth_provider_user_id) DO UPDATE SET profile_id`, satisfying the
acceptance criterion's idempotency for the same-profile case and permitting the move for the rest.

**No confirm step, and no audit event as a control.** An earlier draft proposed one; it answered a
threat the auth gate already closes.

### D5 — the HMAC gates an agent→API call, never the user-clicked URL

What remains after D4 is not profile takeover but **Slack-side experience hijack**: bind a victim's
principal to your own profile and the victim's future mentions silently resolve to *you*, so the
victim — believing they are saving to their own KB — writes into yours. The exfiltration is
victim-authored content.

Slack user ids are **visible in the workspace**. So an open start endpoint would make that attack
need nothing but reading a user id off a profile card. The HMAC gate is what reduces it to "steal a
message only the victim can see": the URL is minted only in response to a real mention and delivered
ephemerally to that user. That is the difference between public information and an already-
compromised account, and it is the honest justification for the gate.

**The signature cannot ride in the URL the user clicks.** `internal_sig::MAX_SKEW_SECS` is 30
(`temper-core/src/internal_sig.rs:35`), and a human clicks a Slack link minutes later. Widening the
skew to fit human latency would loosen a gate that is tight for good reason. So the HMAC covers the
**agent→API** call (`POST /internal/slack/link-state`), which is immediate and well inside 30s;
what the user receives is the IdP's own authorize URL carrying an opaque `state`, with nothing
forgeable in it.

Reuses `internal_sig::{sign, verify, timestamp_is_fresh}` (`:48,54,66`) unchanged — HMAC-SHA256 over
`"{timestamp}.{body}"`, constant-time, already `pub` and generic over bytes.

### D6 — `state` is opaque + DB-backed, not signed (deviation from the acceptance wording)

T2's acceptance says `state` must be "signed, single-use, TTL-bounded". **Signed-and-stateless
cannot be single-use** — burning it requires a store regardless. An opaque random nonce in a row
delivers single-use, TTL **and** unguessability from one mechanism, and satisfies the actual
requirement ("a tampered/expired `state` is rejected") strictly better than a signature would.
Recorded as a deliberate deviation.

Single-use is the atomic consume:

```sql
UPDATE kb_slack_link_intents
   SET consumed_at = now()
 WHERE state_nonce = $1 AND consumed_at IS NULL AND expires_at > now()
RETURNING code_verifier, slack_principal_id;
```

Zero rows ⇒ unknown, expired, or replayed — indistinguishably, and safely. This is the replay-proof
pattern of `packages/temper-cloud/src/oauth/flow.ts:56-77` (`bindCodeToFlow`), reused as a **pattern,
not as code**.

**Rejected:** extending `kb_oauth_flow`. It is the AS's own bookkeeping with a
`status CHECK IN ('pending_saml','code_issued','consumed')`
(`migrations/20260701000006_saml_as_tables.sql:45-58`); widening a shipped CHECK to carry
client-side state would tangle the two halves of OAuth that D2 separates.

### D7 — the confirmation is a browser page; temper-api holds no Slack credential

The callback renders HTML to the browser the user is already looking at. temper-api needs no
`SLACK_BOT_TOKEN`, no channel/thread knowledge, and `state` carries the principal only. The next
`@mention` working is its own confirmation.

**Rejected:** the callback calling `chat.postEphemeral` — it would put a workspace credential in the
API server, which the "one deployment = one workspace" finding
(research `019f6be2-1e14-7160-9caa-861859251a23`) says does not generalize. **Rejected:** a callback
→ agent notify hop — a new authenticated inbound surface on the agent for a confirmation the user
is already looking at.

### D8 — a `temper-auth` crate for the shared mechanics

Naively reusing `login.rs`'s helpers would make temper-api depend on temper-**client**, inverting
server→client. Instead, a new **`temper-auth`** crate holds the pure, shareable mechanics, and both
temper-services and temper-client depend on it:

- `generate_pkce_pair()` (from `login.rs:41`)
- `build_authorize_url(params)` (from `login.rs:58`) — takes a **params struct**: 7 inputs
  (authorize_url, client_id, audience, redirect_uri, scopes, state, challenge) is past the repo's
  5-parameter rule. This also retires the baked-in `port: u16` (`login.rs:60`, written into `state`
  at `:80`) — the single biggest reuse blocker.
- the `TokenResponse` wire type (from `login.rs:95`, currently private)

**Scoped to what T2 needs.** Moving `internal_sig` or `auth_config` out of temper-core/temper-services
into `temper-auth` is *not* in T2: T2 consumes `internal_sig` fine where it already lives, and the
move would churn every existing consumer (`middleware/internal_auth.rs`) for no gain to this task.
If `temper-auth` proves out, those are candidates for a later consolidating pass — deliberately
deferred, not overlooked.

Pure crypto and strings — **no HTTP**, so no reqwest in the shared crate and no CLI bloat.
temper-client drops its local copies (CLI behavior unchanged); temper-services adds a small
`oauth_client` doing the form POST and deserializing `TokenResponse`. That shares the crypto and the
wire type per "shared types at boundaries", duplicating only a form POST rather than inverting the
dependency direction to avoid ~25 lines.

**What must NOT move into `temper-auth`:** the claims→profile seam. `authenticate` /
`resolve_from_claims` are `pub(crate)` *as the security property*; lifting them into a shared crate
turns `pub(crate)` into `pub` across a crate boundary and the guarantee evaporates silently.
`temper-auth` = mechanics; temper-services keeps the seam and its crate-privacy.

### D9 — the endpoint answers "what do I say?", not "mint me a URL"

The first cut of this design had the agent ask for an authorize URL on **every** mention,
unconditionally, and nothing ever asked whether the user was already linked. Two consequences, both
real:

1. **The re-prompt regression.** A user who successfully completed the link got told to link again
   on their very next mention — **forever**. The "linked" branch did not exist and was quietly
   deferred to a later task, so the success path had no reply of its own. The one thing the flow
   exists to achieve was also the thing it could never acknowledge.
2. **A junk intent row per mention.** Every mention from a linked user minted a PKCE pair and a
   `kb_slack_link_intents` row that nobody would ever click. Unbounded, one per mention per user,
   for no purpose.

The mistake was in the question. The agent's real question per mention is **"what do I say to this
person?"** — "mint me a URL" is one possible *answer* to it, and hard-coding the answer into the
request is what made the other answer unrepresentable. So the endpoint answers the question:

```rust
#[serde(tag = "status", rename_all = "snake_case")]
enum SlackLinkStateResponse {
    Linked { handle: String },        // mints nothing
    Unlinked { authorize_url: String },
}
```

A **discriminated union**, not a struct of `Option`s. The two arms carry disjoint data, and a struct
with two nullable fields would make "both set" and "neither set" representable — two states that must
not exist, on a surface where "neither set" reads as a silent failure. The agent mirrors the union in
`agent/lib/link.ts`, so both ends are forced to handle both arms. The lookup runs **before** the mint
and short-circuits it; that ordering is the entire fix.

The lookup is deliberately **not** filtered on `kb_profiles.is_active`. A deactivated profile is not
an unlinked one — the link genuinely exists. Reporting "unlinked" would send the user into a link
flow whose callback then refuses them (`authenticate_token_existing_only` rejects a deactivated
profile), which is a loop with no exit and no explanation. Answering "linked" tells the truth about
the directory and lets the deactivation surface where it is actionable.

**Both arms still `postEphemeral` and still drop** (no model turn). The linked arm has nothing to
dispatch *to* yet — reads under proven identity are T4 — so it says exactly that, and says it
ephemerally: the unlinked message is a credential, and the linked one is per-mention status noise no
public channel asked for.

**Consequence worth naming: re-link is no longer reachable by mentioning again.** A linked user is
never issued a fresh challenge, so D4's rebind cannot be driven from Slack. The upsert stays
idempotent and the rebind stays correct where it is still reachable (two mentions before either link
is clicked — the e2e test models exactly this, and it is the only route to a second callback). A
deliberate "connect a different account" affordance is a **separate feature**; it was previously
reachable only as a side effect of the re-prompt bug, which is not a design.

## Components

### New: `kb_slack_link_intents`

Additive migration. Client-side flow state, distinct from the AS's `kb_oauth_flow`.

| column | notes |
|---|---|
| `id` | UUIDv7 |
| `state_nonce` | **UNIQUE** — the opaque `state` handed to the IdP |
| `code_verifier` | PKCE verifier, held across the redirect |
| `slack_principal_id` | the whole opaque principal — **never** split on `:` |
| `expires_at` | TTL bound |
| `consumed_at` | NULL until burned; the single-use marker |
| `created_at` | |

> **Never `split(":")` the principal.** It has 2–4 segments
> (`slack:<team>:<user>`, `slack:<user>`, `slack:<team>:bot:<user>`, `slack:bot:<user>`). An index
> parse silently mis-keys a user by reading `<user>` from the `<team>` slot. Store and compare it
> whole — which is exactly what `auth_provider_user_id VARCHAR(128)` wants.

### New: `POST /internal/slack/link-state` (temper-api)

HMAC-gated (D5). Body carries the principal. Looks the principal up in `kb_profile_auth_links`
first: a hit returns `{ status: "linked", handle }` and **mints nothing**; a miss generates the PKCE
pair, inserts the intent, and returns `{ status: "unlinked", authorize_url }` built mode-aware from
`AuthConfig`. See D9 for why the endpoint answers this question rather than minting on demand.

**Why `/internal/*` and not `/api/*`:** the namespace is a routing fact, not a naming preference.
`vercel.json` routes `/internal/(.*)` to the internal function and leaves `/api/*` to the public
axum function, so an `/api/…/link-state` path would land on the wrong function entirely. It also
reads true: this is the same server-to-server, HMAC-gated, non-JWT surface as
`/internal/saml/reconcile`, and it shares that gate's implementation. The callback is the opposite
kind of thing — browser-facing — and correctly stays at `/api/auth/slack/callback`, where the
public function serves it.

### New: `GET /api/auth/slack/callback` (temper-api)

The registered `redirect_uri`. Consumes the intent (D6), exchanges the code, resolves the profile
lookup-only (D3), upserts the link (D4), hands the RT to T3's seam, renders the success page (D7).

### New: link service functions (temper-services)

The upsert, plus `lookup_linked_handle` — the `kb_profile_auth_links ⋈ kb_profiles` read backing
D9's linked arm, returning the profile's slug (the `kb_profiles.handle` column; the Rust `Profile`
maps it to `slug`). SQL lives in the service layer; never inline in a handler.

### Changed: `packages/agent-workflows/mention/agent/channels/slack.ts`

`:36` currently `ctx.thread.post(unlinkedPrompt(...))` — a **public** thread post, harmless today
because the prompt carries no link. **The moment it carries an authorize URL it is a credential in a
public channel.** It becomes `ctx.thread.postEphemeral(user_id, …)`, verified available on
`ctx.thread` in `onAppMention`. The user id comes from **`attributes.user_id`**, never from parsing
`principalId`. The agent calls `link-state` first and branches on `status`: `linked` → `linkedPrompt(handle)`,
`unlinked` → `unlinkedPrompt(authorize_url)` (D9). `:51`'s deliberate drop stays, and now covers
**both** arms: post, then drop; no model turn while there is nothing a turn can honestly do —
unlinked there is no identity, linked there is nothing yet to dispatch to.

### Mode-aware provider config

`AuthConfig { issuer, jwks_url, audience, mode: ExternalIdp | TemperAs }`
(`temper-services/src/auth_config.rs`) already gives the server its mode. `authorize_url`/`token_url`
derive from it; the link client_id is new env. `init.rs:548-566` maps `Idp::TemperAs` to
`{base}/oauth/authorize`, where base is the instance URL rather than a separate auth host.

**The AS-mode derivation is verified by trial on a real AS instance after merge, not by inference
now** (decided 2026-07-16). It is the one genuine unknown in this design: the Auth0-mode path is
exercised by temperkb.io, but no local test reproduces a real AS instance's env, so a green suite
would not evidence it either way. Naming it as an accepted post-merge trial is honest; asserting it
from `init.rs` would not be. If it fails, the failure is legible — a wrong `authorize_url`/`token_url`
base — and the fix is contained to this derivation. Nothing else in the flow depends on it.

## Decomposition

Two PRs, in order. The split is not arbitrary — the first is a pure refactor with no behavior change,
and bundling it into the second would bury a cross-crate move inside a feature diff.

1. **`temper-auth` extraction** — new crate; lift `generate_pkce_pair`, `build_authorize_url`
   (now params-struct, no `port`), `TokenResponse`; repoint temper-client. **One atomic commit** —
   a cross-crate type move that does not compile in halves. No behavior change; the CLI's existing
   login tests are the regression net.
2. **The link flow** — migration, both endpoints, the service function, the agent change, e2e.

## Error handling

Every failure renders **HTML, never JSON** — a human is looking at it. The intent consume is the
atomic UPDATE, so replay, expiry and forgery collapse to one indistinguishable rejection. A failed
exchange, an absent profile and a bad state render distinct *user-actionable* text but leak nothing
about which profiles exist: the lookup-only refusal reads "No temper account is linked to this
login — sign in at temperkb.io first, then reconnect" whether the account is absent or something
else went wrong.

Auth-before-write holds naturally: the profile resolves before the upsert.

## Testing

Pure units test in isolation: PKCE, authorize-URL building, intent consume.

The flow needs the **e2e tier**. `test-db` green is a false signal for access-semantics changes, and
this is squarely one. Note `cargo make test-e2e` compiles out `test-embed`-gated tests; use
`cargo make test-e2e-embed` where relevant.

The three load-bearing tests:

1. **Lookup-only refuses an unknown `sub` — and no `kb_profiles` row appears.** This is the D3
   invariant, and asserting the refusal alone would not catch a regression that creates the profile
   and then errors. Assert the absence of the row.
2. **A second callback with the same `state` is rejected.** The single-use invariant (D6).
3. **A linked principal gets `status: "linked"` — and the intent count does not move.** The D9
   invariant. The count is the assertion, not the status: a regression that answers correctly and
   *still* mints the junk row on the way would pass a status-only check, and the waste would be
   invisible and unbounded. Count before, count after, unchanged.

## Ops / deployment

The `redirect_uri` must be registered — **in both modes, and it is not optional**:

- **Auth0 mode** — Allowed Callback URLs on the link client.
- **AS mode** — `AS_CLIENTS` (JSON `{clientId: string[]}`), the source of truth for allowlisted
  redirect URIs per client (`packages/temper-cloud/src/oauth/clients.ts:5-24,18`; rejected at
  `endpoints.ts:107`).

`clients.ts:8` names the exact attack if this is skipped: an attacker crafts an authorize link with
their own PKCE pair and a `redirect_uri` they control, tricks a victim into completing the login,
and receives the code.

New env, three vars, all on temper-api: `SLACK_LINK_CLIENT_ID` (the link client_id),
`SLACK_LINK_SECRET` (the HMAC secret shared with the agent — fail-closed if unset, per
`internal_auth.rs:7-8`'s precedent), and `PUBLIC_BASE_URL` (this instance's public origin, which
the callback `redirect_uri` is derived from). They are parsed as a unit: all three present ⇒ the
flow is enabled, any absent ⇒ disabled. The derived `redirect_uri`
(`<PUBLIC_BASE_URL>/api/auth/slack/callback`) must be registered with the IdP — Auth0's Allowed
Callback URLs, or the client's `AS_CLIENTS` entry on an AS instance.

## Out of scope

### Rejected

- **Email-based auto-mapping.** Slack supplies **no email** on the wire
  (`packages/agent-workflows/mention/CLAUDE.md:53-56`), so `reconcile_by_email`
  (`profile_service.rs:263`) is not merely undesirable here — it is **inexpressible**. The link is
  the trust root.
- **A confirm step on rebind** (D4), and **an audit event as a security control** (D4).
- **Management-API mint** (D1), **in-process AS minting** (D2), **extending `kb_oauth_flow`** (D6),
  **Slack posts from temper-api** (D7).

### Deferred

- **The vault + refresh** → T3. T2 hands the RT to a seam.
- **Presenting the token to temper-mcp** → T4. **Writes + HITL** → T5.
- **`login.rs` never validates the returned `state`** — the CLI has no CSRF check on its callback.
  Pre-existing, low severity behind a loopback, real. Not T2's narrative; file separately rather
  than bundle.
- **The community multi-workspace credential architecture** — one deployment serves one workspace
  (research `019f6be2-1e14-7160-9caa-861859251a23`; decision task
  `019f6be2-7630-7d83-9b5c-30df4cca93cb`). T2 is unaffected: self-hosted is the natural shape.

## Key file index

| What | Where |
|---|---|
| PKCE + authorize URL + `TokenResponse` | `crates/temper-client/src/login.rs:41,58,60,80,95,107` |
| HMAC sign/verify/freshness | `crates/temper-core/src/internal_sig.rs:35,48,54,66` |
| The seam (`pub(crate)`, deliberate) | `crates/temper-services/src/auth/mod.rs:95,139,160-164,165` |
| Human resolve + JIT create | `crates/temper-services/src/services/profile_service.rs:99,117,151-155,169,205,328,451` |
| Link table DDL | `migrations/20260624000001_canonical_schema.sql:331-342`; `20260709000004_auth_link_email_verified.sql:14-15` |
| Auto-join trigger + fn | `migrations/20260624000002_canonical_functions.sql:79-81`; `20260629000002_auto_join_team_generalization.sql:13-16,41,91-95` |
| AS surface (server half) | `packages/temper-cloud/src/oauth/{endpoints,flow,clients,pkce,metadata}.ts` |
| AS mandates PKCE S256 + state | `packages/temper-cloud/src/oauth/endpoints.ts:96,99,101,117,199` |
| Both instance types | `packages/temper-cloud/src/oauth/metadata.ts:93-95` |
| Mention agent delivery point | `packages/agent-workflows/mention/agent/channels/slack.ts:28,32,36,51` |
| Unit D blueprint (not used — D1) | `docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md:84-88,159-164,190-198` |
