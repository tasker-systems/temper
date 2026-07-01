-- SAML SP Phase 1 (issue #224) — Authorization Server state + IdP config tables.
-- Plan: docs/superpowers/plans/2026-07-01-saml-sp-phase1.md, Task 2.1.
-- Design: docs/superpowers/specs/2026-07-01-saml-sp-temper-authorization-server-design.md.
--
-- This migration is PURELY ADDITIVE: four new tables backing the minimal OAuth Authorization
-- Server that fronts a SAML IdP (temper-cloud `api/oauth/*` functions). No existing table or
-- function is touched, and no Rust `sqlx::query!` macro reads these tables (the AS lives entirely
-- in the temper-cloud TypeScript layer), so the committed `.sqlx/` cache is unaffected.
--
-- Namespace-free by construction (no `SET search_path`): every name resolves against the
-- connection's search_path — `public` everywhere (prod/dev/e2e). Runs on both PG17 (Neon cloud)
-- and PG18 (local/CI Docker); no version-specific SQL.

-- ============================================================================
-- kb_saml_idp — the (single active) upstream IdP configuration.
-- ----------------------------------------------------------------------------
-- Decision (design §): single active IdP per instance, non-singleton row shape so an operator can
-- stage a replacement and flip `is_active`. `idp_*` describe the IdP; `sp_*`/`acs_url` describe how
-- this Temper instance presents itself as the SP. `email_attr`/`stable_id_attr` name the assertion
-- attributes that map to the minted token's `email` / `sub` (persistent NameID preferred for `sub`).
CREATE TABLE kb_saml_idp (
    idp_key        TEXT PRIMARY KEY,
    is_active      BOOLEAN NOT NULL DEFAULT false,
    idp_cert       TEXT NOT NULL,
    idp_sso_url    TEXT NOT NULL,
    idp_entity_id  TEXT NOT NULL,
    sp_entity_id   TEXT NOT NULL,
    acs_url        TEXT NOT NULL,
    nameid_format  TEXT NOT NULL,
    email_attr     TEXT NOT NULL,
    stable_id_attr TEXT NOT NULL,
    created        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ============================================================================
-- kb_oauth_flow — a single in-flight authorization-code flow (PKCE), keyed by relay_state.
-- ----------------------------------------------------------------------------
-- Lifecycle: `pending_saml` (created at /oauth/authorize, awaiting the IdP round-trip) →
-- `code_issued` (assertion validated at ACS, one-time `code_hash` bound) → `consumed` (code
-- exchanged at /oauth/token). `relay_state` and `code_hash` are UNIQUE — the unique constraint is
-- itself the btree index used for equality lookups on both columns, so no separate index is
-- declared (that would be redundant on an immutable migration). `code_hash` is NULL until a code is
-- issued (UNIQUE permits multiple NULLs). `claims` carries the mapped AuthClaims to mint at token time.
CREATE TABLE kb_oauth_flow (
    id                   UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    relay_state          TEXT NOT NULL UNIQUE,
    code_hash            TEXT UNIQUE,
    status               TEXT NOT NULL CHECK (status IN ('pending_saml', 'code_issued', 'consumed')),
    client_id            TEXT NOT NULL,
    redirect_uri         TEXT NOT NULL,
    code_challenge       TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL,
    oauth_state          TEXT NOT NULL,
    audience             TEXT NOT NULL,
    claims               JSONB,
    created              TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at           TIMESTAMPTZ NOT NULL
);

-- ============================================================================
-- kb_oauth_refresh_tokens — issued refresh tokens (hashed), single-use rotation chain.
-- ----------------------------------------------------------------------------
-- `token_hash` UNIQUE (its constraint index serves the lookup). Rotation: exchanging a refresh token
-- sets `revoked_at` and points `rotated_to` at the successor row; a token with `revoked_at` set is
-- dead. `claims` carries the AuthClaims to re-mint on refresh.
CREATE TABLE kb_oauth_refresh_tokens (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    token_hash  TEXT NOT NULL UNIQUE,
    client_id   TEXT NOT NULL,
    claims      JSONB NOT NULL,
    created     TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL,
    revoked_at  TIMESTAMPTZ,
    rotated_to  UUID REFERENCES kb_oauth_refresh_tokens(id)
);

-- ============================================================================
-- kb_saml_replay — assertion-ID replay guard. One row per consumed assertion; the PK enforces
-- single-use (INSERT ... ON CONFLICT DO NOTHING; 0 rows ⇒ replay). `expires_at` bounds retention so
-- consumed rows can be swept once the assertion's own validity window has passed.
CREATE TABLE kb_saml_replay (
    assertion_id TEXT PRIMARY KEY,
    expires_at   TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_kb_saml_replay_expires ON kb_saml_replay(expires_at);
