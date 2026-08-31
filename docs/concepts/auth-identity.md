# Auth Identity

**For operators** — anyone standing up a Temper deployment and configuring which identity
provider it trusts. Also relevant to **integrators**, who need to know what tokens carry.

## The contract

A Temper instance validates tokens from **exactly one issuer**. It checks an **audience** on
each surface: the HTTP surface validates `AUTH_AUDIENCE`, and the MCP surface validates
`MCP_AUDIENCE` — which defaults to `AUTH_AUDIENCE` when unset — plus, deliberately, `AUTH_AUDIENCE`
itself, because machine tokens and sessions minted before the split carry it. Both audiences name
the same instance; the split exists because conformant MCP clients refuse a `resource` that is
neither the MCP server URL nor its origin, and the only honest way to satisfy that check is for
the MCP surface to have its own resource indicator. The issuer, the audiences, and the JWKS
endpoint the server fetches keys from are the variables that carry whose tokens this instance
trusts and which tokens it accepts. They must agree, and the server refuses to start if they do
not.

## The two modes

The mode is decided by a single signal: whether the Temper Authorization Server issuer is set.

- **External IdP** — the AS issuer is unset. Auth0 or Okta mints tokens; Temper is a pure
  resource server that validates them. This is the shape the hosted instance runs.
- **Temper AS** — the AS issuer is set. Temper's own authorization server mints tokens. This
  is the mode that backs SAML SSO and temper-issued machine credentials.

## The agreement rules

| | External IdP (AS issuer unset) | Temper AS (AS issuer set) |
|---|---|---|
| Auth issuer | The IdP's issuer URL | **Must equal** the AS issuer |
| JWKS URL | The IdP's JWKS endpoint | **Must be** the AS issuer + `/oauth/jwks` |
| Auth audience | The IdP's API identifier — validated by the HTTP surface | **Must equal** the AS audience |
| MCP audience | Optional; the MCP surface's own `resource`. Defaults to the auth audience; must be a URI | Same rule |
| AS issuer | Unset — setting it flips the instance into AS mode | Required — the instance origin |
| AS audience | Unset — never read | Required — **must equal** the auth audience |

In AS mode, the AS audience and the auth audience are **one value spelled two ways** — the AS
mints API-flow tokens with the server-side AS audience and the HTTP surface validates exactly
that. The MCP audience is independent: when set, an MCP authorization flow asks the AS for that
resource and the AS mints its token with it. `MCP_AUDIENCE` unset collapses everything back to a
single audience, which is the shape instances ran before the split.

Under an external IdP, there is no AS, so the AS-specific variables are unset entirely. That is
why the agreement rules are mode-dependent: an operator should not have to hold six independent
knobs in their head when the instance carries two facts.

## The mode decides which endpoints exist

The two modes do not serve the same surface, because some endpoints only mean something in one
of them. An endpoint that belongs to the other mode answers `404` — it is absent, not broken.

| Endpoint | External IdP (AS issuer unset) | Temper AS (AS issuer set) |
|---|---|---|
| `/oauth/authorize`, `/oauth/token` | The loopback redirect proxy, which forwards to the IdP | The Temper AS |
| `/api/auth/mcp-callback` | The proxy's relay, part of that forwarding | **`404`** — the AS runs the whole flow and redirects nothing here |
| `/oauth/jwks` | **`404`** — the IdP publishes its own keys | The AS's public keys |
| `/.well-known/oauth-authorization-server` | Describes the IdP | Describes the AS |
| `/api/auth/cli-callback` | Served — the CLI login relay is mode-independent | Served |

The loopback redirect proxy exists because some IdP tenants reject the ephemeral
`http://127.0.0.1:<port>` callbacks that MCP CLI clients use. It encrypts the client's original
callback into the `state` parameter, which is what `MCP_PROXY_SECRET` keys — so that variable is
required under an external IdP and unused under the Temper AS.

## Boot failure is the control

An incoherent auth config **refuses to start**. Both surfaces parse the identity once, at
startup, through the same code path, and an instance that violates any rule above names the
offending variable and the relation it must satisfy — it never prints a value. The old behavior
was a `warn` line and a served request; a warning in a serverless log is not a control.

This cannot break a working deployment. A divergent audience verifies nothing, a divergent
issuer trusts the wrong party, and a misdirected JWKS URL checks no signature against the keys
that actually signed the token. The boot check names rules that were already true. It can only
refuse to start an instance that was already broken and had not noticed.

## Trailing slashes are normalized

Auth0 issuers conventionally end in `/`; the Temper AS strips them. The comparison normalizes
before checking, so `https://temper.acme.com` and `https://temper.acme.com/` are the same
issuer.

## What this means for integrators

A caller does not configure any of these variables — the operator did, at deployment. What the
caller needs is the **audience** the surface it targets validates, and the way to discover it is
in [The Trust Boundary](./trust-boundary.md): HTTP callers read the authorization-server
metadata; MCP clients read the protected-resource metadata. Each discovery answer is
authoritative for its surface.

## Further reading

- **The trust boundary these variables configure:**
  [The Trust Boundary](./trust-boundary.md).
- **Standing up a deployment (external IdP):**
  [Self-hosting Temper](../playbooks/self-host-temper.md).
- **SAML SSO (Temper AS mode):**
  [Self-hosting with SAML](../playbooks/self-host-with-saml.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/deployment](https://temperkb.io/operating/deployment).
