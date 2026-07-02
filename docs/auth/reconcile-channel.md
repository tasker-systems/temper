# The internal reconcile channel

When the Temper Authorization Server (native SAML) is about to mint a token, it first calls
`temper-api` at `POST /internal/saml/reconcile` to reconcile the user's team memberships
from their SAML-asserted groups. This is a **server-to-server** call between two co-deployed
siblings — not a browser-facing endpoint, not a JWT path.

Source: `crates/temper-api/src/middleware/internal_auth.rs`. Operator setup:
[../guides/self-hosting-saml.md](../guides/self-hosting-saml.md#3-map-idp-groups-to-temper-teamsroles-phase-2).

## Trust model today: a shared secret

The endpoint is gated by a static shared secret in the `X-Temper-Internal-Secret` header,
compared constant-time against `INTERNAL_RECONCILE_SECRET`. The comparison is **fail-closed**:
if the secret is unset the endpoint is disabled and never matches, so an unconfigured
instance simply does no group provisioning (authentication still works).

```rust
// internal_auth.rs — length-checked, no early return on content
fn secret_matches(presented: &str, configured: Option<&str>) -> bool { … }
```

The AS and the API share one Vercel project env, so `INTERNAL_RECONCILE_SECRET` is set to
the **same** value on both by construction.

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

## Bounded blast radius

Even if the endpoint were reached by an attacker, the damage is bounded by design:

- It can only apply **operator-pre-configured** `kb_saml_group_mappings` — never arbitrary
  grants. An attacker cannot invent a team or a role; they can only trigger the mappings an
  operator already wrote.
- It **never touches `native` memberships** (added in-app or by join-request approval) or
  auto-join teams — reconcile manages only `source='idp'` rows.
- It is **purely authorization**: it never creates, deletes, or deactivates a profile.

## Hardening lever (planned): HMAC + timestamp signing

The honest upgrade for this topology — tracked as the auth-seam plan's Stage 3 — replaces
the raw header with a signed request:

- The AS signs `HMAC(secret, canonical_body ‖ timestamp)`; the API verifies the MAC and
  rejects stale timestamps (~30s window).
- Wins over the raw header: the **secret never travels the wire**, and the call becomes
  **replay-proof**. Same trust model, meaningfully hardened.
- The canonical-body + timestamp contract is a shared wire concern across a TS signer
  (temper-cloud) and a Rust verifier (`internal_auth.rs`) — it must be pinned once (a small
  typed contract in temper-core with ts-rs, or at minimum a documented canonicalization) so
  the two sides cannot drift on byte order.
- Operational companions: a strong/rotated `INTERNAL_RECONCILE_SECRET` (≥32 random bytes,
  with a **documented rotation procedure**) and edge rate-limiting (Vercel Firewall/WAF) on
  the path.

Spec:
[../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md](../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md)
(Stage 3). **Not yet built** — today the static secret is the control.
</content>
