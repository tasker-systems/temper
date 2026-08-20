# Machine Tokens

**For integrators** — anyone writing code that authenticates as a machine against Temper. Also
relevant to **operators**, who provision and manage machine credentials.

## The boundary: who mints vs. who validates

Temper's token boundary is **issuer-mints / resource-server-validates**, and it stays split:

- **Issuers** — either Auth0 (on the hosted instance) or Temper's own Authorization Server (on
  a self-hosted SAML instance). The issuer mints tokens; it does not validate them.
- **Resource server** — `temper-api` and `temper-mcp` are pure resource servers. They validate
  and normalize tokens. They never mint or advertise grants.

The only thing that needs to be pinned across this boundary is the **token claim shape** —
what the issuer puts on the token and what the resource server reads off it.

## The single machine-token claim shape

A machine token (an agent acting *as itself*, no human behind it) carries a distinct claim
shape that both issuers must produce identically:

| Claim | Value | Note |
|-------|-------|------|
| `azp` / `client_id` | `<clientid>` | the stable agent identity |
| `sub` | `<clientid>@clients` | Auth0's M2M convention |
| `gty` | `client-credentials` | grant-type marker — this is what distinguishes machine from human |
| `email` | *(absent)* | a machine has no verified human email |
| `aud` | the target API/MCP audience | set by the issuer, checked by the validating surface |

Detection keys on `gty == "client-credentials"`, not on `azp` presence — a human access token
also carries `azp`. Classification is total: there is no default arm. An unrecognized token
shape is refused, not silently treated as human.

## The token request shape

The machine-token exchange is a standard OAuth 2.0 `client_credentials` grant:

| | Value |
|---|---|
| `Content-Type` | `application/x-www-form-urlencoded` (RFC 6749 §4) |
| Required params | `grant_type=client_credentials`, `client_id`, `client_secret` |
| Client auth | HTTP Basic (preferred) or the two params in the form body |
| `audience` | Auth0 requires it; Temper's AS ignores it (mints with its own audience) |
| JSON body | `invalid_request` — form-encoded only |

A contract file pins this shape language-neutrally, so every client SDK and the AS's own
integration suite assert against the same wire form.

## Agent principals ride the ordinary rails

Once provisioned, an agent profile is an ordinary accountable principal — no auth-path
special-casing:

- It passes the same authentication and system-access gates as a human.
- It takes **ordinary grants**: team membership for read reach, explicit write grants for
  authoring. Registration is just a convenient way to confer those same ordinary grants at
  mint time, bounded by what the minter could confer on a human.
- There is **no machine-specific authorization path** — machine RBAC falls out of the same
  team-and-grant predicates as human RBAC. The credential *is* the boundary.

A machine must be **registered ahead of its first call**. There is no just-in-time create: an
unregistered or revoked `client_id` is a 401. And registration is not admission — a registered
machine still needs an admin to approve its standing before any data call succeeds. Nothing
before that step fails, which is what makes it easy to omit.

## Further reading

- **The trust boundary machine tokens cross:**
  [The Trust Boundary](./trust-boundary.md).
- **How to provision and use machine credentials (playbook):**
  [Machine credentials](../playbooks/standing-up-a-machine-credential.md).
- **How JWT verification works:**
  [Token verification](./token-verification.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
