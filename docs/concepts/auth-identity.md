# Auth Identity

**For operators** — anyone standing up a Temper deployment and configuring which identity
provider it trusts. Also relevant to **integrators**, who need to know what tokens carry.

## The contract

A Temper instance validates tokens from **exactly one issuer**. It checks **one audience** on
both surfaces (HTTP and MCP). The issuer, the audience, and the JWKS endpoint the server fetches
keys from are six variables that carry two facts — whose tokens this instance trusts, and which
tokens it accepts. They must agree, and the server refuses to start if they do not.

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
| Auth audience | The IdP's API identifier — the one audience, validated on both surfaces | **Must equal** the AS audience |
| MCP audience | Optional; if set, must equal the auth audience | Same rule |
| AS issuer | Unset — setting it flips the instance into AS mode | Required — the instance origin |
| AS audience | Unset — never read | Required — **must equal** the auth audience |

In AS mode, the three audiences are **one value spelled three ways** — the AS mints every
token with the server-side AS audience, the auth audience is what both surfaces validate, and
the MCP audience (if set) merely restates it. They are the same string or the instance verifies
nothing.

Under an external IdP, there is no AS, so the AS-specific variables are unset entirely. That is
why the agreement rules are mode-dependent: an operator should not have to hold six independent
knobs in their head when the instance carries two facts.

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
caller needs is the **audience** the server validates, and the way to discover it is in
[The Trust Boundary](./trust-boundary.md). The auth-identity contract is why the discovery
answer is authoritative: the server validates exactly one audience, and it is the one the
operator set.

## Further reading

- **The trust boundary these variables configure:**
  [The Trust Boundary](./trust-boundary.md).
- **Standing up a deployment (external IdP):**
  [Self-hosting Temper](../playbooks/self-host-temper.md).
- **SAML SSO (Temper AS mode):**
  [Self-hosting with SAML](../playbooks/self-host-with-saml.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/deployment](https://temperkb.io/operating/deployment).
