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
discover it** — and discover it from the door that matches the surface you are calling.

**HTTP callers** (CLI, machine clients, SDK users):

```
GET <origin>/.well-known/oauth-authorization-server
```

The response includes a `resource` field carrying exactly the audience the HTTP surface
validates. For the hosted instance:

```json
{ "issuer": "https://temperkb.io/",
  "resource": "https://temperkb.io/api" }
```

On a self-hosted SAML instance (Temper AS), the response has **no `resource` field**. That
authorization server serves a fixed set of resources — the instance's API audience and, when the
operator configured one, its MCP resource — and a caller may name one of them with the RFC 8707
`resource` parameter (or Auth0's `audience` spelling) at `/oauth/authorize`. A requested resource
outside that set is refused; a request that names none gets the API audience. HTTP callers can
simply omit it.

**MCP clients** read the RFC 9728 protected-resource metadata instead:

```
GET <origin>/.well-known/oauth-protected-resource
```

Its `resource` is the MCP surface's own audience — `MCP_AUDIENCE` when the operator set it
(conventionally the MCP server URL itself, which is what lets conformant MCP clients pass their
`resource`-matches-the-server-URL-or-origin check), `AUTH_AUDIENCE` otherwise. A conformant MCP
client requests that resource, and the token it receives names it.

> **Each discovery door answers for its surface; both doors accept the same tokens.** The HTTP
> and MCP doors accept the same instance audience set. When `MCP_AUDIENCE` differs from
> `AUTH_AUDIENCE`, a token naming either resource authenticates at either door. This is
> deliberate: the MCP service relays the caller through the API, and the two transports are one
> authorization domain. The audience split satisfies resource discovery and token targeting; it
> is not a least-privilege boundary between the transports. A token naming any other audience is
> refused at both.

### Who signs what in Temper-AS mode

On a SAML instance two different signatures are in play, and they are not interchangeable:

- **The external SAML identity provider** authenticates the user and signs the SAML
  *assertion* it posts back to Temper. Temper's authorization server verifies that signature
  against the IdP certificate the operator configured.
- **Temper's authorization server** signs the OAuth *access token* (EdDSA). That token is the
  only thing either door ever sees, and it is verified against the authorization server's own
  published keys — the instance's one issuer.

So the issuer a caller's token names is always the Temper instance, never the SAML IdP. The
`resource` the flow requested is carried through the authorization code and every refresh of
the session, so a refreshed token keeps the audience its login was granted.

## What an unauthenticated caller can learn

The rule: an unauthenticated caller learns nothing except from the designated public routes.

The public routes are:

- `GET /api/health`, which publishes the deployed crate version and commit;
- the Slack link callback, `/api/auth/slack/callback`, a browser's redirect target from the identity
  provider. It answers a link it cannot honour (unknown, expired, replayed, a `state` that is not a nonce, or on an
  instance with no Slack linking configured) with one page, so its page does not say how the
  instance is configured (its timing can; see the residues);
- the discovery and OAuth doors the Node and MCP functions serve: `/.well-known/*`, `/oauth/*`,
  `/mcp` and `/mcp/*`. Their answers are their own, not this page's refusal. The rest of that
  public, auth-establishing surface, stated so nobody has to rediscover it:
  - **The `/api/` aliases.** `vercel.json` serves the function files before its rewrites, so each
    function is also reachable at its own path under `/api/`. The OAuth functions answer there as
    they do at their doors (`/api/oauth/*`). `/api/auth/cli-callback` and `/api/auth/mcp-callback`
    have no other path: they are the redirect URIs registered with the identity provider. The
    API and MCP functions' own paths (`/api/axum`, `/api/internal`, `/api/mcp`) are unmatched in
    their routers and answer the refusal.
  - **`/build-marker.txt`**, the static file the build writes because Vercel's build refuses an
    empty output directory (`scripts/vercel-build.sh`). It says the project has no static output.
  - **The MCP function's unmatched paths.** Under `/.well-known/` and `/oauth/` it answers its
    structured `404`: RFC 8615 makes that the answer for a name a server does not publish, and MCP
    clients probe `/.well-known/oauth-protected-resource/mcp` and the OpenID configuration and
    fall back on it. Anywhere else, such as `/api/mcp`, it answers the refusal, the same bytes as
    the API's, whoever asks. `/mcp` and everything under it but `/mcp/health` keep the gate's
    `401` with `WWW-Authenticate`, which is how an MCP client finds the authorization server.
  - **The MCP function's headers.** It has no service outside routing, so its answers carry no
    `x-request-id` (and, with an allowlist configured, expose only `link`, where the API also
    exposes `x-request-id`). Its CORS layer answers every `OPTIONS` itself, `200` on every path
    alike (with `Allow` only on its public routes) and with none of the security headers, where
    the API routes a non-preflight `OPTIONS` to the refusal and gives its preflight the security
    headers. While its key store is failing, `/mcp` answers a bearer `503` and its other answers
    do not change, where the API answers every refusal "unavailable". Like the API, it answers an
    undecodable `Content-Encoding` the `415` before routing (an `OPTIONS` excepted), and on `/mcp`
    a body declaring a `Content-Length` over the door's limit is the `413` before the gate. These
    sort the MCP function's paths from the API's; `vercel.json` already says which paths those
    are.
  - **Case.** The platform matches `vercel.json` routes without regard to case: a preview probe
    showed `/MCP` and `/OAUTH/register` reaching the MCP function. So a case variant of a door
    reaches that door's function. Every router here matches bytes, so the variant gets what its
    byte-exact spelling would get there: a public door's own answer, the family's `404` (for
    example `/.well-known/OAUTH-PROTECTED-RESOURCE`), the `/mcp` gate's challenge (`/mcp/HEALTH`),
    or the refusal.
  - **The web UI on the canonical origin.** On `temperkb.io` the web UI is the origin's front
    door. Its server proxy (`packages/temper-ui/src/lib/server/proxy.ts`) forwards `/api`, `/mcp`,
    `/oauth` and `/.well-known`, and everything under them, to the deployment above. It serves
    every other path itself: its own pages, and its own `404` page for a path it does not have
    (`/no-such-path`). It matches those prefixes by exact case, so a case variant of a prefix
    (`/MCP`, `/OAUTH/register`, `/API/x`) is the UI's `404` page there, never the API's. The UI is
    public by design, and its `404` is the same for every path it does not serve, so it says
    nothing about which API routes exist. A path under a lower-case `/api/` reaches the API
    whatever the case of the rest, and gets the refusal (`/api/NO-SUCH`);
- the API reference under `/api-docs/`, only when an operator enables it, so whether it answers
  says whether it is enabled.

Every other request is answered the same way, on any path, matched or not, by any method, with any
credential that does not verify. (A `Content-Encoding` the server cannot decode is answered the 415
in the table below, before any authentication, and that too is the same on every path.) A refusal
before identity is established is the same bytes whatever the door (bearer, signature, cron secret,
broker attestation) and whatever was wrong with the credential: the status, the body, the security
headers, the CORS headers for the caller's `Origin`, and an `x-request-id` minted for the request.
`Allow` rides only a `405`, so it never names a path's methods on a refusal. The operator's log line
for it, `auth error`, carries that request id and the real cause, so a caller who quotes the id can
be helped without the answer saying anything.

A CORS preflight from an allowlisted origin is answered with the same bytes on every path, before
routing. Any other `OPTIONS` (no `Origin`, a foreign one, or no `Access-Control-Request-Method`) is
an ordinary request, and an unauthenticated one gets the refusal.

An unmatched path is answered the same way. Without a validly signed bearer it is that refusal. With
one, it is the 404 `No route matches …`, whether or not the token's identity has a profile, unless a
gated route would refuse the same token for itself (a deactivated account, an unregistered or
revoked machine, an email that cannot be resolved), which keeps that answer. Resolving the caller
never creates a profile; like sign-in, it may refresh an auth link, attach a verified email's link
to the profile that holds it, or touch a registered machine's last-seen time. Under `/internal/`
(however many leading slashes, or the bare `/internal`), where no bearer route lives, every
unmatched path is the refusal whoever asks.

"Unavailable" is server-wide state, not a per-path answer. Each instance keeps one bearer key store.
When its keys are stale, every refusal it gives joins one shared refresh of that store, whatever its
door, and waits for it from the request's arrival (for a signed request, from when its body is in)
until a little past the fetch's own five-second timeout; a bearer lookup joins the same refresh,
with its wait counted from the lookup. So an instance whose routes never verify a bearer still
learns of an outage, a failing store recovers without bearer traffic, and a refresh normally ends
inside a request that is waiting on it, on a platform that suspends idle instances (one started late
in a slow request can outlive it, and the next request that needs it finishes it). A request that
arrived while the store was failing, or whose wait ended with no usable keys, is answered
"unavailable" whatever its door.

The residues, stated so nobody has to rediscover them:

- **Whether an origin is allowlisted.** Anyone can send a preflight, or any request, with an
  `Origin` and see whether the answer carries `Access-Control-Allow-Origin`. That is what CORS is.
- **The CORS mode.** The CORS headers depend on the instance's configuration, never on the path or
  the request: with no origins configured every answer carries `vary: origin`; with an allowlist,
  `vary: origin, access-control-request-headers` and `access-control-expose-headers:
  link,x-request-id`; in permissive mode (`*`), `access-control-allow-origin: *` and
  `access-control-expose-headers: *`, with no `vary`. These are tower-http's bytes, so a caller can
  tell the three configurations apart and nothing more. (The MCP function's allowlist exposes only
  `link`; see the MCP function's headers above.)
- **The cost of a signed caller's mistyped path.** A validly signed token on an unmatched path
  costs one token verification and one profile lookup, plus the email lookup (and possibly one call
  to the identity provider's `/userinfo`) when the token carries no email claim and no link is
  cached. Reaching any of it takes a validly signed token, so it is no lever for an unauthenticated
  caller.
- **The Slack callback's own work.** On an instance with Slack linking configured, a callback
  carrying a well-formed state looks it up in the database, and one without linking does not. So
  the two can be told apart by timing, and by the "Something went wrong" page a configured instance
  gives when its database cannot answer. Each such request costs the configured instance one
  database round trip; the route has no rate limit of its own.
- **Which function serves a path.** On a split deployment the cron doors and `/internal/*` are
  served by one function and everything else by another (`vercel.json`, which is public); each can
  also be reached at its own path, `/api/internal` and `/api/axum`. The two start cold separately
  and keep separate key stores, so first-request latency, or an outage one function has noticed
  and the other has not, sorts paths by function. That is the routing the public repository
  already states. To an unauthenticated caller both answer the same refusal; a validly signed
  caller gets a 404 on an unmatched path from one and the refusal from the other.
- **An authenticated caller can map routes.** A signed caller tells an unmatched path (404) from a
  route it is refused on, and a route's methods from its `405`. If the platform's routing matches
  paths without regard to case, a path that differs from a door only in case may also reach a
  different function and answer a signed caller differently. None of it is visible without a
  validly signed token.
- **The moment of the change.** "Unavailable" holds until a refresh succeeds. The state moves when a
  refresh completes, and a refresh is started by a request. After the endpoint recovers, an instance
  answers "unavailable" until its next refusal or bearer lookup past the five-second failed-refresh
  backoff refetches, and that request too. Public routes, and requests that succeed without a bearer
  lookup, do not refresh the store.
- **A slow endpoint is slow for everyone.** While a refresh is in flight, every refusal on that
  instance waits for it, up to the fetch's timeout. That is the same on every door, but it is
  visible: a refusal that takes seconds says a refresh is running, never which path you hit. A
  refusal may also close its connection when its request body was large and unread, whether or
  not it waited; that framing is the same on every door.
- **The cost of a hanging endpoint.** While the identity provider's key endpoint accepts
  connections and never answers, every refusal holds its connection, and on a serverless platform
  its billed function time, for about five seconds, then answers at once through the five-second
  backoff that follows, and so on. At most 32 requests per instance wait at once; the rest answer
  at once with what the store knows, so a flood during an outage costs at most that many held
  connections for the wait. An unauthenticated caller cannot cause the outage.
- **Each instance on its own.** Instances, including the separate functions of a split deployment,
  do not share the store. During an outage one may report it while another has not yet tried.
- **The webhook intake's own key store.** The intake verifies a Vercel Connect attestation against
  Vercel's key endpoint, in a store of its own that the refusal path does not wait on. If that
  endpoint hangs while the identity provider's is healthy, or on the intake's first request on a
  cold instance, an attested-looking request to the intake waits for that fetch where other doors
  answer at once. The intake's refusal body is the constant one either way.
- **An unpublished `kid`.** A bearer naming a key the cache does not hold refetches the JWKS at
  most once a minute per instance, and that request waits for the fetch. Any caller can do this
  with a healthy identity provider, so a bearer door can answer one request a minute more slowly
  than a door that verifies no bearer. The refetch is what lets a newly rotated signing key verify
  without waiting out the cache.
- **A signed request's body is read before routing, on every path.** A request carrying all three
  `X-Temper-*` signature headers has its body read, up to 64 KiB, before the server decides which
  door it reached, so a declared body that never arrives waits on every path alike, and
  `Expect: 100-continue` is answered `100 Continue` on every path alike. Over 64 KiB, unreadable,
  or not all in within ten seconds of the headers, it is the refusal on every path, the public
  routes and the routes that take larger bodies included, so a request carrying those headers
  cannot send more than 64 KiB anywhere. A declared length over the cap is refused without
  reading the body, so without `100 Continue`. The 64 KiB counts the bytes on the wire; a
  signature door then checks the decompressed body against the same limit. A platform or proxy in
  front that buffers bodies or answers `Expect` itself is outside this: it changes what the caller
  sees, the same way on every path.
- **Timing between doors.** Outside the refresh, the work before a refusal differs by door (a
  signature is checked against its body, a bearer against a key), and no attempt is made to
  equalise that.
- **The MCP surface** answers its own outage as `503`, not `401`. It is a different function,
  reached at `/mcp`, and already told apart by its discovery documents.

## The error contract

Every error response with a body carries `{"error":{"code":"…","message":"…","details"?:…}}`
(the 405 row and the undecodable-encoding 415 row below have none). The `code`
is a stable string; `details` rides only `SYSTEM_ACCESS_REQUIRED` and `PLAN_REFUSED`.

| status | code | means | caller does | retry |
|---|---|---|---|---|
| 401 | `UNAUTHORIZED` | `Unauthorized: authentication required`: no verified credential. It was missing, malformed, invalid or expired (signature, issuer, audience, expiry), a signature or secret was refused, or the path matches no route; on every path alike, and the answer does not say which | refresh the token, re-check its audience against discovery, and fix the header if it is not a Bearer | once |
| 401 | `UNAUTHORIZED` | `Unauthorized: Authentication service unavailable`: the instance cannot currently verify credentials (its JWKS fetch is failing); server-wide: while it holds, every refusal before identity says this, whatever the path | back off | yes |
| 401 | `UNAUTHORIZED` | account deactivated; a validly signed token is required to see this | stop; contact operator | no |
| 401 | `UNAUTHORIZED` | machine client not registered — message names the exact provision command; a validly signed token is required to see this | hand the `client_id` to an operator | no |
| 403 | `SYSTEM_ACCESS_REQUIRED` | authentic, no approved standing; `details.refusal.kind` says which | **human:** `request-access`. **machine:** an admin must approve | no |
| 403 | `FORBIDDEN` | refused, deliberately message-less (no capability disclosure) | obtain the grant out of band | no |
| 403 | `FORBIDDEN_DETAIL` | refused, message names the capability | obtain it | no |
| 404 | `NOT_FOUND` | absent or masked (a probe is not an existence oracle). Answered only to an authenticated caller, on an unmatched path as anywhere; an unauthenticated caller gets the first 401 row | infer nothing either way | no |
| 405 | (none) | the path does not mount that method; empty body, and `Allow` names the methods it does mount. Answered only to a caller the route admits, or on a public route; `Allow` rides only a 405 | use a method `Allow` names | no |
| 415 | (none) | a `Content-Encoding` the server cannot decode; empty body, `accept-encoding: gzip`. Answered before any authentication, identically on every path, mounted or not, and never with `Allow` | send the body uncompressed or as gzip | no |
| 400 | `PRECONDITION_NOT_HONOURED` | a write carried `If-Match`, `If-None-Match` or `If-Unmodified-Since` on a route that does not evaluate it; nothing was applied. Answered only to a caller the route already admits: an unauthenticated or refused caller gets the answer it gets without the header | resend without the header to write unconditionally | no |
| 400 | `PLAN_REFUSED` | composition invalid; `details.refusals[]` lists every reason | repair all in one round trip | no |
| 400, 413, 415, 422 | `UNREADABLE_PLAN` | the body is not a readable composition (malformed JSON, wrong content type, over the door's limit, a wrong type); no `details`, and the message repeats at most 1024 bytes of what failed | fix the body; there is no plan to refuse yet | no |
| 409 | `INGEST_ENDED` | append or finalize on an ingest that has ended (`cancelled` or `abandoned`); not resumable | start a new upload | no |
| 422 | `CONTENT_INTEGRITY` | stored bytes fail the hash; not resumable | re-upload from scratch | no |
| 500 | `INTERNAL_ERROR` | server fault | back off | yes |

MCP renders these as JSON-RPC errors with prose — no status, no `code`, no `details`: a
caller fault the agent can repair (a refused or unreadable plan, a refused search) as `-32602`,
the terminal auth refusals as `-32600`, everything else as `-32603`. An integrator writing a service targets HTTP; MCP is an agent-runtime target.

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

Some hosts keep the credential out of the session entirely: an egress proxy attaches it to
requests bound for the API host, so no token is ever in the environment. For those, set
`TEMPER_AUTH=ambient` instead of `TEMPER_TOKEN`. The CLI then sends requests without an
`Authorization` header for the proxy to add, never reads or writes `auth.json`, and learns who
it is from `GET /api/profile`.
The validation is unchanged: whatever the proxy attaches passes the same two gates. Setting
both `TEMPER_AUTH=ambient` and `TEMPER_TOKEN` is refused as a configuration error, and so is
any other `TEMPER_AUTH` value.

Such a session needs no `temper init` either. Set `TEMPER_VAULT` to a directory for the local
projection, and commands that resolve a vault read the absent `~/.config/temper/config.toml` as
defaults. Without a named vault, the config file is still required.

A **human-driven agent** (Claude Code, an IDE plugin) authenticates as the human: the human
runs `temper auth login`, the CLI caches the token, and the agent inherits it by driving the
CLI. There is no separate "agent identity" — the agent is the human, through the CLI.

## MCP is a second surface, not a second API

The MCP server runs on a different base path (`/mcp`, no `/api` prefix) with a JSON-RPC
transport, but the authentication is identical — same issuer, the same accepted audience set
(the API audience and, when configured, the MCP surface's own `MCP_AUDIENCE`), same two
gates. The difference is transport and tool surface (40 registered MCP tools against 105 REST
paths — the REST count from the router's generated `openapi.json`, the tool count from the
MCP server's registered tool list). An integrator writing a service targets HTTP; MCP is an
agent-runtime target. The SDKs are generated from the OpenAPI spec, i.e. HTTP-only.

## Further reading

- **What changed for an unauthenticated caller, release by release:** the [release
  register](https://github.com/tasker-systems/temper/blob/main/release-register/README.md); this
  page's rule was built by the entries `anonymous-options-and-allow`,
  `uniform-pre-identity-refusal`, `cron-and-intake-tier-gates`, `signed-body-read-before-routing`,
  `unmatched-path-uniform-to-anonymous` and `slack-callback-unconfigured-page`.

- **What a machine credential is and how to stand one up:**
  [Machine credentials](../playbooks/standing-up-a-machine-credential.md) (playbook).
- **The auth-identity contract operators configure:**
  [Auth identity](./auth-identity.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
