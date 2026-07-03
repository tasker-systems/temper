# Self-Hosting Temper with a SAML IdP

This guide covers **native SAML** single sign-on for a self-hosted Temper instance. Unlike
[self-hosting with Okta](./self-hosting-okta.md) (which uses Okta's OIDC endpoints directly), this
option fronts your SAML Identity Provider (IdP) with a **minimal OAuth 2.0 Authorization Server (AS)
built into Temper**. Your SAML IdP authenticates the user; the Temper AS mints a short-lived
EdDSA-signed Temper JWT that `temper-api` trusts.

Use this when your organization's IdP speaks SAML 2.0 (e.g. Okta, Entra ID, PingFederate, Shibboleth)
and you want a native SP integration rather than an OIDC bridge.

> **Doing a full ground-up enterprise install?** This guide is one phase. For the single
> end-to-end sequence (deploy → SAML → org → agents) see [enterprise-install.md](./enterprise-install.md).

> **This guide is the operator runbook.** For the *security model* it implements — how
> tokens are verified, the two-level authorization seam, the reconcile channel's trust
> model, and profile deactivation as an authn lever — see [../auth/](../auth/README.md),
> the canonical home for Temper's auth flows.

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
5. The client exchanges the code at `/oauth/token` for an EdDSA-signed access token. `temper-api`
   validates it against the AS's published JWKS and just-in-time provisions the profile.

The AS is **SP-initiated only**, supports a **single active IdP** per instance, and maps a **persistent
NameID** (or a configured stable-id attribute) to the token `sub`. A validly signed assertion implies
`email_verified: true`.

## Quickstart with `temper admin saml` (recommended)

Rather than hand-assembling the keys, environment, and SQL documented in the sections below,
an operator working from a repo checkout can generate them with the **`temper admin saml`**
command group. It is an *emitter*: it prints the exact env bundle and SQL (or writes them with
`--env-out` / runs them with `--apply`), keeping the AS↔API shared values (`AS_AUDIENCE` ==
`AUTH_AUDIENCE`, `AUTH_ISSUER` == `AS_ISSUER`, the one `INTERNAL_RECONCILE_SECRET`,
`AUTH_PROVIDER_NAME` == `saml:<idp-key>`) consistent by construction. The numbered sections that
follow remain the authoritative reference and the manual fallback.

1. **Provision keys + env + the IdP row** — before anyone can log in:

   ```bash
   temper admin saml provision \
     --instance-url https://<instance> --idp-key <acme-okta> \
     --idp-cert-file idp.pem --idp-sso-url https://idp.example.com/sso --idp-entity-id http://idp \
     --client temper-cli=https://<instance>/api/auth/cli-callback \
     --client temper-ui=https://<app-url>/auth/callback
   ```

   Omit flags for interactive prompts, or add `--no-interactive` for a scripted run. It generates
   the Ed25519 signing key (`AS_SIGNING_KEY_PKCS8`), `AS_SIGNING_KID`, and a strong
   `INTERNAL_RECONCILE_SECRET`, then emits the full env bundle and the `kb_saml_idp` INSERT to
   stdout. `--env-out .env.saml` writes the env (mode 0600 — it holds the private key); `--apply`
   runs the SQL against `$DATABASE_URL`. Paste the env into **both** Vercel functions and deploy.

2. **Map IdP groups to teams** — after the teams exist (see the org-bootstrap runbook):

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

> **Ordering.** SAML setup *brackets* the [org-bootstrap runbook](./org-bootstrap.md): run
> `provision` + deploy + apply the IdP row **before** the first admin can log in, then run the
> org-bootstrap (which creates the teams), then run `map-group` **after** those teams exist. See
> that runbook's interleave note.

## 1. Register Temper as an SP with your IdP

In your IdP, create a new SAML application ("SP") with:

| Setting | Value |
| --- | --- |
| ACS (Assertion Consumer Service) URL | `https://<instance>/oauth/saml/acs` |
| SP Entity ID / Audience | a stable URI you choose, e.g. `https://<instance>/saml/metadata` |
| NameID format | **persistent** (recommended) — becomes the token `sub` |
| Sign assertions | **yes** (both the `<Response>` and the `<Assertion>` must be signed) |

Add two attribute statements to the assertion:

- an **email** attribute (e.g. `email`) — becomes the token `email`.
- a **stable identifier** attribute (e.g. `uid`) — the fallback for `sub` when the NameID is not persistent.

Temper publishes its SP metadata at `https://<instance>/oauth/saml/metadata` for import into IdPs
that accept SP metadata XML.

## 2. Configure the active IdP (`kb_saml_idp`)

The IdP configuration lives in the database, not env. Insert exactly **one** active row (flip
`is_active` to rotate to a replacement):

```sql
INSERT INTO kb_saml_idp (
  idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id,
  sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr
) VALUES (
  'acme-okta',                                    -- idp_key: your label for this IdP
  true,
  '-----BEGIN CERTIFICATE-----\n...\n-----END CERTIFICATE-----',  -- idp_cert: the IdP's signing cert (PEM)
  'https://idp.acme.com/app/xxx/sso/saml',        -- idp_sso_url: the IdP SSO (redirect) endpoint
  'http://www.okta.com/xxx',                       -- idp_entity_id: the IdP's entity id
  'https://<instance>/saml/metadata',              -- sp_entity_id: MUST match the Audience you set above
  'https://<instance>/oauth/saml/acs',             -- acs_url
  'urn:oasis:names:tc:SAML:2.0:nameid-format:persistent',
  'email',                                          -- email_attr: the assertion attribute for email
  'uid'                                             -- stable_id_attr: fallback sub source
);
```

## 3. Map IdP groups to Temper teams/roles (Phase 2)

Temper reconciles team membership from SAML-asserted groups **on each login**. This is
eventual, not immediate: a user removed from a group keeps access until their session expires
and they next log in. For immediate deprovisioning use SCIM (not yet available).

**Reconcile only ever manages `source='idp'` memberships. Native memberships (added in-app or by
join-request approval) and auto-join teams are never touched — if a user is already a native
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
   team. Note: the **first** admin still requires the SQL bootstrap step; SAML does not bootstrap
   the system.

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

## 4. Environment variables

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
| `AS_REFRESH_TTL_SECONDS` | `2592000` (default, 30d) | Refresh-token lifetime. |

`AS_CLIENTS` registers the exact redirect URIs each client may use (exact string match — this is the
control that prevents authorization-code exfiltration):

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

### Group provisioning (Phase 2)

These gate the internal reconcile call the AS makes to `temper-api` before minting a token. Set
`INTERNAL_RECONCILE_SECRET` to the **same** value on both the AS and the `temper-api` deployment
(they share a Vercel project env). If unset, the reconcile endpoint is disabled and no group
provisioning occurs (authentication still works).

> Why a shared secret rather than an origin/IP allow-list, and the endpoint's bounded blast
> radius, are explained in [../auth/reconcile-channel.md](../auth/reconcile-channel.md).

| Variable | Where | Purpose |
| --- | --- | --- |
| `INTERNAL_RECONCILE_SECRET` | AS + API (shared) | Shared secret gating the internal reconcile call. Same value on both. Unset ⇒ reconcile disabled, no group provisioning. |
| `INTERNAL_RECONCILE_URL` | AS | Full URL of the `temper-api` `/internal/saml/reconcile` endpoint the AS calls before minting (e.g. `https://<your-api-origin>/internal/saml/reconcile`). |

## 5. Configure the CLI

Run the guided setup and pick the **Temper AS (native SAML)** provider:

```bash
temper init
```

Or non-interactively:

```bash
temper init --no-interactive --instance-url https://<instance> --idp temper-as
```

This writes an `[[auth.providers]]` block with `provider = "temper-as"`, `client_id = "temper-cli"`,
`authorize_url = https://<instance>/oauth/authorize`, `token_url = https://<instance>/oauth/token`,
and `callback_url = https://<instance>/api/auth/cli-callback`. The existing PKCE + loopback login flow
is issuer-agnostic — no other CLI change is needed. `temper login` then authenticates through SAML.

## 6. Configure the UI (optional)

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
`/.well-known/openid-configuration`). Ensure `temper-ui`'s `<app-url>/auth/callback` is listed in
`AS_CLIENTS`. `OIDC_PUBLIC_CLIENT=true` is required for this secret-less path — without it, the UI
fails fast at startup rather than silently running with no client secret.

## 7. Verify

1. `temper login` → a browser opens to your IdP; after SAML login the CLI receives a token.
2. `temper whoami` (or any authenticated command) succeeds.
3. A `kb_profiles` row and a `kb_profile_auth_links` row (with `auth_provider = saml:<idp-key>`) are
   created for the user on first login.

## 8. Deactivating an account (authn control)

Team membership is **authorization**; it does not control whether an account can log in. To stop
an account from authenticating at all — regardless of what the IdP asserts — soft-delete the
profile:

```sql
UPDATE kb_profiles SET is_active = false WHERE id = '<profile-uuid>';
```

A deactivated profile is rejected by the API auth middleware (`401`) even with a valid token.
This never deletes the profile or its history, and it is independent of SAML group provisioning
(re-activating restores access). Reconcile/deprovisioning of a team never deactivates a profile.

> `is_active` is enforced by the shared authorization seam (Level 1), so **both** surfaces —
> `temper-api` and `temper-mcp` — reject a deactivated profile identically. See
> [../auth/authorization-seam.md](../auth/authorization-seam.md).

## Running it as the applier

[`saml-setup.sh`](../../scripts/bootstrap/saml-setup.sh) automates the `temper admin saml`
sequence above from a declarative profile — it loops `provision` / `map-group` / `verify` from
[`saml-profile.yaml`](../../schema-artifact/saml-profile.yaml):

```bash
# Dry-run first — prints the commands without executing:
scripts/bootstrap/saml-setup.sh --profile schema-artifact/saml-profile.yaml --dry-run

# Emit only (default) — env bundle + kb_saml_idp SQL, safe to run anytime, no DB writes:
scripts/bootstrap/saml-setup.sh --profile schema-artifact/saml-profile.yaml

# Apply — writes the kb_saml_idp row, applies group mappings, verifies against the live DB
# (needs DATABASE_URL + psql; run post-migrate, and after the org-bootstrap teams exist):
DATABASE_URL=postgresql://… scripts/bootstrap/saml-setup.sh \
  --profile schema-artifact/saml-profile.yaml --apply-db
```

It needs `yq` to read the profile and `temper` on PATH. Emit-by-default and idempotency are
inherited from the underlying `temper admin saml` commands, not reimplemented by the script. It
is the SAML sibling of
[`system-bootstrap.sh`](../../scripts/bootstrap/system-bootstrap.sh)
(kept separate so that script stays usable for Auth0/Okta-OAuth installs) — see
[enterprise-install.md](./enterprise-install.md#the-timeline) for how the two appliers interleave
across the full install timeline.

## Limitations (Phase 1)

- **Reconcile-on-login only.** Profile attributes refresh when the user logs in; there is no live
  deprovisioning. A user removed at the IdP retains access until their token expires (bounded by
  `AS_ACCESS_TTL_SECONDS`) and cannot re-login. Automated deprovisioning (SCIM) is **Phase 3**.
- **Single active IdP** per instance, **SP-initiated** flows only.
- **Single issuer** per instance: an instance is either an AS/SAML instance (`AS_ISSUER` set) or an
  Auth0/OIDC instance, not both.
- Role/team mapping from SAML attributes is **Phase 2**.
