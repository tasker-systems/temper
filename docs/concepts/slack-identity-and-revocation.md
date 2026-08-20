# Slack Identity and Revocation

**For operators and integrators** — anyone who needs to reason about how a
Slack mention becomes a Temper action, what credentials are retained, and what
revocation actually stops.

## What the Slack integration does

A Slack human mentions `@temper` in a channel. The mention agent — an eve app
that watches Slack — asks Temper whether that Slack principal is linked to a
Temper profile, and if so, mints an access token that acts as that human under
their own reach: their contexts, their resources, nobody else's. Nothing is
inferred, matched, or auto-provisioned. The link is established once, in a
browser, by the human's own consent.

The trust boundary this crosses is the same two-gate boundary every Temper call
crosses — see [the trust boundary](./trust-boundary.md). The Slack path adds
its own inbound boundary (Slack to the agent, verified by Slack's request
signature) and an internal boundary (agent to Temper, verified by a shared HMAC
secret), but a minted token then re-enters through the ordinary front door:
full JWKS validation, standard visibility predicates. The Slack path gets no
shortcut.

## The principal is an opaque string

A Slack principal is an opaque string with two to four segments, because the
team id is nullable and bots carry an extra segment:

| team id | author | principalId | principalType |
|---|---|---|---|
| yes | human | `slack:<team>:<user>` | `user` |
| yes | bot | `slack:<team>:bot:<user>` | `service` |
| no | human | `slack:<user>` | `user` |
| no | bot | `slack:bot:<user>` | `service` |

The string is treated as opaque everywhere — stored whole, compared whole,
logged whole. Temper deliberately never splits it on `:`. Only a human principal
(`principalType === "user"`) is admitted; bots surface as `service` and are
dropped. The human gate is written positively, so a principal type added by the
framework later is refused by default rather than admitted by accident.

## The binding lives in the identity table

The link row maps `(auth_provider = 'slack', auth_provider_user_id = <the whole
principal>)` to a `profile_id`, with a uniqueness constraint that means a
principal binds to exactly one profile.

The principal binds once. There is no rebind: a link attempt for a principal
already bound to a different profile is refused atomically — nothing is written.
"Start fresh" is an explicit disconnect, never a side effect of linking again.

The identity row holds **no secret** and must never grow one. Identity and
secret are deliberately separated: the secret lives in a separate table,
encrypted at rest.

## Why the link is keyed on the principal, not email

Because there is no email on the Slack wire. The mention agent transmits exactly
one field to Temper on both internal routes: `{"slack_principal_id": "..."}`.
No email field exists to read. The link row records this by leaving `email`
NULL.

An email-based auto-link is therefore not merely undesirable — it is not
expressible with what arrives. The email the flow *does* use is the one on the
IdP's token during the browser callback, which is a different channel entirely
and carries the user's own consent.

## Linking is lookup-only

The OAuth callback resolves the freshly-exchanged access token through a
lookup-only path — it resolves an *existing* profile or refuses. It never
auto-provisions one. Linking an existing identity is not a registration route:
a stray click does not mint an account or confer team reach.

## What credentials are retained

When a human completes the browser link, Temper stores two things, deliberately
separated:

1. **The identity row** — `auth_provider = 'slack'`, the opaque principal, the
   `profile_id`. This row holds no secret.

2. **The encrypted grant** — the human's refresh token (their independent grant
   from their own consent, never a copy of anyone's CLI token), sealed with
   XChaCha20-Poly1305. The database never sees the encryption key or any
   plaintext token. Each ciphertext is additionally bound to its principal, so
   a stolen database row cannot be replayed under another user.

A short-lived cached access token may also be present on the vault row; it is
derived from the refresh token and expires within the IdP's access-token TTL.

The encryption key is a single AEAD key held in the platform's secret store,
never in the database. Rotation is a flag day: changing the key makes every
stored grant unreadable, and affected users re-link. A future zero-downtime
keyring is reserved in the schema but not implemented today.

Three other secrets are in play but are **not retained** — they authenticate
calls, not people:

- The **Slack bot token** and **Slack signing secret** live on the agent only;
  Temper never sees them.
- The **link secret** and **mint secret** are shared HMAC keys that gate
  agent-to-Temper calls. The link secret answers "is this principal linked?";
  the mint secret gates the endpoint that vends an act-as-the-human token. They
  are deliberately different values, because one answers a question and the
  other confers reach.

## The relationship between Slack identity and Temper profiles

A Slack principal is not a Temper profile. It is an *auth link* — a binding from
an external identity to an existing profile. The profile existed before the link
(linking is lookup-only), and it persists after a disconnect (disconnect is not
deactivation). One profile may hold auth links from multiple providers; one
Slack principal maps to exactly one profile.

This is the same identity model described in
[auth identity](./auth-identity.md): Temper validates tokens from exactly one
issuer, and a Slack-minted token is a JWT from that issuer, carrying the linked
human's profile. What differs is how the token is obtained — a credential
exchange against a vaulted grant, not a browser login.

## Revocation: what disconnect actually stops

This is the section to read before treating disconnect as an offboarding
control.

`temper slack disconnect` (self-serve) and `temper admin slack disconnect
<principal>` (operator) both run one chokepoint that:

- **Deletes the identity row** — the principal is unlinked from its profile.
- **Destroys the encrypted grant** — the vault row is deleted, not flagged. The
  sealed refresh token stops existing.
- **Sweeps pending link intents** for that principal — a security step, because
  disconnect removes the no-rebind guarantee, and an intent minted just before
  it would otherwise survive as a live first-link URL for a now-unlinked
  principal.
- **Attempts to revoke the grant at the identity provider** — best-effort under
  an external IdP, atomic under the Temper AS.

### What disconnect does NOT do

- **It is not deactivation.** The profile, its team memberships, and its
  resources are untouched.
- **It is not an instant cutoff.** Disconnect stops *future* token mints; an
  access token already issued stays valid until its own expiry (up to the IdP's
  access-token TTL, commonly one hour). Token validation is stateless JWKS —
  there is no revocation list and no consultation of the grant vault. The vault
  governs *minting*; it has no say over a token already minted.
- **It does not uninstall the Slack app.** That is workspace-level and
  admin-only, which is precisely why a per-user disconnect has to exist.

**"Disconnected" means "cannot mint again." It does not mean "cannot act."**

To actually cut someone off, **deactivate the profile** — that is enforced per
request, at latency zero, on every surface. The mint path checks `is_active`
before decrypting anything, and the auth seam refuses a token whose profile is
deactivated.

### How long an issued token survives

The token's real validity is the JWT's own `exp`, set by the IdP when it minted
the token. Temper does not control it. The exposure window is the IdP's
access-token TTL — commonly one hour — and Temper cannot shorten it after the
fact.

### IdP revocation failure

Under an external IdP, revocation is best-effort. If it fails, the disconnect
still succeeds — the local grant is destroyed either way, so Temper can no
longer use it. The grant may remain live at the IdP until it expires; revoke it
from the IdP dashboard if that matters. Under the Temper AS, revocation is local
and happens in the same transaction as the deletes, so it cannot fail this way.

### After a vault key rotation

Rotating the encryption key makes every pre-rotation ciphertext unopenable —
that is the point. Disconnect still works: it destroys the identity row, the
grant row, and the intents, and reports that no IdP revocation was attempted
(it could not open the token). Revoke those grants at the IdP out-of-band.
Failing the disconnect here would be strictly worse: the situation that
motivates a key rotation is a compromise, which is exactly when the unbind lever
must work.

Temper deliberately does not keep the token around to retry revocation later —
doing so would preserve the exact secret the user just asked it to destroy.

## Reconnecting

Just mention `@temper` again. The principal is unlinked, so the normal flow
offers a fresh authorize URL — there is no special reconnect path.

## Further reading

- **The trust boundary the Slack path crosses:**
  [The Trust Boundary](./trust-boundary.md).
- **The auth-identity contract the minted token satisfies:**
  [Auth Identity](./auth-identity.md).
- **Governance and administration (profile deactivation, admin disconnect):**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
