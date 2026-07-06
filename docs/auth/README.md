# Temper auth & security

Canonical home for how Temper authenticates callers and authorizes what they can do.
If you are changing anything on an auth path — a new gate, a new token shape, a new
issuer, the reconcile channel — the explanation lives here, and this is where the
**"did I touch both surfaces?"** discipline is written down.

## The one thing to remember

Authorization is enforced **on two surfaces** — `temper-api` (HTTP middleware) and
`temper-mcp` (per-tool) — but the **gate sequence lives once**, in the
`temper-services::auth` seam. A gate added to the seam is enforced on both surfaces; a
gate hand-added to one surface's middleware silently misses the other.

> This is not hypothetical. SAML Phase 2 added the `is_active` deactivation gate to
> `temper-api` only; a deactivated account's valid token kept full MCP tool access until
> the 2026-07-02 review caught it. The seam exists so the next gate cannot repeat that.

## The surfaces and the seam

```text
  temper-api (HTTP)                          temper-mcp (rmcp tool call)
  ───────────────────                        ────────────────────────────
  require_auth (middleware/auth.rs)          require_mcp_auth (middleware.rs)
    · verify JWT (JwksKeyStore)                · verify JWT (same JwksKeyStore)
    · resolve email ladder                     · aud = mcp_audience
    · aud = auth_audience                      ↓ inject RawJwtClaims
    ↓ normalize_machine → AuthClaims          ensure_profile_from_parts (service.rs)
        │                                          │
        └──────────────┐          ┌────────────────┘
                       ▼          ▼
             temper-services::auth  (the seam — sequence lives here once)
               authenticate(pool, &claims)          → AuthenticatedProfile   [Level 1]
               require_system_access(pool, &authed) → SystemAuthorized       [Level 2]
                       │          │
        ┌──────────────┘          └────────────────┐
        ▼                                            ▼
  map AuthzError → HTTP status                map AuthzError → rmcp ErrorData
  (401 / SystemAccessRequired)                (INVALID_REQUEST, terminal)
```

Each surface owns **only** two things the seam does not: (1) JWT signature verification +
claim normalization (audience differs legitimately per surface), and (2) mapping
`AuthzError` to its transport's words-on-the-wire. Everything between — resolve the
profile, gate on `is_active`, gate on `system_access` — is the seam.

## The two-level chain

A future gate belongs to exactly **one** of these levels. "Add a gate = edit one
function" holds *per level*.

| Level | Function | Gate it adds | Runs on |
|-------|----------|--------------|---------|
| 1 — **Authenticated** | `authenticate` | resolve profile + `is_active` | every authed route/tool, both surfaces |
| 2 — **System-authorized** | `require_system_access` | `has_system_access` (gating-team membership) | the *gated* tier of both surfaces |

Level 2 is a **typestate** chain: `require_system_access` only accepts an
`AuthenticatedProfile` (produced solely by `authenticate`) and returns `SystemAuthorized`.
The compiler makes it impossible to run Level 2 without having passed Level 1.

**Why two levels and not one monolithic `authorize()`:** `temper-api` splits into two
router tiers. The **auth-only** tier (view own profile, request access, `team join`) runs
Level 1 but deliberately **skips** Level 2 — that is how a not-yet-approved user requests
access in the first place. The **gated** tier adds Level 2. A single always-run-all-gates
function would break the request-access flow. (`temper-mcp` has no auth-only tier — every
tool requires Level 2.) See [authorization-seam.md](./authorization-seam.md).

## Checklist: changing an auth path

- [ ] **New gate?** Add it to the seam (`crates/temper-services/src/auth/mod.rs`) at the
      correct level — *not* to a surface's middleware. Both surfaces pick it up for free.
- [ ] **New `AuthzError` variant?** Map it in **both** transport mappers:
      `temper-api` `middleware/system_access.rs` + `middleware/auth.rs`, and `temper-mcp`
      `service.rs::map_authz_error`. The compiler's exhaustiveness check enforces this.
- [ ] **New token shape / issuer?** See [jwt-verification.md](./jwt-verification.md) and
      the [machine-token contract](./machine-token-contract.md) — one claim contract both
      issuers conform to; the Rust seam normalizes exactly one machine shape.
- [ ] **Ran the parity e2e?** `tests/e2e/tests/auth_seam_parity_e2e.rs` drives the
      *production caller* on both surfaces. A direct-call unit test passes even if a
      surface forgot to wire the seam — the e2e is what proves the wiring.

## Documents in this area

- **[authorization-seam.md](./authorization-seam.md)** — the two-level chain, `AuthzError`,
  the typestate, the router-tier split, per-surface transport mapping, and the parity test.
- **[cognitive-map-authoring.md](./cognitive-map-authoring.md)** — the *per-resource* axis:
  who may author into a cogmap or modify a resource. The three predicates
  (`cogmap_authorable_by_profile` / `can_modify_resource` / `anchor_readable_by_profile`),
  the full per-op gate map, agent-vs-human principals, and the known hardening gaps (F1–F3).
- **[jwt-verification.md](./jwt-verification.md)** — `JwksKeyStore`, RS256 (Auth0/OIDC) and
  EdDSA (the SAML Authorization Server), the per-surface audience split, and the email
  resolution ladder.
- **[reconcile-channel.md](./reconcile-channel.md)** — the internal SAML reconcile channel,
  its shared-secret/HMAC trust model, *why not an origin allow-list on Vercel*, and the
  bounded blast radius.
- **[machine-token-contract.md](./machine-token-contract.md)** — the issuer /
  resource-server boundary and the single machine-token claim contract both issuers
  conform to (M2M agent principals; Stage 4a+4b shipped). Includes the end-to-end flow and
  the operator runbook for provisioning an Auth0 M2M agent.

## Related, elsewhere

- **Operator setup** for SAML SSO (env, keys, IdP row, group mappings):
  [../guides/self-hosting-saml.md](../guides/self-hosting-saml.md). That guide is the
  runbook; *this* area is the security model it implements.
- **Design spec** the seam was built from:
  [../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md](../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md).
