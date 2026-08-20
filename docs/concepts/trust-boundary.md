# The Trust Boundary

**For integrators** — anyone writing code that calls the Temper API, whether a service, a
script, or an agent runtime. Also relevant to **operators**, who configure the boundary their
deployment enforces.

If you are an individual user authenticating through the CLI, the short version is: install
the binary, run `temper init`, run `temper auth login`, then `temper auth request-access` and
wait for an admin to approve. This page is about what the server enforces, not how to run a
login command.

## What the boundary is

Every call to Temper — HTTP or MCP — crosses the same two-gate boundary, enforced in one place
(the auth seam) and projected onto two surfaces (the HTTP API and the MCP server). A call that
fails either gate is refused before any data is touched.

The two gates, stated as **outcomes** (not levels):

1. **Authentication (401 on failure).** The caller presents a Bearer JWT. The server validates
   its signature against exactly one issuer's keys, checks its audience, checks its expiry, and
   resolves it to a profile. If any of these fail, the response is `401 UNAUTHORIZED`. The
   caller fixes the token and retries.

2. **System access (403 on failure).** The authenticated profile must have **approved standing**
   on this instance. A brand-new signup is born *denied*; `request-access` moves it to
   *requested*; only an admin can move it to *approved*. If the profile is not approved, the
   response is `403 SYSTEM_ACCESS_REQUIRED` with a payload explaining why. There is no path from
   signup to a successful data call that does not pass through an admin.

**Registration is not admission, and nothing fails in between.** A machine can register, mint a
token, pass authentication, and be refused only at the system-access gate — on every data call.
Nothing warns earlier. This is the single most valuable thing to know about the boundary if you
are automating against it.

## The API base URL

The API base is **`<origin>/api`** — the origin plus an `/api` prefix the client prepends per
request. For the hosted instance that is `https://temperkb.io/api`. Verify it live:

```
GET https://temperkb.io/api/health → 200 (unauthenticated)
```

The published API reference — every endpoint, every schema, rendered from the router's
OpenAPI spec — is at **[docs.temperkb.io](https://docs.temperkb.io/)**. Link the site root, not
a deep page; per-endpoint pages have import-generated ids that may not survive a re-import.

## How a caller obtains the audience

The audience is the string the JWT must carry for the server to accept it. **Do not guess it;
discover it.**

```
GET <origin>/.well-known/oauth-authorization-server
```

The response includes a `resource` field carrying exactly the audience the server validates.
For the hosted instance:

```json
{ "issuer": "https://temperkb.us.auth0.com/",
  "resource": "https://temperkb.io/api" }
```

On a self-hosted SAML instance (Temper AS), the response has **no `resource` field** —
correctly, because that AS ignores a request-supplied audience. The caller omits `audience`
and the server does too.

> **The trap.** `/.well-known/oauth-**protected-resource**` (RFC 9728) returns a *different*
> `resource` — the MCP base URL, not the audience. An integrator who reaches for the wrong
> well-known document gets a value that silently fails audience validation. Use
> `oauth-authorization-server` (RFC 8414), not `oauth-protected-resource`.

## The error contract

Every error response carries `{"error":{"code":"…","message":"…","details"?:…}}`. The `code`
is a stable string; `details` rides only `SYSTEM_ACCESS_REQUIRED` and `PLAN_REFUSED`.

| status | code | means | caller does | retry |
|---|---|---|---|---|
| 401 | `UNAUTHORIZED` | missing or malformed Bearer | fix the header | no |
| 401 | `UNAUTHORIZED` | invalid or expired token — signature, issuer, audience, expiry | refresh; re-check audience against discovery | once |
| 401 | `UNAUTHORIZED` | auth service unavailable (JWKS fetch) | back off | yes |
| 401 | `UNAUTHORIZED` | account deactivated | stop; contact operator | no |
| 401 | `UNAUTHORIZED` | machine client not registered — message names the exact provision command | hand the `client_id` to an operator | no |
| 403 | `SYSTEM_ACCESS_REQUIRED` | authentic, no approved standing; `details.refusal.kind` says which | **human:** `request-access`. **machine:** an admin must approve | no |
| 403 | `FORBIDDEN` | refused, deliberately message-less (no capability disclosure) | obtain the grant out of band | no |
| 403 | `FORBIDDEN_DETAIL` | refused, message names the capability | obtain it | no |
| 404 | `NOT_FOUND` | absent or masked (a probe is not an existence oracle) | infer nothing either way | no |
| 400 | `PLAN_REFUSED` | composition invalid; `details.refusals[]` lists every reason | repair all in one round trip | no |
| 422 | `CONTENT_INTEGRITY` | stored bytes fail the hash; not resumable | re-upload from scratch | no |
| 500 | `INTERNAL_ERROR` | server fault | back off | yes |

MCP collapses all of these to JSON-RPC `-32600` with prose — no status, no `code`, no
`details`. An integrator writing a service targets HTTP; MCP is an agent-runtime target.

## The machine-principal sharp edge

**A machine cannot use the remedy its own 403 advertises.** The `SYSTEM_ACCESS_REQUIRED`
payload includes a `cli_command` (`temper auth request-access`), but machines can never
*request* access — the state transition that moves a profile from `denied` to `requested` is
human-only. The only remedy is an admin running `temper admin access approve <profile_id>`, and
**the payload does not name it.** State this in any automation you write, because nothing in
the product will.

And it is invisible until it fails: a registered-but-unapproved machine mints a token (200),
passes JWT validation, passes the registration gate, passes authentication — and is refused
only at the system-access gate, on every data call. Nothing warns earlier.

## Machine principals and headless sessions

A **machine principal** is not a user with a long-lived token; it is its own kind of principal,
registered ahead of time. The authentication path is the same Bearer JWT, minted by the same
issuer, validated by the same two gates. What differs is how the token is obtained — a
machine credential exchange, not a browser login.

A headless CLI session (a cloud agent, a CI pipeline) can authenticate without browser OAuth
by setting environment variables:

| Variable | Purpose | Required? |
|---|---|---|
| `TEMPER_TOKEN` | JWT access token for the API | **Yes** |
| `TEMPER_API_URL` | API base URL (e.g. `https://temperkb.io/api`) | **Yes** — without it the client has no endpoint and fails at send time |
| `TEMPER_PROVIDER` | Auth provider name that issued the token | No — defaults to `auth0` |
| `TEMPER_DEVICE_ID` | Stable device id for this session | No — a fresh UUIDv7 is generated if unset |

A **human-driven agent** (Claude Code, an IDE plugin) authenticates as the human: the human
runs `temper auth login`, the CLI caches the token, and the agent inherits it by driving the
CLI. There is no separate "agent identity" — the agent is the human, through the CLI.

## MCP is a second surface, not a second API

The MCP server runs on a different base path (`/mcp`, no `/api` prefix) with a JSON-RPC
transport, but the authentication is identical — same issuer, same single audience, same two
gates. The difference is transport and tool surface (~26 consolidated tools against 82 REST
paths). An integrator writing a service targets HTTP; MCP is an agent-runtime target. The SDKs
are generated from the OpenAPI spec, i.e. HTTP-only.

## Further reading

- **What a machine credential is and how to stand one up:**
  [Machine credentials](../playbooks/standing-up-a-machine-credential.md) (playbook).
- **The auth-identity contract operators configure:**
  [Auth identity](./auth-identity.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
