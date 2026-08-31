# temper-cloud

The Vercel-deployed cloud surface: the REST API gateway, the OAuth layer
(authorization-server metadata, the loopback redirect proxy, the Temper AS mint,
and SAML/Okta flows), and the deploy-time additive-migration apply. The Rust
functions it routes to live in `api/` and compile from the workspace crates;
`vercel.json` is the authoritative routing contract.

The full operator story — choosing audiences, registering APIs at the IdP,
standing up an instance — lives in
[`docs/playbooks/self-host-temper.md`](../../docs/playbooks/self-host-temper.md);
the identity contract behind it is
[`docs/concepts/auth-identity.md`](../../docs/concepts/auth-identity.md).

## OAuth environment variables this package reads

| Variable | Role |
| --- | --- |
| `AUTH_ISSUER` | The issuer whose tokens the instance trusts (Auth0/Okra tenant, or the instance itself in AS mode) |
| `AUTH_AUDIENCE` | The HTTP surface's audience — what REST tokens must carry as `aud`, and what the authorization-server metadata advertises as its `resource` to API callers (CLI, machine clients) |
| `MCP_AUDIENCE` | The MCP surface's own RFC 8707 resource indicator, advertised by the protected-resource metadata. Optional; defaults to `AUTH_AUDIENCE`. Set it to the MCP server URL (e.g. `https://<instance>/mcp`) so conformant MCP clients — which require the advertised `resource` to equal the server URL or its origin — complete the OAuth flow |
| `MCP_BASE_URL` | The instance's public base URL, used in OAuth discovery responses |
| `MCP_CLIENT_ID` | The pre-registered MCP native application's client id, echoed by the DCR proxy |
| `MCP_PROXY_SECRET` | Keys the loopback redirect proxy's state token (external-IdP instances only) |
| `AS_ISSUER` / `AS_AUDIENCE` / `AS_SIGNING_KEY_PKCS8` / `AS_CLIENTS` | Temper-AS mode: the instance mints its own tokens (SAML path). Set `AS_ISSUER` to flip the instance into AS mode |

Discovery doors, per surface: HTTP callers read
`/.well-known/oauth-authorization-server` (`resource` = the API audience); MCP
clients read `/.well-known/oauth-protected-resource` (`resource` = the MCP
audience). Each door states the fact for the audience that discovers through
it; they agree whenever `MCP_AUDIENCE` is unset.
