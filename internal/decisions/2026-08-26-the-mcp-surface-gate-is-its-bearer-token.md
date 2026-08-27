# The MCP surface's gate is its bearer token

**Date:** 2026-08-26
**Status:** Decided — recorded, not narrowed
**Scope:** `disable_allowed_hosts()` in `temper-mcp`'s `build_router`
**Task:** `01a035f2-d37a-7a83-9f6c-b93d58eb5847`

## Decision

Temper disables rmcp's `Host`-header DNS-rebinding check on the MCP router **unconditionally**, and
relies on the `Authorization: Bearer` JWT gate (`require_mcp_auth`) plus a config-derived CORS policy
as the controls on that surface.

**It is not narrowed to a serverless-only build**, and that is the decision — the draft register row
asked whether it should be, and the answer is no.

## The question that was open, and its answer

The row's open question was: *does any supported topology run `temper-mcp` as a local HTTP listener*,
where rebinding is the actual threat?

**No.** Three independent proofs, the first of which is a compile-time fact:

1. **`temper-mcp` is linked by exactly two packages.** A reverse-dependency scan of `Cargo.lock`
   returns `temper-cloud` (the root package, whose `[[bin]] mcp` is `api/mcp.rs`) and `temper-e2e`
   (test-only). `crates/temper-api/Cargo.toml` and `crates/temper-cli/Cargo.toml` contain **no**
   `temper-mcp` dependency, so the `temper-api` binary and the `temper` CLI **physically cannot**
   serve MCP — the code is not in their link graph.
2. **No local-run harness exists.** `Makefile.toml` has zero MCP tasks. `docker-compose.yml` declares
   one service, `temper-postgres` — no app container. No `examples/` or `benches/` under
   `crates/temper-mcp`.
3. **Every documented topology is Vercel.** `DEPLOYING.md`: *"A deployment target is one Vercel
   project pointed at this repo."* `docs/playbooks/self-host-temper.md`: *"Single-instance
   self-hosting (one Vercel project + one Neon project + one Auth0 tenant) is the supported target
   today."* The SAML, Okta, and enterprise playbooks are auth-variant deltas on that base and
   introduce no alternative host.

**The loopback URLs in the docs are not the MCP server.** `http://127.0.0.1:<port>/callback` is an
OAuth callback receiver *in the client*; the temper-side legs of that hop chain are all
`https://<instance>`. Local clients connect over remote HTTPS —
`docs/playbooks/connect-claude-desktop.md` gives the URL as `https://temperkb.io/mcp` and explicitly
warns off `claude_desktop_config.json`, which is the *stdio* path.

The one local listener that exists is `fn spawn_mcp_server` in
`tests/e2e/tests/auth_seam_parity_e2e.rs`, which binds `127.0.0.1:0`. That is a test harness, not a
topology anyone deploys.

## Why

Two arguments, and they are not equally durable — both belong in the record for that reason.

**The topology argument.** DNS rebinding defends a listener a victim's browser can reach,
canonically `localhost`. Temper ships no such listener, so the residual threat the check would block
is a same-machine browser attacking a listener that does not exist. This argument is true today and
**depends on a fact that could change**.

**The mechanism argument, which survives a topology change.** The credential on this surface is an
`Authorization: Bearer` header. A browser never attaches a bearer header ambiently the way it
attaches cookies — and ambient credential attachment is precisely what the rebinding attack
depends on. So even a hypothetical local listener would not be exploitable by this mechanism.

**Why not the middle option.** rmcp offers `with_allowed_hosts`, and its own field docs recommend it
(*"Public deployments should override this list with their own hostnames"*). `MCP_BASE_URL` is a
required env var carrying the public base URL, so a config-derived allowlist **is** constructible for
production. It is rejected for **Vercel preview deployments specifically**: each preview carries a
per-deployment `Host` that no static list can enumerate, so an allowlist covering production would
reject every preview. That is the exact condition — naming it matters, because a future change could
remove it.

## Two corrections to what the code currently says

The rationale comment at the call site is materially accurate but wrong in two details, both of which
should be fixed when this doc is referenced from it:

- **It says the default allowlist "would 400 every production request." rmcp returns 403.**
  `validate_dns_rebinding_headers` calls `forbidden_response`. The only 400 on that path is
  `bad_request_response("Bad Request: missing Host header")`, which fires on a *missing* Host and
  which `disable_allowed_hosts` does **not** suppress.
- **It says "rmcp 1.4+ added" the feature.** The pin in `crates/temper-mcp/Cargo.toml` is
  `rmcp = { version = "1.8", ... }`.

## A second rmcp guard is also off, and was not in the register row

`allowed_origins` stays at rmcp's default empty vec, which per `validate_origin_header` disables
Origin validation entirely at the rmcp layer.

Temper's own `cors_layer` covers the browser case at the axum layer, but these are **different
mechanisms**: CORS governs what a browser will let a page *read* (and blocks preflighted requests);
rmcp's Origin check would reject server-side regardless of client cooperation. A record of what is
disabled should name both.

## How it is enforced

**It is not.** There is no test and no CI guard on this.

- `rg` over the entire repo finds `allowed_hosts` in exactly one code location — the call site itself.
- No test in `crates/temper-mcp/tests/` sends a `Host` header at all.
- No `.github/` guard mentions it.

The green suite says nothing here. `cors_from_config_test.rs` and `auth_surface_test.rs` cover CORS
and auth, which are different claims.

**What actually holds this decision in place is a Cargo dependency-graph fact** — that
`temper-api` and `temper-cli` do not link `temper-mcp` — **and that fact is unguarded.** Adding the
dependency is a one-line change nothing would flag.

`[provisional — 2026-08-26, judgement call]` If enforcement is wanted rather than a record, the
cheapest mechanism guards the *premise* rather than the call site: a `.github/scripts` guard
asserting `temper-mcp` appears as a dependency in no `Cargo.toml` other than the root and
`tests/e2e`. That is strictly stronger than pinning the `disable_allowed_hosts()` line, because the
line is only safe *because* of the premise. **Not built** — recorded as the identified option.

## Revisiting

Re-review if any of these becomes true:

1. Any `[[bin]]` other than `api/mcp.rs` gains a `temper-mcp` dependency — in particular
   `temper-api` or `temper-cli`.
2. A documented topology appears that binds MCP to a local port or a non-Vercel host: a container
   image, a `cargo make serve-mcp`, an app service in compose.
3. **The MCP surface accepts a credential a browser attaches ambiently** — a cookie or a session. At
   that point the bearer-is-not-ambient argument collapses and the check becomes load-bearing. This
   is the condition that matters most, because it defeats the durable argument rather than the
   contingent one.
4. Any route under `/mcp` moves outside `require_mcp_auth`.
5. Vercel preview deployments stop carrying dynamic per-deployment hosts, which would remove the
   reason `with_allowed_hosts` was rejected.
