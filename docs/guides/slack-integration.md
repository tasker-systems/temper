# The Slack Integration — identity, credentials, and what revocation actually means

Temper's Slack integration lets a human mention `@temper` in Slack and get an answer computed
**under their own temper reach** — their contexts, their resources, nobody else's. This document is
the reference for **operating and reviewing** that: the identity model, the four credentials in
play and what each is worth, where the trust boundaries sit, what fails closed, how the secrets
must be sequenced across deploys, and — the section most worth your time — **what "disconnect"
does and does not stop.**

It is deliberately *not* a setup runbook. To stand the thing up, follow
[slack-setup.md](slack-setup.md) step by step; come back here when you need to reason about it.
The split mirrors [machine-credentials.md](machine-credentials.md) (the credential model) versus
the install guides.

> **Every claim here is cited to code.** Where a behaviour is defended by a test or a CI guard, the
> guard is named. Where a mitigation lives outside this repo, it says so. Where something is
> best-effort, it says best-effort.

---

## If you are here because something broke

| Symptom | Most likely cause | Where to look |
|---|---|---|
| Every mention `401`s at temper | `SLACK_LINK_SECRET` differs between `temper-mention` and `temper-cloud`, or is unset on either | `crates/temper-api/src/middleware/internal_auth.rs:123-141` (fail-closed on unset) |
| Mentions resolve, but the answer never comes and the log shows a mint `401` | `SLACK_MINT_SECRET` set but **not byte-identical** on both projects — a clipped trailing `=` does this. It is an opaque string, never decoded | `internal_auth.rs:82`; `packages/agent-workflows/mention/agent/lib/link.ts:20` |
| An env var was changed and nothing changed | Vercel does **not** rebuild on an env change — redeploy | [Deployment](#deployment-and-secret-sequencing) |
| Mentions work; the bot says it can't answer | `SLACK_MINT_SECRET` unset ⇒ the mint route is **disabled but the link flow is fine** | `internal_auth.rs:166-179`; `crates/temper-services/src/config.rs:181` |
| The whole link flow says "Account linking is not configured" | One of the **four** link vars missing, *or* `SLACK_VAULT_ENC_KEY` malformed | `config.rs:204-225` — all-or-nothing, and a bad key logs loudly then disables |
| A user is "connected" but every mention says re-link | Linked with no vaulted grant ⇒ mint answers `not_vaulted` | `crates/temper-api/src/handlers/slack_mint.rs` `NotVaulted` arm |
| "Already connected to a different temper account" | The no-rebind guard fired; the principal is bound elsewhere | `crates/temper-services/src/services/slack_link_service.rs:168-226` |
| A disconnected user still seems to have access | **Expected.** See [Revocation](#revocation-what-disconnect-actually-stops) — this is not a bug | `crates/temper-api/src/middleware/auth.rs:49-102` |
| Agent function dies at import | `TEMPER_MCP_URL` unset — read at **module load**, not per-request | `packages/agent-workflows/mention/agent/connections/temper.ts:35` |

Slack-side symptoms (URL verification, bot silence) are in
[slack-setup.md § Troubleshooting](slack-setup.md#troubleshooting).

---

## The identity model

A Slack human becomes a temper principal by a **one-time, browser-completed account link**. Nothing
is inferred, matched, or auto-provisioned.

### The principal is an opaque string

eve mints a `principalId` for every inbound Slack message. It has **four shapes**, because the team
id is nullable and bots carry an extra segment
(`packages/agent-workflows/mention/agent/lib/identity.ts:10-25`):

| team id | author | `principalId` | `principalType` |
|---|---|---|---|
| yes | human | `slack:<team>:<user>` | `user` |
| yes | bot | `slack:<team>:bot:<user>` | `service` |
| no | human | `slack:<user>` | `user` |
| no | bot | `slack:bot:<user>` | `service` |

So the segment count varies from **2 to 4**, and the string is treated as **opaque everywhere** —
stored whole, compared whole, logged whole. temper's own validator is explicit that this is a
refusal to parse, not an oversight (`crates/temper-api/src/handlers/slack_link.rs:126-150`):

> Three checks, all shape and none semantic: non-empty, within the storage column's width, and
> carrying the `slack:` prefix. The principal is OPAQUE — 2 to 4 segments … so it is deliberately
> NEVER split on ':'. A prefix check plus a length check is the whole of what is knowable without
> parsing something we have no business parsing.

`accepts_every_shape_of_real_principal` (`slack_link.rs:527-536`) asserts all four shapes pass — a
test whose failure means the guard has started parsing.

Where a bare Slack user id *is* needed (delivering an ephemeral), it comes off
`attributes.user_id`, never a parse (`agent/channels/slack.ts:83-86`).

### The binding lives in `kb_profile_auth_links`

The link row is `(auth_provider = 'slack', auth_provider_user_id = <the whole principal>) →
profile_id`, with `UNIQUE(auth_provider, auth_provider_user_id)`
(`migrations/20260624000001_canonical_schema.sql:331-340`). The column is `VARCHAR(128)`, which is
why the handler rejects over-long principals at the door rather than at the final upsert
(`slack_link.rs:36-40`) — `rejects_a_principal_wider_than_the_storage_column`
(`slack_link.rs:544`) pins that boundary.

**The directory row holds no secret and must never grow one.** The secret lives in a separate
table, encrypted at rest (`migrations/20260717000030_slack_grant_vault.sql:6-7`). Identity and
secret are deliberately separated.

**The principal binds once.** There is no rebind, and the guard is atomic — a `WHERE` on the
`ON CONFLICT ... DO UPDATE`, so a different-profile attempt matches zero rows and returns a refusal
rather than raising (`slack_link_service.rs:198-226`):

```sql
ON CONFLICT (auth_provider, auth_provider_user_id)
DO UPDATE SET linked_at = now()
WHERE kb_profile_auth_links.profile_id = EXCLUDED.profile_id
```

Same profile ⇒ idempotent re-stamp. Different profile ⇒ `AlreadyLinkedToAnotherProfile`, nothing
written. "Start fresh" is an explicit `temper slack disconnect`, never a side effect of linking
again.

### Why the link is keyed on the principal and not email

**Because there is no email on the Slack wire.** This is verified twice over:

1. eve's Slack channel sets exactly these attributes — `author_type`, `channel_id`, `thread_ts`,
   `user_id`, plus optional `user_name`, `full_name`, `team_id`
   (`packages/agent-workflows/mention/CLAUDE.md`, "THERE IS NO EMAIL", verified against
   eve@0.18.1's `buildSlackAuthContext`). No email field exists to read.
2. Independently, the **entire** body the agent transmits to temper is one field, on both routes:
   `JSON.stringify({ slack_principal_id: principalId })` (`agent/lib/link.ts:53`,
   `agent/lib/mint.ts:56`). A grep for `email` across the agent's `agent/`, `tests/`, and manifest
   returns zero matches.

The link row records this by leaving `email` NULL, and says why
(`slack_link_service.rs:189-190`):

> `email` stays NULL: Slack supplies no email on the wire, which is exactly why the link is keyed
> on the opaque principal.

An email-based auto-link is therefore not merely undesirable — it is **not expressible** with what
arrives. The email the flow *does* use is the one on the **IdP's** token during the browser
callback, which is a different channel entirely and carries the user's own consent.

### Linking is lookup-only

The callback resolves the freshly-exchanged access token through
`authenticate_token_existing_only`, **never** `authenticate_token`
(`slack_link.rs:339-387`):

> the latter auto-provisions a profile, which on a stray click would mint an account and confer
> auto-join team reach. Linking an existing identity is not a registration route.

`callback_with_an_unknown_identity_creates_no_profile`
(`tests/e2e/tests/slack_link_test.rs:455`) defends this.

---

## The four credentials, and what each is worth

This is the conceptual core. Four secrets are in play and they are worth **wildly** different
amounts.

| Credential | Authenticates | Reach it confers in temper |
|---|---|---|
| **Slack bot token** (`SLACK_BOT_TOKEN`) | eve → the Slack Web API | **None.** temper never sees it; temper-api holds no Slack credential and knows no channel (`slack_link.rs:159-163`). |
| **`SLACK_SIGNING_SECRET`** | Slack → eve (inbound webhook HMAC) | **None.** It gates entry to the agent, not to temper. |
| **`SLACK_LINK_SECRET`** HMAC | request integrity, agent → temper on `/internal/slack/link-state` | **None — it is not an identity at all.** It authenticates *the call*, not a person. The endpoint answers one question: "is this principal linked, and what do I say?" |
| **`SLACK_MINT_SECRET`** HMAC | request integrity, agent → temper on `/internal/slack/mint` | **None itself — but it gates the row below.** Possession is the *only* thing that makes a named principal mintable. |
| **A minted per-user access token** | the linked human | **That human's ENTIRE reach.** `resources_visible_to` takes a profile and nothing else — there is no narrowing behind it. Whoever holds the token *is*, to temper, that person. |

### Why the link secret and the mint secret are deliberately different

Because one **answers a question** and the other **confers reach**. Sharing a key would mean that
compromising the ability to ask *"is Alice linked?"* also yields *"give me a token that is Alice."*

The reasoning is stated in three places, all consistent
(`crates/temper-api/src/middleware/internal_auth.rs:143-165`):

> **This is the highest-privilege gate in the file, and the reason it has its own key.** The other
> two guard endpoints that *report* something … This one guards an endpoint that hands back an
> **act-as-the-human access token** … The endpoint that **confers reach** cannot share a key with
> one that merely answers a question, however convenient one variable would be. Same scheme, third
> key.

`slack_mint_secret` is therefore **deliberately not a field on `SlackLinkConfig`**, for two
independent reasons (`crates/temper-services/src/config.rs:45-61`): the privilege asymmetry above,
**and** because `parse_slack_link` is all-or-nothing, so folding it in would make a deploy that has
not yet set the mint secret silently disable the *entire* link flow.

The mint gate is also load-bearing in a second, subtler way. `mint_access_token` **enforces no
authorization** and mints for whatever principal it is handed
(`slack_grant_vault_service.rs:233-236`). The rule *"naming a principal must not be sufficient to
mint its token"* is a claim about **provenance**, which is extrinsic — handed
`"slack:T123:U456"`, no function can tell whether it was read off a Slack-signed webhook or typed
by an attacker. So it cannot be a predicate in the service, and a service-side check would be
theatre (`slack_mint_service.rs:8-41`). **The transport gate is the whole enforcement.**

> A consequence worth internalising: **a test that calls `mint_for_mention` directly and passes has
> proved nothing about authorization.** The test must drive the route.

**Three keys, three gates, pairwise distinct** — and this is enforced structurally in CI, not by
convention. `.github/scripts/audit-signature-secrets.sh` asserts each gate reads a distinct secret
field, and the distinctness check is **computed, not baselined**, so `UPDATE_BASELINE` cannot
silence it. Its header names the exact failure it exists to catch:

> Collapsing two of these onto one config field is a one-line edit that no type checks, no route
> audit notices (all three layers are still mounted, so `audit-route-auth.sh` stays green), and —
> until this script — nothing in CI caught. It was defended only by an e2e test, i.e. only where
> someone remembered to look.

The agent side is defended too: `tests/mint.test.ts:42-56` asserts the mint call's signature equals
the HMAC under the *mint* key and **not** the link key, with `SLACK_LINK_SECRET` stubbed
present-and-wrong so the assertion is non-vacuous. On the server side,
`mint_refuses_the_link_state_key` (`tests/e2e/tests/slack_link_test.rs:1506`) drives the real route.

---

## Trust boundaries

Untrusted input enters at exactly one place — Slack — and is re-authenticated at every hop.

```
[Slack workspace]                          ← untrusted; anyone in the workspace can type anything
      │  POST /eve/v1/slack
      │  ── verified by: Slack request signature (SLACK_SIGNING_SECRET), HMAC over the raw body
      ▼
[eve runtime, in the mention agent]        ← boundary 1
      │  verifyInbound() is the FIRST statement of the route; failure ⇒ 401 before any dispatch
      │  principalId is derived HERE, by eve, from the verified event
      ▼
[agent handler: onAppMention]              ← boundary 2 (policy, not authentication)
      │  decideIdentity(): principalType === "user" or drop; bots surface as "service"
      │  body = { slack_principal_id } and NOTHING else
      │  ── signed with: HMAC-SHA256(secret, "{timestamp}.{body}") → X-Temper-Signature
      ▼
[temper-api /internal/slack/{link-state,mint}]   ← boundary 3
      │  require_slack_link_signature | require_slack_mint_signature
      │  fresh timestamp (±30s) + constant-time MAC over the exact bytes received
      ▼
[services → DB]                            ← boundary 4
         resources_visible_to / can_modify_resource scope every query to the profile
```

**Boundary 1 — Slack → eve.** eve verifies the Slack request signature as the route's first
statement and fails closed: a missing `SLACK_SIGNING_SECRET` *throws*, is caught, and the route
answers `401`. The `url_verification` branch lives inside `handleEventPost`, reached only after
verification passes (`packages/agent-workflows/mention/slack-app-manifest.yml:10-14`). This is
framework code; **no test in this repo covers it.**

**Boundary 2 — the principal's provenance.** The agent never reads message text or any
user-supplied field to build the principal. It comes from eve's `defaultSlackAuth(message, ctx)`
and is passed through verbatim (`agent/lib/identity.ts:88-92`). The human gate is written
positively — `principalType === "user"`, not `!== "service"` — so a principal type eve adds later is
**refused by default rather than admitted by accident** (`identity.ts:71-73`).

**Boundary 3 — agent → temper.** `HMAC-SHA256(secret, "{timestamp}.{body}")`, lowercase hex, in
`X-Temper-Signature` with `X-Temper-Timestamp` (`crates/temper-core/src/internal_sig.rs:26-36`).
The MAC covers the **raw body bytes as transmitted**, so there is no cross-language
canonicalisation to drift on; a shared known-answer vector pins the Rust and TypeScript
implementations together. Verification is constant-time (`verify_slice`, `internal_sig.rs:53-63`)
and skew is ±30s in either direction (`MAX_SKEW_SECS`, `internal_sig.rs:36`).

**Boundary 4 — the DB.** A minted token then re-enters through the ordinary front door:
`require_auth` does full JWKS validation and every query scopes through the standard visibility
predicates. The Slack path gets no shortcut.

One thing this diagram is careful about: **the browser callback is not on this path.** It is a
separate, deliberately public route (`/api/auth/slack/callback`) whose compensating control is
PKCE + a single-use unguessable state nonce, burned atomically
(`slack_link.rs:196-204`). The link URL handed to the user is the IdP's own authorize URL, *not* a
signed temper URL — signing it would force loosening the 30s skew to human-click timescales
(`slack_link.rs:77-80`).

Every route's auth posture is pinned in CI by `.github/scripts/audit-route-auth.sh`, which freezes
the set of routes in review-required groups **and** asserts the layer wiring is still present, so a
silently deleted auth layer fails immediately.

---

## Fail-closed behaviours

Every configuration gap in this feature disables something. None opens anything.

| # | Behaviour | Where |
|---|---|---|
| 1 | **Unset `SLACK_LINK_SECRET` disables the link-state endpoint.** No secret ⇒ every request rejected. | `internal_auth.rs:123-141` |
| 2 | **Unset `SLACK_MINT_SECRET` disables the mint endpoint** — and *only* that. An instance can legitimately run with linking on and minting off. | `internal_auth.rs:166-179`; `config.rs:181` |
| 3 | **`parse_slack_link` is all-or-nothing.** `Some` only when all four values are present, non-empty, **and** the vault key parses. A partial set is unconfigured, not half-configured. | `config.rs:200-225` |
| 4 | **A malformed `SLACK_VAULT_ENC_KEY` disables the whole link flow with a loud error**, rather than booting a flow whose vault writes would fail at the callback. | `config.rs:207-217` |
| 5 | **No config ⇒ minting is impossible, not merely unconfigured.** Without the vault key there is no key to unseal a grant with. | `slack_mint_service.rs:64-67` |
| 6 | **The callback is one transaction.** Identity row and sealed grant commit together or not at all — a half-write was unrecoverable, because the state nonce is already burned. | `slack_link.rs:225-238`, `:329-332` |
| 7 | **No refresh token ⇒ the link is rolled back, and the page does NOT render success.** It used to warn and render "Account connected" at a user whose link was inert. | `slack_link.rs:279-311` |
| 8 | **`getTemperToken` fails closed on every non-token outcome**, including an unrecognised fourth status — the `never` binding makes a new variant a compile error, and the runtime arm covers a server that ships one before the agent redeploys. | `agent/lib/mcp-auth.ts:107-157` |
| 9 | **`requireEnv` treats `""` as missing**, so an empty-string secret throws rather than signing with an empty key. | `agent/lib/link.ts:78-82` |
| 10 | **eve's inbound verification fails closed on a *missing* secret**, not just a bad signature. | `slack-app-manifest.yml:10-14` |
| 11 | **DMs are explicitly refused** (`onDirectMessage: async () => null`). Leaving the key absent would inherit eve's default, which dispatches unconditionally — no identity gate, no link-state, no mint pre-flight. | `agent/channels/slack.ts`; `mention/CLAUDE.md` |
| 12 | **The tool allow-list is read-only and is the enforcement point.** Nine names. Writes are absent deliberately: a read-only context member can currently create a resource in that context, so a write tool would exercise that bug under a real human's whole reach. | `agent/lib/mcp-auth.ts:46-74` |

On #8, the specific failure it exists to prevent is worth quoting
(`mcp-auth.ts:142-146`): falling out of the switch would return `undefined` where eve expects a
`TokenResult`, and "the connection would then call the MCP server with no credential, which is the
one thing this function exists to prevent."

`mint_is_disabled_without_its_secret_but_linking_still_works`
(`tests/e2e/tests/slack_link_test.rs:1748`) defends #2 end to end.

---

## Deployment and secret sequencing

`TEMPER_MCP_URL` is new with the agent half and is **required**: it is read at **module load** by
`agent/connections/temper.ts:35`, so an unset value fails the whole function at import rather than
per-mention. (This is why `getTemperToken` and the allow-list live in `agent/lib/mcp-auth.ts` —
importing the connection file in a test process would otherwise throw.) The other five agent vars
are read at request time and throw on the first mention.

### The trap: two Slack secrets with opposite rules

> ⚠️ **This is the pattern-match hazard in this feature.** Two `SLACK_*` secrets have **opposite**
> deployment rules, and they look alike enough that treating them the same way is the natural
> mistake. Read this before touching either.

| Secret | Rule | Why |
|---|---|---|
| `SLACK_VAULT_ENC_KEY` | **MUST be set as part of the deploy that ships the vault** — not after. | `parse_slack_link` is all-or-nothing (`config.rs:204-225`). Deploying vault code to an instance already running the link flow, without the key, **turns the link flow off**. |
| `SLACK_MINT_SECRET` | **MUST NOT be set until its caller ships.** | Setting it early makes a live act-as-any-linked-human endpoint reachable with **no legitimate consumer**. That is exposure with zero upside. The endpoint ships dark by design (`config.rs:57-60`, `internal_auth.rs:163-165`). |

One says *set it early or you break things*; the other says *set it late or you expose things*. The
underlying principle is the same and is worth stating once: **turn on a fail-closed dependency
before the code that needs it; turn on a capability only when something legitimate is asking for
it.** They differ because one key is a *prerequisite* and the other is a *capability*.

### Which deployments this actually lives on

> **The server half is `temper-cloud`, not `temper-api`.** Those name a crate *and* two different
> Vercel projects, and they are not the same target:
>
> | Vercel project | What it is | Slack? |
> |---|---|---|
> | **`temper-cloud`** | the temperkb.io community deployment | **Yes** — every Slack variable below goes here |
> | **`temper-mention`** | the `@temper` mention agent (`packages/agent-workflows/mention`) | **Yes** — the agent-side variables |
> | `temper-api` | the enterprise deployment | **No** — no Slack piece is deployed here yet |
>
> "Deploy temper-api" is the right sentence about the *crate* and the wrong one about the
> *environment*. Set Slack variables on **temper-cloud**.
>
> Vercel does **not** rebuild on an environment-variable change. After setting any of these,
> trigger a redeploy or the running function keeps the old values — and a build-time variable like
> `TEMPER_MCP_URL` will keep failing until you do.

Ordering that follows from this:

1. Deploy **temper-cloud** with all four link vars including `SLACK_VAULT_ENC_KEY`. Linking works;
   minting is off and rejecting.
2. Deploy **temper-mention** with `TEMPER_API_URL`, `SLACK_LINK_SECRET`, `TEMPER_MCP_URL`. Mentions
   resolve link state.
3. **Only then** set `SLACK_MINT_SECRET` on **both** — byte-identical, and **different from**
   `SLACK_LINK_SECRET`.

> **On generating `SLACK_MINT_SECRET`:** `openssl rand -base64 32` is right, but note it is an
> **opaque string** — `verify(secret.as_bytes(), …)` (`internal_auth.rs:82`) and
> `createHmac("sha256", secret)` (`link.ts:20`) both consume it raw. It is **never base64-decoded**,
> so the trailing `=` is part of the secret, not encoding to strip.
>
> **Do not carry this reasoning to `SLACK_VAULT_ENC_KEY`**, which *is* decoded
> (`VaultKey::from_base64`, `config.rs:207`) and must be exactly 32 bytes. Two base64-looking Slack
> secrets, only one of them actually base64. A mismatched mint secret is a **401 on every mention,
> silent** — not a warning, not a degraded mode.

Rolling back is the reverse: unset `SLACK_MINT_SECRET` first, which cleanly disables minting while
leaving linking untouched.

Full variable tables live in [slack-setup.md § Step 2](slack-setup.md#step-2--temper-api-environment)
and [enterprise-install.md](enterprise-install.md).

---

## Revocation: what "disconnect" actually stops

**This is the section to read before treating disconnect as an offboarding control.**

`temper slack disconnect` (self-serve) and `temper admin slack disconnect <principal>` (system
admin, gated in the *service* so future surfaces inherit it —
`slack_disconnect_service.rs:262-278`) both run one chokepoint. Here is what each effect costs in
latency, honestly:

| Effect | Latency | Mechanism |
|---|---|---|
| Vault row deleted; cached AT and RT destroyed locally | **0** | `DELETE FROM kb_slack_grant_vault` in the disconnect transaction (`slack_disconnect_service.rs:160-167`) — the row is *deleted*, not flagged |
| Next mint answers `not_vaulted` | **0** | No row ⇒ `MintOutcome::NotVaulted` (`slack_grant_vault_service.rs:260-262`) |
| Link intents for that principal swept | **0** | `slack_disconnect_service.rs:216-222` — load-bearing, see [Residual risks](#known-residual-risks) |
| **An already-issued access token stays valid at temper's API** | **up to its full remaining TTL** | See below |
| eve's per-user token cache | bounded by the same TTL | Cache is eve's, keyed `user:${issuer}:${id}`; the agent memoizes **nothing** (`mcp-auth.ts:79-85`). Bounded by `expiresAt`, passed verbatim from `expires_at_ms` |
| IdP-side grant revoked, **`TemperAs` mode** | **0, atomic** | A row update in the *same* transaction — no network, no failure mode (`slack_disconnect_service.rs:121-140`) |
| IdP-side grant revoked, **`ExternalIdp` mode** | **best-effort** | On failure it logs a structured warning and **commits the local destruction anyway** (`slack_disconnect_service.rs:141-156`) |
| IdP-side grant revoked, ciphertext unopenable after a key rotation | **never attempted** | An unopenable ciphertext has no value to revoke; returns `Ok(None)` and destroys anyway (`slack_grant_vault_service.rs:348-402`) |
| Profile deactivation (`is_active = false`) | **0, enforced per request** | The mint path checks it before decrypting anything (`slack_grant_vault_service.rs:263-267`), and `require_auth` refuses the token (`middleware/auth.rs:90-93`) |

### Why an issued token survives

`require_auth` is **stateless JWKS validation**: extract bearer, fetch the cached decoding key,
`decode()` with algorithm-scoped validation, then resolve the profile through the shared auth seam
(`crates/temper-api/src/middleware/auth.rs:49-102`). There is **no revocation list, no `jti`
check, and no consultation of the Slack grant vault anywhere in that path.** The vault governs
*minting*; it has no say over a token already minted.

The schema says so at the point of definition
(`migrations/20260717000030_slack_grant_vault.sql:48-51`):

```sql
-- Set by `revoke`. A revoked row mints nothing further. HONEST SEMANTICS: this stops FUTURE
-- ... validation consults no revocation list. Revocation is not instant cutoff.
revoked_at         TIMESTAMPTZ,
```

> ### **"Disconnected" means "cannot mint again". It does NOT mean "cannot act".**
>
> **Disconnect is not an offboarding control.** A token issued moments before the disconnect keeps
> working until its own `exp`. To actually cut someone off, **deactivate the profile** — that is
> enforced per request, at latency 0, on every surface. The CLI says as much on stderr
> (`crates/temper-cli/src/commands/admin_slack.rs`): *"disconnect unbinds an identity, it does not
> deactivate an account."*

### How long is "up to its full TTL"?

Be precise here, because two different clocks get conflated:

- **The token's real validity** is the JWT's own `exp`, set by the IdP when it minted the token.
  temper does not control it. Auth0's default access-token lifetime is 3600s.
- **`access_expires_at` on the vault row is temper's cache bookkeeping**, derived from the
  provider's `expires_in`, defaulting to `DEFAULT_AT_TTL_SECS = 3600` when the IdP omits it and
  **clamped** at `MAX_AT_TTL_SECS = 86_400` (`slack_grant_vault_service.rs:22-50`). The clamp is a
  safety measure against a hostile or broken `expires_in` wrapping the cast negative — it does
  **not** shorten the token.

So the exposure window is *the IdP's access-token TTL*, commonly one hour, and temper cannot
shorten it after the fact.

### `AT_REFRESH_SKEW` is a FLOOR, not a ceiling

A common misreading. The cached-token branch is
(`slack_grant_vault_service.rs:273`):

```rust
if expires_at > Utc::now() + AT_REFRESH_SKEW {
```

with `AT_REFRESH_SKEW = Duration::minutes(5)` (`:20`). This means a cached token is handed back
**only while more than five minutes of life remain**. It is therefore a **lower bound on the
remaining life of any token temper vends** — every minted token is good for *at least* 5 more
minutes — not a cap that limits a token to 5 minutes. Reading it as a ceiling makes the exposure
window look twelve times smaller than it is.

### The mint outcomes, and why the refusal is typed

A refusal is not an error, so it is not an HTTP failure — a `200` carrying a refusal is the honest
encoding of *"the request was fine; there is nothing to mint, and here is exactly why"*
(`handlers/slack_mint.rs`). The response is two arms:

```jsonc
{ "status": "token",   "access_token": "…", "expires_at_ms": 1784505600000 }
{ "status": "refused", "reason": "not_linked" }
{ "status": "refused", "reason": "not_vaulted" }
{ "status": "refused", "reason": "standing", "refusal": { "kind": "denied" } }
```

**The reason exists because the remedies differ, and naming the wrong one is worse than saying
nothing.** `not_vaulted` is fixed by re-linking. A `standing` refusal is fixed by an admin
approving the human — re-linking cannot move principal standing, so telling a denied user to
reconnect sends them round a loop that cannot terminate. That was a real shipped bug: the mint
once answered a flat `revoked` for both, and the agent offered `temper slack disconnect` to
everyone it hit.

`reason` is `LinkRefusal` (`temper-core/src/types/slack.rs`); under `standing` it carries
`temper_principal::Refusal` verbatim, so the ledger, the API and the agent cannot disagree about
what a refusal means. Three nested tags — `status`, `reason`, `kind` — are distinct on purpose:
a newtype variant under a shared tag emits a duplicate key and will not deserialize.

`NotVaulted` is reachable for a user whom `link-state` calls `linked` — which is exactly why it is
its own arm: the agent must not tell such a user things are working. Only the six refusals
`temper_principal::admit` can produce are reachable here, pinned by
`only_admit_reachable_refusals_ever_surface` (`slack_link_state.rs:173`).

Pinned end to end by `mint_reports_not_vaulted_distinctly_from_not_linked`
(`slack_link_test.rs:1761`) and `an_unapproved_principal_links_but_cannot_mint_until_approved`
(`:1526`) — the latter asserting a born-Denied human is refused on **standing**, not told to
re-link. On the agent side `tests/identity.test.ts` and `tests/slack-dispatch.test.ts` assert the
replies stay distinct, that **only** the unlinked one carries a URL, and that the re-link remedy
never leaks into a standing reply.

---

## Known residual risks

Stated with honest severity and status. None of these is hypothetical hand-waving; each is a
consequence of a deliberate trade recorded in the code.

### 1. First-link URL theft — **open by design, bounded**

**Severity: moderate. Status: accepted; partially mitigated.**

The authorize URL is a **bearer capability with no browser binding**. Whoever opens it and completes
the IdP login binds *their* profile to the victim's Slack principal, and thereafter receives that
person's `@temper` traffic.

Two mitigations bound it, and both are real:

- **An already-linked user is never issued a URL.** The link-state read comes first and
  short-circuits; the linked arm mints nothing (`slack_link.rs:93-99`).
- **Rebind is refused atomically** (`slack_link_service.rs:198-226`).

Together those close the *already-linked* case outright. What remains is: **victim not yet linked,
attacker steals their first-link message.** The URL is delivered as a channel-root **ephemeral**
visible only to the mentioning user, so the attacker must steal a message only the victim can see —
and the intent carries a 15-minute TTL (`INTENT_TTL`, `slack_link.rs:23`) and is single-use.

> **The sting is in the tail.** The no-rebind guard that closes the already-linked case makes a
> *wrong first binding* **unrecoverable by the victim** — they cannot link, because the principal is
> taken, and they cannot unbind it, because they do not own it. Recovery requires
> `temper admin slack disconnect` by a system admin. The refusal page names this (it admits the
> principal *is* linked, a deliberate bounded disclosure) but the other profile's handle is never
> revealed (`slack_link.rs:252-269`).

### 2. `internal_sig` binds neither method nor path — **key separation is the sole defence**

**Severity: moderate. Status: accepted, and the compensating control is enforced in CI.**

The MAC covers `"{timestamp}.{body}"` and nothing else (`internal_sig.rs:38-46`). It does **not**
cover the HTTP method or the request path. Both Slack internal routes take a body of the identical
shape — `{"slack_principal_id": "..."}`.

The consequence: a validly-signed link-state request body is byte-identical to a validly-signed mint
request body. **The only thing preventing a captured link-state call from being replayed against the
mint route is that the two routes verify under different keys.** That is precisely why
`audit-signature-secrets.sh` computes distinctness rather than baselining it, and why
`slack_mint_internal_routes` is a third router rather than one more route on
`slack_link_internal_routes` (`routes.rs:278-295`; layered at **both** merge sites — `create_app`
`:386-390` and `create_internal_app` `:435-439`).

> **The two-merge-site hazard is itself defended, and the story is instructive.** Because each
> layer name appears twice in `routes.rs`, deleting *one* of the two mounts left the name still
> present elsewhere in the file — so a whole-file `grep -q` **stayed green through exactly that
> edit, while one deployed surface served the route ungated.** `audit-route-auth.sh:152-158` now
> asserts the layer **per builder** (`require_layer_in ... $APP_BUILDERS`). Its own rationale is
> blunt about the stakes: "For `slack_mint` that is not a downgrade to authenticated-but-broad, it
> is act-as-any-user."

If a fourth gate is ever added with a same-shaped body, key separation is again the entire defence.
Binding method+path into the MAC would be the structural fix; it is not implemented.

### 3. A signed request is replayable within the skew window — **accepted**

**Severity: low. Status: accepted.**

`timestamp_is_fresh` accepts ±30s in either direction (`MAX_SKEW_SECS = 30`,
`internal_sig.rs:36,66-68`). There is no nonce cache and no single-use tracking of signatures, so a
captured request can be replayed against the same route for up to ~60 seconds of wall clock.

For link-state this is near-harmless (it is a read; the linked arm writes nothing). For mint it
means an attacker with a captured request can obtain the same principal's token during that window
— which, given the token is then valid for its full TTL anyway, does not meaningfully extend their
reach beyond capturing the response directly.

### 4. The IdP refresh-token rotation dual-write window — **deliberate; mitigated only out-of-repo**

**Severity: low (availability, not confidentiality). Status: documented, unmitigated in code.**

The RT rotation is an IdP side effect that cannot be made atomic with a local commit. If the process
dies or the `COMMIT` fails **after** Auth0 returns a rotated RT but **before** the `UPDATE` commits,
the row keeps a dead RT and the next mint trips reuse-detection, bricking the grant until the user
re-links (`slack_grant_vault_service.rs:216-223`):

> No row lock can make an external HTTP effect atomic with a local commit.

The row lock (`SELECT ... FOR UPDATE OF v`) *does* serialize concurrent mints of the same principal,
so two simultaneous mentions never both spend the same stored RT — but it does not and cannot close
this window.

> **The mitigation is an Auth0 refresh-token-rotation *leeway* setting, and nothing in this repo
> verifies it is enabled.** It is out-of-repo configuration, asserted by neither a test nor a CI
> guard nor a startup check. An operator who skips [slack-setup.md
> § Step 1.4](slack-setup.md#step-1--the-idp-client-auth0-or-the-temper-as) gets no warning — only
> occasional users whose grant bricks. Recovery in that case is a re-link.

### 5. Credential-bearing types and derived `Debug` — **closed, and now guarded**

**Severity: was low-moderate. Status: FIXED. Kept here because the failure mode recurs.**

Two types held credentials behind a *derived* `Debug`, so anything formatting them with `{:?}`
would have written a secret to the platform log:

- `ApiConfig` (`crates/temper-services/src/config.rs`) — `internal_reconcile_secret`,
  `embed_dispatch_secret` and `slack_mint_secret`, the keys behind all three signature gates.
- `TokenResponse` (`crates/temper-auth/src/token.rs`) — the plaintext refresh token *and* access
  token, bound on the mint path in `slack_grant_vault_service::mint_access_token`.

Both now hand-write a redacting `Debug`, joining `MintOutcome`, `NewGrant`, `SlackMintResponse`,
`VaultKey` and `SlackLinkConfig`. No leak was ever live — no call site formatted either type — so
this was a latent trap, not an incident.

**Why it is worth remembering how these two were missed**, because the pattern will recur:

`SlackLinkConfig`'s hand-written impl carries this rationale — a derived `Debug` *"would print it
verbatim wherever this **or the enclosing `ApiConfig`** is formatted."* The author reasoned about
the parent by name, protected the nested config, and left the parent derived. The hazard was
identified and then not acted on, in the same sentence.

`TokenResponse` was missed for a different reason: it lives in a **different crate**
(`temper-auth`), and the redaction convention had propagated only within the crates where it was
first written.

The convention was enforced by nothing at all — no lint, no trait bound, no test. It is now
enforced by `.github/scripts/audit-credential-debug.sh`, which fails CI on a credential-bearing
type with a derived `Debug`. Treat that guard, not the convention, as the thing keeping this
closed: a convention that had been stated *and reasoned about* still failed to protect the type it
named.

### 6. Vault key rotation is a flag day — **accepted**

**Severity: low. Status: accepted and documented.**

There is one key. Rotating `SLACK_VAULT_ENC_KEY` makes every stored grant unreadable; affected users
re-link. The schema reserves a `key_version` column for a future keyring, **stamped `1` and not yet
read** (`migrations/20260717000030_slack_grant_vault.sql:23`) — do not treat rotation as seamless.
Disconnect is deliberately built to keep working across a rotation, because the reason to rotate is
compromise, which is exactly when the unbind lever must work.

---

## What defends these claims

An operator should know which properties are enforced and which are conventions.

| Property | Defended by | Kind |
|---|---|---|
| Each signature gate reads a distinct secret | `.github/scripts/audit-signature-secrets.sh` (**computed**, not baselined) | CI guard |
| Every route's auth posture; no silently-dropped layer | `.github/scripts/audit-route-auth.sh` | CI guard |
| No new credential type silently derives `Debug` | `.github/scripts/audit-credential-debug.sh` | CI guard (baselined) |
| Grant-sink chokepoint | `.github/scripts/audit-grant-sinks.sh` | CI guard |
| The guards themselves fail when the thing they protect breaks | `test-audit-*.sh` × 4, in the `guard-tests` job | meta-guard |
| Layer present in **each** app builder, not just somewhere in the file | `audit-route-auth.sh:152-158` + `test-audit-route-auth.sh:88,96` | CI guard |
| The mint route rejects the link key (**bidirectionally**) / forged / unsigned calls | `slack_link_test.rs:1506`, `:1544` (drive the **route**) | e2e |
| The link-state route rejects a forged signature | `slack_link_test.rs:799` | e2e |
| An already-linked user is never issued a stealable URL | `slack_link_test.rs:731` | e2e |
| A link with no refresh token writes nothing and reports failure | `slack_link_test.rs:1587` | e2e |
| A transplanted ciphertext will not open (AEAD associated data) | `slack_grant_vault_service.rs:723` | unit |
| A deactivated profile mints nothing | `slack_grant_vault_service.rs:687` | unit |
| Minting disabled without its secret; linking unaffected | `slack_link_test.rs:1748` | e2e |
| `not_vaulted` vs `not_linked` stay distinct | `slack_link_test.rs:1761` | e2e |
| A refused human is told the remedy that works (standing ⇒ admin, never re-link) | `slack_link_test.rs:1526` (server) · `tests/identity.test.ts` "offers re-link ONLY where re-linking is the actual remedy" (agent) | e2e + unit |
| Only `admit`-reachable refusals surface on the mint | `slack_link_state.rs:173` | unit |
| Link is lookup-only (no profile creation) | `slack_link_test.rs:455` | e2e |
| State nonce is single-use | `slack_link_test.rs:505` | e2e |
| No rebind to a different profile | `slack_link_test.rs:622` | e2e |
| Disconnect unbinds; next mention re-prompts | `slack_link_test.rs:954`, `:1316` | e2e |
| Admin disconnect refuses a non-admin | `slack_link_test.rs:1116` | e2e |
| Rust/TS HMAC constructions cannot drift | shared known-answer vector: `internal_sig.rs` tests + `tests/oauth/wire-contract.test.ts` | contract test |
| Agent never signs a mint with the link key | `mention/tests/mint.test.ts:42-56` | unit |
| `getToken` fails closed on every non-token status | `mention/tests/mcp-auth.test.ts:73-89` | unit |
| No token reaches any log sink | `mention/tests/mcp-auth.test.ts:97-134` | unit |
| Tool allow-list stays read-only | `mention/tests/mcp-auth.test.ts:143-196` (exact list + mutating-name-family scan) | unit |
| All four principal shapes accepted, none parsed | `slack_link.rs:527-536`; `mention/tests/identity.test.ts:44-121` | unit |
| No new public eve event sink after an upgrade | `mention/tests/events.test.ts` (derives from eve's real `defaultEvents` **at runtime**) | unit |

**Convention only — not enforced anywhere:** Auth0 rotation leeway being enabled; eve's own inbound
Slack signature verification (framework code, covered by no test in this repo); the one-app :
one-workspace deployment ceiling (a structural property of eve + `@vercel/connect`, documented in
`mention/CLAUDE.md`, not asserted).

**Coverage gaps worth knowing:**

- `handlers/slack_mint.rs` and `services/slack_mint_service.rs` contain **no `mod tests` at all**.
  The mint path is covered exclusively by the e2e suite and the vault-service unit tests. That is
  defensible — per `slack_mint_service.rs:39-41`, a test that calls `mint_for_mention` directly
  bypasses the only thing enforcing authorization, so route-level e2e is the coverage that *means*
  something — but it does mean there is no fast local signal on this file.
- No test asserts the *absence* of extra fields on the agent's request bodies. The link and mint
  body assertions check the principal and the URL, not that the body has no other keys — so
  "no email crosses the wire" is currently a property of the code, not a defended invariant.

---

## Related

- [slack-setup.md](slack-setup.md) — the operator runbook: Slack app, IdP client, env, verification,
  disconnect UX, troubleshooting.
- [machine-credentials.md](machine-credentials.md) — the *other* non-human credential model. A Slack
  minted token is emphatically **not** a machine principal: it acts as a human, with a human's
  reach.
- [../development/security-audit-playbook.md](../development/security-audit-playbook.md) — how to
  re-verify this trust boundary from the surfaces in.
- [enterprise-install.md](enterprise-install.md) — the full environment-variable surface.
- [`packages/agent-workflows/mention/CLAUDE.md`](../../packages/agent-workflows/mention/CLAUDE.md) —
  agent internals: eve's inbound identity contract, the ephemeral delivery rules, the
  one-workspace ceiling.
