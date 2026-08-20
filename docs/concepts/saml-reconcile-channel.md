# The SAML Reconcile Channel

**For operators** — anyone running a self-hosted Temper instance with SAML SSO. This is a
server-to-server call inside one deployment; it is not something an integrator or end user
interacts with.

## What it is

When the Temper Authorization Server (the native SAML mode) is about to mint a token, it first
calls the API to reconcile the user's team memberships from their SAML-asserted groups. This
is a **server-to-server** call between two co-deployed siblings — not a browser-facing
endpoint, not a JWT path.

## Trust model: an HMAC signature over the body

The AS signs each request with `HMAC-SHA256(secret, "{timestamp}.{raw_body}")` and sends two
headers; the API recomputes the MAC over the bytes it received and rejects a stale timestamp.

- **The secret never crosses the wire.** Only a signature derived from it travels, so a
  captured request never leaks the secret.
- **Captured requests are replay-proof.** The signed timestamp must be within ±30s of the
  verifier's clock; a replayed request is stale.
- **Fail-closed.** If the shared secret is unset, the endpoint is disabled and every request is
  rejected. An unconfigured instance simply does no group provisioning — authentication still
  works.

The AS and the API share one Vercel project env, so the secret is the same value on both by
construction. Rotation is a single atomic swap: generate a new secret, set it on the project
env, and redeploy.

## Why not an origin allow-list

The instinctive control — "only accept this call from our own AS's origin/IP" — does not work
on Vercel serverless:

1. A server-side `fetch` sends no meaningful `Origin` header (that is a browser artifact).
2. Serverless egress IPs are not pinnable (they shift across a pool).
3. The two siblings share a deployment, not a network boundary.

The **secret itself is the sibling-trust signal** — it is the only thing that reliably
distinguishes "our AS" from any other caller in this topology.

## Bounded blast radius

Even if the endpoint were reached by an attacker, the damage is bounded:

- It can only apply **operator-pre-configured** group mappings — never arbitrary grants. An
  attacker cannot invent a team or a role.
- It never touches memberships added in-app or by join-request approval — only IdP-sourced
  memberships.
- It never deletes or deactivates a profile. The worst case is a spurious profile with only
  operator-mapped memberships, not an escalation of an existing one.

## Further reading

- **The auth-identity contract this channel serves:**
  [Auth identity](./auth-identity.md).
- **Standing up SAML SSO (the operator playbook):**
  [Self-hosting with SAML](../playbooks/self-host-with-saml.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
