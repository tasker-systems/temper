# Self-host Temper with a SAML IdP

**For operators** — anyone running a self-hosted Temper deployment whose identity provider
speaks **SAML 2.0**. This covers native SAML single sign-on: Temper fronts your SAML Identity
Provider (IdP) with a minimal OAuth 2.0 Authorization Server (AS) built into Temper. Your SAML
IdP authenticates the user; the Temper AS mints a short-lived EdDSA-signed Temper JWT that
`temper-api` trusts.

By the end you will have a Temper instance in Temper-AS mode — your SAML IdP authenticating
users, the built-in AS minting tokens both surfaces validate, CLI and (optionally) UI login
flowing through SAML, MCP clients connected, and IdP groups mapped to Temper teams and roles.

Use this when your organization's IdP speaks SAML 2.0 (e.g. Okta, Entra ID, PingFederate,
Shibboleth) and you want a native SP integration rather than an OIDC bridge. For the OIDC
bridge through Okta, see [Self-host with Okta](./self-host-with-okta.md).

> **Doing a full ground-up enterprise install?** This playbook is one phase. For the single
> end-to-end sequence (deploy → SAML → org → agents) see
> [enterprise install](./enterprise-install.md).

> **This playbook is the operator runbook.** For the *security model* it implements — how
> tokens are verified, the authorization boundary, and where it is enforced — see
> [The Trust Boundary](../concepts/trust-boundary.md). For the auth-identity contract this mode
> configures, see [Auth identity](../concepts/auth-identity.md).

## Prerequisites

- **A base Temper deployment.** This page assumes you have completed
  [Self-hosting Temper](./self-host-temper.md) through the non-auth steps. SAML replaces the
  Auth0/OIDC auth provisioning with the Temper AS; everything else (Neon, Vercel, routing,
  verification mechanics) is shared.
- **The auth-identity contract.** A Temper instance validates tokens from exactly one issuer
  against one audience. In SAML mode the Temper AS *is* that issuer, and the agreement rules
  differ from an external-IdP install. Read [Auth identity](../concepts/auth-identity.md) for
  the contract and the rules the server enforces at boot.

## How it works

```text
Browser ──(1)──▶ /oauth/authorize ──▶ /oauth/saml/login ──(2)──▶ SAML IdP
                                                                     │
   ┌───────────────(4) code ◀── /oauth/saml/acs ◀──(3) signed assertion
   ▼
/oauth/token ──(5) EdDSA JWT ──▶ temper-api  (validates via /oauth/jwks, JIT-provisions the profile)
```

1. The CLI/UI starts an OAuth authorization-code + PKCE flow at `/oauth/authorize`.
2. Temper redirects to your SAML IdP (SP-initiated).
3. The IdP posts a signed assertion back to the AS's ACS endpoint.
4. The AS validates the assertion, maps it to claims, and issues a one-time code.
5. The client exchanges the code at `/oauth/token` for an EdDSA-signed access token.
   `temper-api` validates it against the AS's published JWKS and just-in-time provisions the
   profile.

The AS is **SP-initiated only**, supports a **single active IdP** per instance, and maps a
**persistent NameID** (or a configured stable-id attribute) to the token `sub`. A validly
signed assertion implies `email_verified: true`.

## Quickstart with `temper admin saml` (recommended)

Rather than hand-assembling the keys, environment, and SQL documented in the sections below,
an operator working from a repo checkout can generate them with the **`temper admin saml`**
command group. It is an *emitter*: it prints the exact env bundle and SQL (or writes them with
`--env-out` / runs them with `--apply`), keeping the AS↔API shared values (`AS_AUDIENCE` ==
`AUTH_AUDIENCE`, `AUTH_ISSUER` == `AS_ISSUER`, the one `INTERNAL_RECONCILE_SECRET`,
`AUTH_PROVIDER_NAME` == `saml:<idp-key>`) consistent by construction. The sections that follow
remain the authoritative reference and the manual fallback.

1. **Provision keys + env + the IdP row** — before anyone can log in:

   ```bash
   temper admin saml provision \
     --instance-url https://<instance> --idp-key <acme-okta> \
     --idp-cert-file idp.pem --idp-sso-url https://idp.example.com/sso --idp-entity-id http://idp \
     --client temper-cli=https://<instance>/api/auth/cli-callback \
     --client temper-ui=https://<app-url>/auth/callback
   ```

   Omit flags for interactive prompts, or add `--no-interactive` for a scripted run. It
   generates the Ed25519 signing key (`AS_SIGNING_KEY_PKCS8`), `AS_SIGNING_KID`, and a strong
   `INTERNAL_RECONCILE_SECRET`, then emits the full env bundle and the `kb_saml_idp` INSERT to
   stdout. `--env-out .env.saml` writes the env (mode 0600 — it holds the private key); `--apply`
   runs the SQL against `$DATABASE_URL`. Paste the env into **both** Vercel functions and deploy.

2. **Map IdP groups to teams** — after the teams exist (see the org-bootstrap playbook):

   ```bash
   temper admin saml map-group --idp-key <acme-okta> engineering +engineering --role member
   temper admin saml map-group --idp-key <acme-okta> --from-seen   # groups the IdP has actually asserted
   ```

   Emits a `kb_saml_group_mappings` INSERT (add `--apply` to run it). `--from-seen` reads
   `kb_saml_seen_groups` so you can add mappings reactively.

3. **Verify**:

   ```bash
   temper admin saml verify --instance-url https://<instance> --db
   ```

   Confirms AS metadata/JWKS are reachable, that you resolve as a **system admin** (a missing
   `gating_team_slug` otherwise fails silently with 403s), and — with `--db` — that exactly one
   active `kb_saml_idp` row exists.

> **Ordering.** SAML setup *brackets* the [org-bootstrap playbook](./bootstrap-an-org.md): run
> `provision` + deploy + apply the IdP row **before** the first admin can log in, then run the
> org-bootstrap (which creates the teams), then run `map-group` **after** those teams exist. See
> that playbook's interleave note.

## Register Temper as an SP with your IdP

In your IdP, create a new SAML application ("SP") with:

| Setting | Value |
| --- | --- |
| ACS (Assertion Consumer Service) URL | `https://<instance>/oauth/saml/acs` |
| SP Entity ID / Audience | a stable URI you choose, e.g. `https://<instance>/saml/metadata` |
| NameID format | **persistent** (recommended) — becomes the token `sub` |
| Sign assertions | **yes** (both the `<Response>` and the `<Assertion>` must be signed) |

Add two attribute statements to the assertion:

- an **email** attribute (e.g. `email`) — becomes the token `email`.
- a **stable identifier** attribute (e.g. `uid`) — the fallback for `sub` when the NameID is
  not persistent.

Temper publishes its SP metadata at `https://<instance>/oauth/saml/metadata` for import into
IdPs that accept SP metadata XML.

## Configure the active IdP (`kb_saml_idp`)

The IdP configuration lives in the database, not env. Insert exactly **one** active row (flip
`is_active` to rotate to a replacement). To roll the IdP's *signing certificate* without an
authentication outage, see [Rotate the IdP's signing certificate](#rotate-the-idps-signing-certificate)
below — that is a different operation and it does not involve `is_active`:

```sql
INSERT INTO kb_saml_idp (
  idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id,
  sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr
) VALUES (
  'acme-okta',                                    -- idp_key: your label for this IdP
  true,
  -- idp_cert: the IdP's signing cert. Dollar-quoted so the line breaks are real -- a plain SQL
  -- literal does not interpret \n, and a certificate in that shape is not usable.
  $cert$-----BEGIN CERTIFICATE-----
MIIC...
-----END CERTIFICATE-----$cert$,
  'https://idp.acme.com/app/xxx/sso/saml',        -- idp_sso_url: the IdP SSO (redirect) endpoint
  'http://www.okta.com/xxx',                       -- idp_entity_id: the IdP's entity id
  'https://<instance>/saml/metadata',              -- sp_entity_id: MUST match the Audience you set above
  'https://<instance>/oauth/saml/acs',             -- acs_url
  'urn:oasis:names:tc:SAML:2.0:nameid-format:persistent',
  'email',                                          -- email_attr: the assertion attribute for email
  'uid'                                             -- stable_id_attr: fallback sub source
);
```

## Rotate the IdP's signing certificate

Your IdP will re-key on its own schedule, and the SAML ACS is the only human authentication door on
an AS-mode instance. `kb_saml_idp` therefore holds **two** certificate slots: `idp_cert` and
`idp_cert_secondary`. An assertion signed by **either** is accepted, and that overlap is what lets
the IdP cut over whenever it likes instead of at a moment you have to coordinate.

`idp_cert_secondary` is `NULL` outside a rotation. Requires migration `20260827000010`; on an
instance that has not run it the column does not exist and step 1 fails saying so.

> [!IMPORTANT]
> **Paste the PEM with real line breaks.** A plain SQL literal does not interpret `\n` — under the
> default `standard_conforming_strings` you get the two characters, not a newline, and a
> certificate in that shape is not usable. Use a dollar-quoted literal, as the examples below do,
> or paste the PEM across real lines. A slot that does not hold a usable certificate is refused by
> a constraint on write, so you will hear about it here rather than at step 3.

**1. Add the incoming certificate.**

```sql
UPDATE kb_saml_idp
   SET idp_cert_secondary = $cert$-----BEGIN CERTIFICATE-----
MIIC...
-----END CERTIFICATE-----$cert$
 WHERE idp_key = 'acme-okta';
```

**2. Check that what landed is the certificate you meant**, by fingerprint. This is the step that
catches a wrong or truncated paste, and it matters because nothing later will: logins are still
being signed by the *outgoing* key, so a login test at this point passes no matter what is in the
second slot. Compare against the fingerprint your IdP publishes.

```bash
psql "$DATABASE_URL" -tAc \
  "SELECT idp_cert_secondary FROM kb_saml_idp WHERE idp_key = 'acme-okta'" \
  | openssl x509 -noout -fingerprint -sha256
```

Sign in once as well, to confirm step 1 broke nothing. To undo: `SET idp_cert_secondary = NULL`.

**3. Let the IdP cut over.** Do this at the IdP, on its schedule. Logins keep working throughout,
signed by whichever key the IdP happens to be using, and they keep working if it cuts back.

**4. Drop the outgoing certificate.**

```sql
UPDATE kb_saml_idp
   SET idp_cert = idp_cert_secondary,
       idp_cert_secondary = NULL
 WHERE idp_key = 'acme-okta'
   AND idp_cert_secondary IS NOT NULL;
```

The `AND` matters: without it, running this a second time would try to move a `NULL` into
`idp_cert`. Steps 1–3 are each reversible on their own. **Step 4 is not** — it discards the
outgoing certificate, and putting it back means having kept a copy outside the database. Keep one
until you are sure.

Step 4 stops the retired certificate being accepted at that write, with no cache to wait out —
`loadActiveIdp` reads the row on every request. **That bounds new logins only.** Sessions already
issued continue: an access token for up to `AS_ACCESS_TTL_SECONDS`, and a refresh chain for up to
`AS_REFRESH_CHAIN_MAX_SECONDS` from its last full login. If you are rotating because a signing key
was *compromised*, step 4 is not the control you need — revoke the affected principals, which ends
their chains in the same transaction (see [Deactivating an account](#deactivating-an-account-authn-control)).

> [!NOTE]
> Two slots widen the accepted signers by **exactly one named certificate**, and only for this IdP.
> A certificate belonging to some other IdP, or to none, is refused during an overlap window exactly
> as it is outside one, and the instance still has a single active IdP throughout.
>
> `temper admin saml verify --db` checks that there is exactly one active IdP row. It does **not**
> inspect either certificate, so it stays green through every step above, including a rotation you
> have half-finished. It is not a check on the rotation.

## Map IdP groups to Temper teams and roles

Temper reconciles team membership from SAML-asserted groups **on each login**. This is
eventual, not immediate: a user removed from a group keeps access until their session expires
and they next log in. For immediate deprovisioning use SCIM (not yet available).

**Reconcile only ever manages `source='idp'` memberships. Native memberships (added in-app or
by join-request approval) and auto-join teams are never touched — if a user is already a native
member of a team, the IdP reconcile skips that team for them entirely.** Group provisioning is
purely authorization; it never creates, deletes, or deactivates the profile itself.

1. Tell the SP which assertion attribute carries the group list:

   ```sql
   UPDATE kb_saml_idp SET groups_attr = 'groups' WHERE idp_key = 'acme-okta';
   ```
   Leave `groups_attr` NULL to keep authentication-only behavior (no membership changes).

2. Map groups to `(team, role)`. Teams must already exist. Two groups mapping to the same team
   collapse to the strongest role (`owner > maintainer > member > watcher`):

   ```sql
   INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) VALUES
     ('acme-okta', 'engineering',   '<team-uuid>', 'member'),
     ('acme-okta', 'eng-leads',     '<team-uuid>', 'maintainer'),
     ('acme-okta', 'temper-admins', '<gating-team-uuid>', 'owner');
   ```
   The last row is "admin via group" — it makes members of `temper-admins` owners of the gating
   team. Note: the **first** admin still requires the SQL bootstrap step; SAML does not
   bootstrap the system.

Unmapped asserted groups are ignored for provisioning, but they ARE recorded in
`kb_saml_seen_groups` (with first/last-seen) so you can discover what the IdP actually sends and
add mappings reactively — the mapping table never needs to be pre-populated:

```sql
-- What groups has the IdP actually asserted? (add mappings for the ones you care about)
SELECT group_value, first_seen, last_seen FROM kb_saml_seen_groups
 WHERE idp_key = 'acme-okta' ORDER BY last_seen DESC;
```

**Removal semantics.** Removing a group from the assertion revokes the corresponding `idp`
membership on the next login. The distinction matters: if the assertion **omits the groups
attribute entirely** (e.g. a transient IdP misconfiguration), reconcile is **skipped** and no
memberships are revoked; only an assertion that carries the attribute with **no values** ("in no
mapped groups now") revokes all of the user's `idp` memberships.

## Environment variables

### Authorization Server (temper-cloud / the API deployment)

Generate an Ed25519 signing key:

```bash
openssl genpkey -algorithm ed25519 -out as_signing_key.pem
# AS_SIGNING_KEY_PKCS8 is the full PKCS#8 PEM contents of this file.
```

| Variable | Value | Notes |
| --- | --- | --- |
| `AS_ISSUER` | `https://<instance>` | The AS issuer URL. **Setting this flips the instance into AS mode** (it serves AS metadata/JWKS instead of Auth0). |
| `AS_AUDIENCE` | `https://<instance>/api` | Audience claim minted into tokens (must equal the `temper-api` `AUTH_AUDIENCE`). |
| `AS_SIGNING_KEY_PKCS8` | *(PEM contents)* | Ed25519 private signing key (PKCS#8 PEM). Keep secret. |
| `AS_SIGNING_KID` | e.g. `as-2026-07` | Key id published in the JWKS. |
| `AS_CLIENTS` | *(JSON, see below)* | **Required** allowlist of `client_id → [redirect_uris]`. Without it every `/oauth/authorize` is rejected (fail-closed). |
| `AS_ACCESS_TTL_SECONDS` | `900` (default) | Access-token lifetime. |
| `AS_REFRESH_TTL_SECONDS` | `2592000` (default, 30d) | Refresh-token lifetime. Slides on every rotation. |
| `AS_REFRESH_CHAIN_MAX_SECONDS` | `7776000` (default, 90d) | **Absolute** lifetime of a refresh chain, from the last full SAML login. Rotation never moves it, so this — not the two TTLs above — is the bound on how long IdP-removed reach can persist. See [Limitations](#limitations). |
| `AS_SAML_ASSERTION_MAX_SECONDS` | `3600` (default, 1h) | **The widest assertion validity window your IdP issues** — read it off the IdP, not off Temper. It is the floor below which a consumed assertion is never forgotten, so the retention sweep cannot delete a replay row that could still be replayed. Too large costs a little storage; too small re-opens replay. A value that is present but not a usable number of seconds **fails the sweep rather than substituting the default** — rows stay on disk until you fix it; unset or blank uses the default. **Read by `temper-api`, not by the AS** — if you run the two as separate processes, set it on `temper-api`. See [What is kept, and for how long](#what-is-kept-and-for-how-long). |
| `AS_REFRESH_REPLAY_GRACE_SECONDS` | `10` (default) | How soon after a rotation a client may present the spent refresh token again and be treated as a retry rather than a thief. Later than this and the whole chain is ended. `0` ends the chain on any replay; the widest value honoured is `3600`. See [Watch for replayed refresh tokens](#watch-for-replayed-refresh-tokens). |

`AS_CLIENTS` registers the exact redirect URIs each client may use (exact string match — this
is the control that prevents authorization-code exfiltration):

```json
{
  "temper-cli": ["https://<instance>/api/auth/cli-callback"],
  "temper-ui":  ["https://<app-url>/auth/callback"]
}
```

### `temper-api`

Point `temper-api` at the AS as its single issuer:

| Variable | Value |
| --- | --- |
| `JWKS_URL` | `https://<instance>/oauth/jwks` |
| `AUTH_ISSUER` | the same value as `AS_ISSUER` |
| `AUTH_AUDIENCE` | the same value as `AS_AUDIENCE` |
| `AUTH_PROVIDER_NAME` | `saml:<idp-key>` (e.g. `saml:acme-okta`) — namespaces the JIT auth link. Max 32 chars. |

### Group provisioning

These gate the internal reconcile call the AS makes to `temper-api` before minting a token. Set
`INTERNAL_RECONCILE_SECRET` to the **same** value on both the AS and the `temper-api` deployment
(they share a Vercel project env). If unset, the reconcile endpoint is disabled and no group
provisioning occurs (authentication still works).

> Why a shared secret rather than an origin/IP allow-list, and the endpoint's bounded blast
> radius, are explained in [The SAML reconcile channel](../concepts/saml-reconcile-channel.md).

| Variable | Where | Purpose |
| --- | --- | --- |
| `INTERNAL_RECONCILE_SECRET` | AS + API (shared) | Shared secret gating the internal reconcile call. Same value on both. Unset ⇒ reconcile disabled, no group provisioning. |
| `INTERNAL_RECONCILE_URL` | AS | Full URL of the `temper-api` `/internal/saml/reconcile` endpoint the AS calls before minting (e.g. `https://<your-api-origin>/internal/saml/reconcile`). |
| `INTERNAL_RESOLVE_URL` | AS | Full URL of the `temper-api` `/internal/principal/resolve` endpoint (e.g. `https://<your-api-origin>/internal/principal/resolve`). Gated by the same `INTERNAL_RECONCILE_SECRET`. The AS calls it to learn which profile a login belongs to, so an administrator's revoke can end that principal's refresh chains. Unset ⇒ chains are minted without an owner and are bounded by `AS_REFRESH_CHAIN_MAX_SECONDS` alone. |

### Slack account link (optional)

These are needed only if you run the **@temper mention agent**. Leave all three unset and the
link endpoint is disabled — everything else works unchanged.

The flow makes temper an OAuth **client** of your own AS: it authorizes as
`SLACK_LINK_CLIENT_ID` and is redirected back to `<PUBLIC_BASE_URL>/api/auth/slack/callback`.
That redirect URI must therefore appear in that client's `AS_CLIENTS` entry — `AS_CLIENTS` is
an exact-match allowlist and unset is fail-closed, so an unregistered URI is refused by the AS
before temper sees the request.

| Variable | Where | Purpose |
| --- | --- | --- |
| `SLACK_LINK_CLIENT_ID` | API | The `client_id` the link flow authorizes as. Must be present in `AS_CLIENTS`, with `<PUBLIC_BASE_URL>/api/auth/slack/callback` among its redirect URIs. |
| `SLACK_LINK_SECRET` | API + mention agent (shared) | Shared secret gating the agent's `/internal/slack/link-state` call. Same value on both. Unset ⇒ link endpoint disabled. |
| `PUBLIC_BASE_URL` | API | This instance's public origin (e.g. `https://<instance>`). The callback `redirect_uri` is derived from it. |

## Configure the CLI

Run the guided setup and pick the **Temper AS (native SAML)** provider:

```bash
temper init
```

Or non-interactively:

```bash
temper init --no-interactive --instance-url https://<instance> --idp temper-as
```

This writes an `[[auth.providers]]` block with `provider = "temper-as"`,
`client_id = "temper-cli"`, `authorize_url = https://<instance>/oauth/authorize`,
`token_url = https://<instance>/oauth/token`, and
`callback_url = https://<instance>/api/auth/cli-callback`. The existing PKCE + loopback login
flow is issuer-agnostic — no other CLI change is needed. `temper auth login` then authenticates
through SAML.

## Configure the UI (optional)

The SvelteKit UI logs in against the AS as a **public PKCE client** (no client secret). Set:

```bash
OIDC_ISSUER=https://<instance>
OIDC_CLIENT_ID=temper-ui
OIDC_DISCOVERY_URL=https://<instance>/.well-known/oauth-authorization-server
OIDC_AUDIENCE=https://<instance>/api
OIDC_PUBLIC_CLIENT=true
# No OIDC_CLIENT_SECRET — the Temper AS uses public PKCE clients.
```

`OIDC_DISCOVERY_URL` points the UI at the AS's RFC 8414 metadata (the AS does not serve
`/.well-known/openid-configuration`). Ensure `temper-ui`'s `<app-url>/auth/callback` is listed
in `AS_CLIENTS`. `OIDC_PUBLIC_CLIENT=true` is required for this secret-less path — without it,
the UI fails fast at startup rather than silently running with no client secret.

> **Single-origin ACS and CSRF (SAML only).** In the single-origin topology, `temper-ui`
> reverse-proxies `/oauth` to the API, so the SAML ACS is reached at `<app-url>/oauth/saml/acs`.
> The SAML **HTTP-POST binding** delivers the assertion as a browser-submitted form `POST` *from
> the IdP's origin* — a legitimately cross-origin POST that SvelteKit's built-in origin CSRF
> check would otherwise reject with `403 Cross-site POST form submissions are forbidden` before
> the proxy could forward it. `temper-ui` handles this: the built-in check is disabled and the
> equivalent origin guard is re-implemented in `hooks.server.ts` (scoped to the UI's own
> routes), after the proxied surface — including the ACS — has already been short-circuited
> upstream. The ACS POST is authenticated by the SAML layer itself (signature, audience,
> destination/recipient, replay guard), not by an `Origin` match. **No operator action is
> required** — this is built in. The **OIDC path does not hit this** at all: its callback
> completes as a `GET` redirect, which CSRF does not touch.

## Connect MCP clients (Claude Desktop / Claude Code)

The remote MCP server (`/mcp`) is served from the same deployment and authenticates against
the Temper AS via OAuth. MCP clients discover the AS through RFC 8414 metadata and **require
dynamic client registration (DCR)** — current Claude Code/Desktop ignore a client-side
`client_id` and fall back to DCR regardless. The AS metadata advertises a
`registration_endpoint` (`/oauth/register`), a thin proxy that echoes a pre-registered static
`client_id`; it never persists client-supplied redirect URIs, so the `/oauth/authorize`
open-redirect protection is unweakened. To enable it on a SAML instance:

1. **Set `MCP_CLIENT_ID`** on the deployment to a client id that is **also a key in
   `AS_CLIENTS`** (e.g. `temper-mcp`). Without `MCP_CLIENT_ID`, `/oauth/register` returns `503
   temporarily_unavailable`.

2. **Add that client to `AS_CLIENTS`** with the redirect URIs its clients use:

   ```json
   {
     "temper-cli": ["https://<instance>/api/auth/cli-callback"],
     "temper-ui":  ["https://<app-url>/auth/callback"],
     "temper-mcp": [
       "https://claude.ai/api/mcp/auth_callback",
       "https://claude.com/api/mcp/auth_callback",
       "http://127.0.0.1/callback"
     ]
   }
   ```

   The two HTTPS callbacks serve the Claude Desktop / web connector (fixed callbacks, exact
   match). The **loopback entry serves Claude Code**: it runs a local callback server on an
   *ephemeral* port, so the allowlist matches loopback redirect URIs by scheme + path with the
   port ignored — a port-less `http://127.0.0.1/callback` entry matches
   `http://127.0.0.1:<random>/callback`. Loopback matching is confined to the local machine and
   normalizes across loopback hosts (`127.0.0.1`, `localhost`, `[::1]`), so one loopback entry
   covers whichever the client sends. Non-loopback (HTTPS) redirect URIs are always
   exact-match.

3. **Audience/issuer alignment is enforced at boot** — you do not have to remember it. The AS
   mints `iss = AS_ISSUER`, `aud = AS_AUDIENCE`; **both** surfaces validate against the
   instance's one `AUTH_AUDIENCE`. If these disagree, the process **refuses to start** and
   names the offending variable:

   | AS mints | The instance validates | Enforced requirement |
   | --- | --- | --- |
   | `AS_ISSUER` | `AUTH_ISSUER` | `AS_ISSUER == AUTH_ISSUER` |
   | `AS_AUDIENCE` | `AUTH_AUDIENCE` | `AS_AUDIENCE == AUTH_AUDIENCE` |
   | (its JWKS) | `JWKS_URL` | `JWKS_URL == $AS_ISSUER/oauth/jwks` |

   `MCP_AUDIENCE` is **optional**. temper-mcp reads the instance's one audience, same as
   temper-api. If you do set `MCP_AUDIENCE`, it must **equal** `AUTH_AUDIENCE`; it is an
   assertion, not a second value.

   These already hold on any correctly-configured SAML instance — a divergent audience means
   no AS-minted token ever verifies. Temper names the rule and fails fast rather than leaving
   you to discover it as a 401.

Then add the server to Claude Code and authenticate:

```bash
claude mcp add --transport http temper https://<instance>/mcp
# then /mcp → authenticate
```

## Verify

1. `temper auth login` → a browser opens to your IdP; after SAML login the CLI receives a
   token.
2. `temper auth status` (or any authenticated command) succeeds.
3. A `kb_profiles` row and a `kb_profile_auth_links` row (with
   `auth_provider = saml:<idp-key>`) are created for the user on first login.

## Deactivating an account (authn control)

Team membership is **authorization**; it does not control whether an account can log in. To
stop an account from authenticating at all — regardless of what the IdP asserts — soft-delete
the profile:

```sql
UPDATE kb_profiles SET is_active = false WHERE id = '<profile-uuid>';
```

A deactivated profile is rejected by the API auth middleware (`401`) even with a valid token.
This never deletes the profile or its history, and it is independent of SAML group provisioning
(re-activating restores access). Reconcile/deprovisioning of a team never deactivates a
profile.

> `is_active` is enforced by the shared authorization seam, so **both** surfaces — `temper-api`
> and `temper-mcp` — reject a deactivated profile identically. See
> [The Trust Boundary](../concepts/trust-boundary.md).

## Running it as the applier

The `saml-setup.sh` script automates the `temper admin saml` sequence above from a declarative
profile (`saml-profile.yaml`) — it loops `provision` / `map-group` / `verify`. Run from a repo
checkout:

- **Dry-run** — prints the commands without executing:
  `saml-setup.sh --profile saml-profile.yaml --dry-run`
- **Emit only (default)** — env bundle + `kb_saml_idp` SQL, safe to run anytime, no DB writes:
  `saml-setup.sh --profile saml-profile.yaml`
- **Apply** — writes the `kb_saml_idp` row, applies group mappings, verifies against the live
  DB (needs `DATABASE_URL` + `psql`; run post-migrate, and after the org-bootstrap teams exist):
  `DATABASE_URL=postgresql://… saml-setup.sh --profile saml-profile.yaml --apply-db`

It needs `yq` to read the profile and `temper` on PATH. Emit-by-default and idempotency are
inherited from the underlying `temper admin saml` commands, not reimplemented by the script. It
is the SAML sibling of `system-bootstrap.sh` (kept separate so that script stays usable for
Auth0/Okta-OIDC installs) — see [enterprise install](./enterprise-install.md) for how the two
appliers interleave across the full install timeline.

## Limitations

- **Reconcile-on-login only.** Profile attributes refresh when the user logs in; there is no
  live deprovisioning. Automated deprovisioning (SCIM) is not yet available.

  **The bound to state is `AS_REFRESH_CHAIN_MAX_SECONDS`, not `AS_ACCESS_TTL_SECONDS`.** A session
  survives across access-token expiries by refreshing, so the access TTL describes how often a
  token is reminted, not how long a session lives. What bounds the session is the **chain's**
  lifetime: every refresh token belongs to a chain whose deadline is stamped at the last full SAML
  login and inherited unchanged by every rotation. Once it passes, the only way forward is a fresh
  login — which reconciles against the IdP, and brings the user's team reach back into agreement
  with what the IdP confers today.

  So the figure to give a review board for IdP-side removals is
  **`AS_REFRESH_CHAIN_MAX_SECONDS` + `AS_ACCESS_TTL_SECONDS`** — 90 days plus 15 minutes on the
  defaults. Lower `AS_REFRESH_CHAIN_MAX_SECONDS` to shorten it, at the cost of how often your users
  re-authenticate.

- **An administrator's revoke is immediate, and does not wait for any of the above.** Revoking or
  deactivating a principal ends their live refresh chains in the same transaction as the standing
  change, and no later sign-in mints them a new one. This is the path to use when you need a
  departure to take effect now; the chain lifetime is the backstop for the case nobody acted on.

  **What a revoked principal can still do, stated exactly**, because "immediate" is easy to
  over-read. They can complete a SAML sign-in and receive an **access token** — deliberately, since
  reaching `/api/access/review-request` to contest the revocation requires one. That token is
  refused on every data route, and it is **not** renewable: no refresh chain is minted for them, so
  it expires within `AS_ACCESS_TTL_SECONDS` and is not carried forward. If you need the sign-in
  itself to stop, disable the account at your IdP — that is the control temper does not own.

  A principal who has *not yet been approved* (or whose request was declined) keeps refreshing
  normally — also deliberately. Nothing has been taken away from them, and they hold a token in
  order to reach the join-request endpoints and ask for access; removing it would make asking
  harder, not safer.

> [!IMPORTANT]
> **Upgrading an existing SAML deployment?** `INTERNAL_RESOLVE_URL` is new, and nothing sets it for
> you — `temper admin saml provision` emits it only into a freshly generated bundle. Without it the
> AS cannot record which principal a refresh chain belongs to, so **an administrator's revoke ends
> no chains** and the only remaining bound is `AS_REFRESH_CHAIN_MAX_SECONDS`. Logins and refreshes
> keep returning `200` throughout, so there is no failure to notice.
>
> **The signal to look for** is the API's `standing terminal ended no refresh chains` warning when
> you revoke someone — but read its `ownerless_live_chains` field, not merely its presence. That
> warning is also the normal outcome for a machine principal and for anyone who simply was not
> signed in; a **non-zero** `ownerless_live_chains` is the part that means the authorization server
> is not recording owners at all. Add the variable, pointing at your API origin's
> `/internal/principal/resolve`, before relying on revoke.
- **Single active IdP** per instance, **SP-initiated** flows only.
- **Single issuer** per instance: an instance is either an AS/SAML instance (`AS_ISSUER` set)
  or an Auth0/OIDC instance, not both.
- Role/team mapping from SAML attributes is available once groups are configured (see
  [Map IdP groups to Temper teams and roles](#map-idp-groups-to-temper-teams-and-roles)).

## Watch for replayed refresh tokens

Refresh tokens are single-use: exchanging one revokes it and issues a successor. If a **spent**
token is presented again, one of two things happened — a client retried an exchange whose response
it never received (or raced two refreshes at once), or **someone else has a copy of the chain**.
RFC 6819 §5.2.2.3 treats the second as the more likely reading once enough time has passed, and
temper acts on it: the whole chain is ended, so the copy and the original both stop working and the
user signs in again.

**`AS_REFRESH_REPLAY_GRACE_SECONDS` is where you set the line between the two readings.** Inside
the window the replay is refused but the chain survives; outside it, the chain is ended.

Ten seconds is the default, and the width is worth understanding before you change it. The server
only ever sees the party that *lost* a rotation race, and cannot tell whether the one that won was
your user or someone holding a copy — so inside the window it lets the winner keep the session and
records the presentation as benign. Every second of width is a second in which a fresh theft gets
that treatment. What the window buys is the client that had two refreshes in flight at once, which
is one exchange still resolving and takes well under a second. A client that merely lost a response
and retried holds no live token either way, so a narrow window costs it nothing but a row in the
table below.

Raise it if you run clients on unreliable networks and would rather read past those rows — **up to
one hour, which is the widest value honoured**; anything larger, negative or non-numeric is read as
a units slip, logged, and replaced by the ten-second default. Set it to `0` for the strictest
reading, where any replay ends the chain.

Either way **the event is recorded**, so you can answer "has any chain been replayed?" without
having retained logs:

```sql
SELECT * FROM vw_oauth_refresh_replays;
```

A row means a spent refresh token came back. **Rows are not by themselves alarming** — a client
that loses a response and retries produces one, so a healthy instance accumulates some. Read
`hostile_count` first; it is the count of presentations that fell outside the grace window.

An empty result means nothing was *recorded*. That is not quite the same as nothing having
happened: if the server could not write the record it says so in its logs (`could not record or act
on a refused rotation`), which is the one case where this table and the logs have to be read
together.

| Column | Read it as |
| --- | --- |
| `profile_handle` | Who held the chain. `NULL` if the AS could not record an owner — see the `INTERNAL_RESOLVE_URL` note above; the replay is still real. |
| `first_replay_age` | How long after the rotation the token reappeared — the age the server judged it on, not a later reading. Hours or days has no benign explanation. **Seconds is *consistent with* a client retry, not proof of one**: a credential stolen and used immediately also reappears in seconds. |
| `replay_count` / `hostile_count` | How many times it was presented, and how many of those fell outside the grace window. `hostile_count = 0` on a chain you did not expect to be racing is still worth a look. |
| `chain_ended` / `chain_ended_at` | Whether the chain was ended in response, and when. |
| `tokens_revoked` | How many live tokens the ending actually revoked. `0` alongside `chain_ended` means the chain held nothing live at that moment — it is still ended, and no successor can be minted into it. |

A replay ends **that chain only** — not the principal's other sessions, and not their ability to
sign in again. If you conclude a credential really was stolen, revoke the principal:

```bash
temper admin access revoke <profile-id> --reason "refresh chain replayed"
```

That ends **every** chain they hold and stops the next sign-in from minting a new one — see
[Limitations](#limitations) for exactly what a revoked principal can still do.

> [!NOTE]
> **What this view covers:** chains issued by this instance from this version onward. Earlier
> chains are bounded by `AS_REFRESH_CHAIN_MAX_SECONDS` and are gone within that window, so an empty
> result means "nothing recorded", which becomes "nothing happened" once your longest chain has
> turned over.

## What is kept, and for how long

Three tables back the authorization server, and a row lands in one of them on **every** login. A
daily sweep (`/api/as/reap`, gated by the same `EMBED_DISPATCH_SECRET` bearer as the other internal
crons — no separate secret to set) deletes rows once they are past a floor. It deletes **at most
50,000 rows per table per night**, so a backlog accumulated before you upgraded drains over as many
nights as that arithmetic gives you rather than in one statement — check `count(*)` on the three
tables if you want to know how long.

| Table | A row per | Deleted once |
| --- | --- | --- |
| `kb_saml_replay` | consumed SAML assertion | `AS_SAML_ASSERTION_MAX_SECONDS` has passed since the row's stamped `expires_at` — so never sooner than that long after the assertion was consumed |
| `kb_oauth_flow` | authorization-code flow (including abandoned ones) | 1 day past `expires_at` |
| `kb_oauth_refresh_tokens` | issued refresh token | 30 days past **both** its own expiry **and** its chain's `chain_expires_at` |

> [!IMPORTANT]
> **Apply migrations before the first sweep runs.** The sweep's indexes and its `ON DELETE
> RESTRICT` on replay evidence ship as a migration, and the cron ships with the deployment. If the
> cron fires first, that run scans instead of seeking — it will exceed the function's 300s limit,
> commit whatever it managed, and try again the next night — and for that window the evidence
> guarantee above rests on the sweep's filter alone rather than on the constraint beneath it.

**Two of these floors are security properties, not housekeeping.**

A `kb_saml_replay` row is what stops a captured assertion being presented twice. Temper does not
read your IdP's `NotOnOrAfter`, so the row's own `expires_at` is an assumption about it rather than a
statement of it — which is why the sweep subtracts `AS_SAML_ASSERTION_MAX_SECONDS` instead of
trusting that stamp. **If your IdP issues assertions valid for longer than an hour, raise this
variable before the first sweep runs.** Leaving it too low deletes assertion ids that are still
replayable; leaving it too high costs you an assertion id and a timestamp per login.

A spent refresh token is kept until its whole chain is dead because that is what makes a **stolen**
one detectable: a rotated token presented again is recognised however long ago it expired, and the
chain is ended in response. Reaping on the token's own expiry would quietly turn that into an
ordinary "invalid grant".

**Detection does end when the row does.** A replay of a token whose chain died more than 30 days ago
is answered with a plain `invalid_grant`, logs nothing and records nothing. Before this sweep
existed nothing was ever deleted, so that reach was unbounded; 30 days past a dead chain is the
bound it now has.

Evidence already recorded in `kb_oauth_refresh_replays` is a different matter: **the sweep never
deletes it, at any age.** It filters those rows out, and underneath that the foreign key is
`ON DELETE RESTRICT`, so the database refuses the delete even if some future caller forgets the
filter. Nothing you can see through
[`vw_oauth_refresh_replays`](#watch-for-replayed-refresh-tokens) is aged out from under you.

Deleting the **profile** a replay belongs to does remove that row, because it cascades from
`kb_profiles` — retention and erasure are different acts, and only the first is what this sweep
does.

Every run logs what it deleted per table, including a run that deleted nothing. Server logs are
JSON, so the record looks like this (fields elided):

```json
{"level":"INFO","fields":{"message":"AS retention sweep complete","saml_replay":1204,
 "oauth_flow":880,"refresh_tokens":0,"more_pending":false,"assertion_window_seconds":3600.0}}
```

`more_pending` is `true` when a table stopped because it hit the 50,000-row cap rather than because
it ran out — the next night's run continues from there. `assertion_window_seconds` echoes the floor
the run actually used, which is the quickest way to confirm a change to
`AS_SAML_ASSERTION_MAX_SECONDS` reached the process.

## Further reading

- **The auth-identity contract this mode configures:**
  [Auth identity](../concepts/auth-identity.md).
- **The security model this playbook implements:**
  [The Trust Boundary](../concepts/trust-boundary.md).
- **The internal reconcile channel between the AS and the API:**
  [The SAML reconcile channel](../concepts/saml-reconcile-channel.md).
- **The base deployment this page builds on:**
  [Self-hosting Temper](./self-host-temper.md).
- **The full enterprise install sequence:**
  [enterprise install](./enterprise-install.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/deployment](https://temperkb.io/operating/deployment).
