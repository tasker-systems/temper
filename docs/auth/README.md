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

And the corollary, added when the seam took over principal construction:
**the seam owns the principal.** A surface hands in a *verified token* and gets back an
`AuthenticatedProfile`. It never builds an `AuthClaims`. If a surface can construct a
principal, it can construct a *different* principal than its sibling would — which is
exactly how the drift above keeps recurring one level down.

## The surfaces and the seam

```text
  temper-api (HTTP)                          temper-mcp (rmcp tool call / initialize)
  ───────────────────                        ────────────────────────────────────────
  require_auth (middleware/auth.rs)          require_mcp_auth (middleware.rs)
    · verify JWT (JwksKeyStore)                · verify JWT (same JwksKeyStore)
    · aud = config.auth.audience               · aud = config.auth.audience (the SAME one)
    ↓ decode → RawJwtClaims                    ↓ inject RawJwtClaims + BearerToken
        │                                      ensure_profile_from_parts (service.rs)
        │                                          │
        └──────────────┐          ┌────────────────┘
                       ▼          ▼
             temper-services::auth  (the seam — sequence lives here once)
               authenticate_token(&state, &raw, token)   → AuthenticatedProfile [Level 1]
                 · classify → Machine | Human | Refuse         (normalize.rs)
                 · human: the email ladder                     (email.rs)
                 · build AuthClaims — the ONLY constructor
                 · resolve profile + `is_active` gate          (authenticate, pub(crate))
               require_system_access(pool, &authed)      → SystemAuthorized     [Level 2]
                       │          │
        ┌──────────────┘          └────────────────┐
        ▼                                            ▼
  map AuthzError → HTTP status                map AuthzError → rmcp ErrorData
  (401 / SystemAccessRequired)                (INVALID_REQUEST, terminal)
```

Each surface owns **only** two things the seam does not: (1) JWT signature verification
(the audience differs legitimately per surface) and its decode into the shared
`RawJwtClaims`, and (2) mapping `AuthzError` to its transport's words-on-the-wire.
Everything between — classify the token, run the human email ladder, construct the
`AuthClaims`, resolve the profile, gate on `is_active`, gate on `system_access` — is the
seam.

There is a third seam entry point, off the token path: **`resolve_federated_human`**, for
an identity a trusted peer already authenticated out-of-band (the SAML
[reconcile channel](./reconcile-channel.md) — HMAC, no JWT, nothing to classify).

## The two-level chain

A future gate belongs to exactly **one** of these levels. "Add a gate = edit one
function" holds *per level*.

| Level | Function | Gate it adds | Runs on |
|-------|----------|--------------|---------|
| 1 — **Authenticated** | `authenticate_token` (public) → `authenticate` (`pub(crate)`) | classify + email ladder + resolve profile + `is_active` | every authed route/tool, both surfaces |
| 2 — **System-authorized** | `require_system_access` | `has_system_access` (gating-team membership) | the *gated* tier of both surfaces |

Level 2 is a **typestate** chain: `require_system_access` only accepts an
`AuthenticatedProfile` (produced solely by `authenticate`, which is reachable only through
`authenticate_token`) and returns `SystemAuthorized`. The compiler makes it impossible to
run Level 2 without having passed Level 1.

**Why two levels and not one monolithic `authorize()`:** `temper-api` splits into two
router tiers. The **auth-only** tier (view own profile, request access, `team join`) runs
Level 1 but deliberately **skips** Level 2 — that is how a not-yet-approved user requests
access in the first place. The **gated** tier adds Level 2. A single always-run-all-gates
function would break the request-access flow. (`temper-mcp` has no auth-only tier — every
tool requires Level 2.) See [authorization-seam.md](./authorization-seam.md).

## Checklist: changing an auth path

- [ ] **New gate?** Add it to the seam (`crates/temper-services/src/auth/mod.rs`) at the
      correct level — *not* to a surface's middleware. Both surfaces pick it up for free.
- [ ] **Building an `AuthClaims`?** Don't. Only the seam constructs a principal; the
      constructor is `authenticate_token` (token path) or `resolve_federated_human`
      (federated path). If you need a third path, add it *inside* the seam.
- [ ] **New `AuthzError` variant?** Map it in **both** transport mappers:
      `temper-api` `middleware/auth.rs` (Level 1) + `middleware/system_access.rs`
      (Level 2), and `temper-mcp` `service.rs::map_authz_error`. The compiler's
      exhaustiveness check enforces this. There are **six** variants today.
- [ ] **New token shape / issuer?** See [jwt-verification.md](./jwt-verification.md) and
      the [machine-token contract](./machine-token-contract.md) — one claim contract both
      issuers conform to; the Rust seam normalizes exactly one machine shape.
- [ ] **Ran the parity e2e?** `tests/e2e/tests/auth_seam_parity_e2e.rs` drives the
      *production caller* on both surfaces; `tests/e2e/tests/auth_seam_m2m_e2e.rs` does the
      same for machine tokens through the real MCP gate. A direct-call unit test passes
      even if a surface forgot to wire the seam — the e2e is what proves the wiring.

## Documents in this area

- **[authorization-seam.md](./authorization-seam.md)** — the three public entry points, the
  two-level chain, why the seam owns principal construction, `AuthzError` and its per-surface
  transport mapping, the typestate, the router-tier split, and the parity test.
- **[cognitive-map-authoring.md](./cognitive-map-authoring.md)** — the *per-resource* axis:
  who may author into a cogmap or modify a resource. The three predicates
  (`cogmap_authorable_by_profile` / `can_modify_resource` / `anchor_readable_by_profile`),
  the full per-op gate map, agent-vs-human principals, and the container-write cascade
  (the F1–F3 hardening findings are all shipped; they are recorded there for provenance).
- **[jwt-verification.md](./jwt-verification.md)** — `JwksKeyStore`, RS256 (Auth0/OIDC) and
  EdDSA (the SAML Authorization Server), the per-surface audience split, and what the
  surface hands the seam (`RawJwtClaims` + the raw bearer).
- **[reconcile-channel.md](./reconcile-channel.md)** — the internal SAML reconcile channel,
  its shared-secret/HMAC trust model, *why not an origin allow-list on Vercel*, and the
  bounded blast radius.
- **[machine-token-contract.md](./machine-token-contract.md)** — the issuer /
  resource-server boundary and the single machine-token claim contract both issuers
  conform to (M2M agent principals; Auth0 *and* the Temper AS both mint them). Includes the
  token-request wire shape, the end-to-end flow, and the operator runbook.

## Related, elsewhere

- **Operator setup** for SAML SSO (env, keys, IdP row, group mappings):
  [../guides/self-hosting-saml.md](../guides/self-hosting-saml.md). That guide is the
  runbook; *this* area is the security model it implements.
- **Operator/integrator guide** for machine principals (mint, reach, rotate, revoke):
  [../guides/machine-credentials.md](../guides/machine-credentials.md). Same split: that
  guide is *how to run one*, [machine-token-contract.md](./machine-token-contract.md) is
  *what the code guarantees about one*.
- **Design spec** the seam was built from:
  [../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md](../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md).
