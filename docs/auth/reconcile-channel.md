# The internal reconcile channel

When the Temper Authorization Server (native SAML) is about to mint a token, it first calls
`temper-api` at `POST /internal/saml/reconcile` to reconcile the user's team memberships
from their SAML-asserted groups. This is a **server-to-server** call between two co-deployed
siblings — not a browser-facing endpoint, not a JWT path.

Source: `crates/temper-api/src/middleware/internal_auth.rs` (the HMAC gate),
`crates/temper-api/src/handlers/internal_saml.rs` (the handler). Operator setup:
[../guides/self-hosting-saml.md](../guides/self-hosting-saml.md#3-map-idp-groups-to-temper-teamsroles-phase-2).

## Trust model: an HMAC signature over the body

The AS signs each request with `HMAC-SHA256(secret, "{timestamp}.{raw_body}")` and sends two
headers; the API (`require_internal_signature`) recomputes the MAC over the bytes it received
and rejects a stale timestamp. Two wins over sending the secret in a header:

- **The secret never crosses the wire.** Only a signature derived from it travels, so a
  captured request never leaks the secret.
- **Captured requests are replay-proof.** The signed timestamp must be within ±30s of the
  verifier's clock (`temper_core::internal_sig::MAX_SKEW_SECS`); a replayed request is stale.

| Header | Value |
|--------|-------|
| `X-Temper-Timestamp` | Unix seconds the signature was computed at |
| `X-Temper-Signature` | lowercase-hex `HMAC-SHA256(secret, "{timestamp}.{body}")` |

Still **fail-closed**: if `INTERNAL_RECONCILE_SECRET` is unset the endpoint is disabled and
every request is rejected, so an unconfigured instance simply does no group provisioning
(authentication still works). The AS and the API share one Vercel project env, so the secret
is the **same** value on both by construction.

**We MAC the raw body bytes, not a re-serialized form.** The signer HMACs the exact JSON
bytes it sends; the verifier buffers the exact bytes it received and HMACs those (before
deserializing). Because both operate on identical bytes, there is no cross-language
canonicalization to drift on — the same discipline every major webhook signature uses
(GitHub `X-Hub-Signature-256`, Stripe). The signing scheme lives once in
`temper_core::internal_sig` (shared home for the header names, the message format, and the
skew window); the TS signer (`packages/temper-cloud/src/oauth/reconcile.ts`) and the Rust
verifier are pinned together by a **shared known-answer test vector** (asserted in both
`internal_sig.rs` and `tests/oauth/wire-contract.test.ts`), so they cannot drift on the HMAC
construction. Rejection is verified end-to-end for wrong-secret, stale-timestamp, and
tampered-body in `crates/temper-api/tests/internal_saml_test.rs`.

### Secret strength & rotation

- **Length.** Use a `INTERNAL_RECONCILE_SECRET` of **≥32 random bytes**
  (e.g. `openssl rand -hex 32`). `temper admin saml provision` generates a strong one.
- **Rotation.** Because both functions read the secret from one shared Vercel project env,
  rotation is a single atomic swap: generate a new secret, set it on the project env, and
  redeploy both functions together. There is no dual-secret overlap window to manage —
  reconcile is fail-open at the ACS handler, so the brief redeploy gap at worst delays group
  provisioning until the next login, never blocks authentication.

## Why not an origin allow-list on Vercel

The instinctive control — "only accept this call from our own AS's origin/IP" — is
**security theater on Vercel serverless**, for three concrete reasons:

1. **A server-side `fetch` sends no meaningful `Origin`.** `Origin` is a browser
   same-origin-policy artifact; the AS's outbound call is not a browser request, so there is
   no `Origin` header to allow-list.
2. **Egress IPs aren't pinnable.** Serverless functions egress from a shifting pool of
   addresses; there is no stable source IP to allow.
3. **The two siblings share a deployment, not a network boundary.** They co-deploy in one
   Vercel project but there is no private network segment between them to gate on.

So the **secret itself is the sibling-trust signal** — it is the only thing that reliably
distinguishes "our AS" from any other caller in this topology. An IP/origin allow-list would
add ceremony and zero real assurance.

A true network boundary (making the API non-publicly-routable) is explicitly **out of
scope**: the same API also serves public OAuth/SAML endpoints, so it must stay reachable, and
private networking is Enterprise-tier Vercel. Not worth it versus hardening the secret.

## The federated path through the seam

The handler does two things: resolve the profile, then reconcile its `idp` memberships. The
resolve half goes through the [authorization seam](./authorization-seam.md)'s **federated
entry point**, `resolve_federated_human` — this endpoint was the third site hand-building a
`PrincipalKind::Human`, and a surface that can construct one can forge one.

Three properties are worth naming, because they are what make a non-JWT path safe to have at
all:

- **There is nothing to classify.** The assertion was already authenticated server-to-server
  by the co-deployed AS (the HMAC above) *before* the token is minted. So this path skips
  `classify` and the email ladder — the email is *asserted*, not resolved — and only
  resolves-or-JITs the profile the minted token will later resolve to.
- **`provider` is server config, never a payload field.** The seam is handed
  `state.config.auth_provider_name`, so the profile this endpoint resolves is the same one
  `authenticate_token` will resolve the AS's freshly minted token to. A payload-supplied
  provider would let the caller land the assertion on a *different* profile.
- **The machine gate still covers it.** `resolve_human_from_claims`'s machine-shape guard is
  the second, independent layer (see the [machine-token contract](./machine-token-contract.md)),
  and it sits on *this* path too: an assertion carrying an `@clients`-suffixed
  `external_user_id` is refused here exactly as it is everywhere else.

## Bounded blast radius

Even if the endpoint were reached by an attacker, the damage is bounded by design:

- It can only apply **operator-pre-configured** `kb_saml_group_mappings` — never arbitrary
  grants. An attacker cannot invent a team or a role; they can only trigger the mappings an
  operator already wrote.
- It **never touches `native` memberships** (added in-app or by join-request approval) or
  auto-join teams — reconcile manages only `source='idp'` rows.
- It **never deletes or deactivates a profile.** It *can* JIT-create one — the same
  resolve-or-JIT the token path performs on a first sign-in, for an identity the AS is about
  to mint a token for anyway. So the worst case is a spurious profile with only
  operator-mapped `idp` memberships, not an escalation of an existing one.

## Further hardening: edge rate-limiting

The HMAC signing above (auth-seam plan's Stage 3) is the load-bearing control and is shipped.
One operational companion remains **operator config, not code**: edge rate-limiting on the
reconcile path via Vercel Firewall/WAF, to blunt brute-force or flooding against the endpoint.
Configure it per instance; the code path does not enforce it.

**Out of scope (explicit):** a true network boundary (making the API non-publicly-routable).
The same API also serves public OAuth/SAML endpoints, so it must stay reachable, and private
networking is Enterprise-tier Vercel — not worth it versus the HMAC signing already in place.

Spec:
[../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md](../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md)
(Stage 3).
