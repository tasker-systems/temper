-- seed-dev-data.sql
--
-- Populates the local dev database with realistic content for UI development.
-- Idempotent — safe to run multiple times (uses ON CONFLICT DO NOTHING/UPDATE).
--
-- Usage:
--   cargo make seed                              # uses SEED_EMAIL env var, or defaults
--   cargo make seed "you@example.com"            # pass email as argument
--   psql $DATABASE_URL -v seed_email="'you@example.com'" -f scripts/seed-dev-data.sql
--
-- If a profile with the given email already exists (e.g. from a prior Auth0
-- login), the seed data is owned by that profile. Otherwise a new "Dev User"
-- profile is created and linked. Either way, log in with that email and the
-- vault browser shows the seed content.
--
-- To remove seed data:
--   DELETE FROM kb_resources WHERE origin_uri LIKE 'seed://%';
--   DELETE FROM kb_contexts WHERE id IN ('00000000-…-00bb-…001', …002, …003);

-- Default email if not passed via -v seed_email='...'
\if :{?seed_email}
\else
  \set seed_email '''dev@temperkb.local'''
\endif

BEGIN;

-- Pass the email in via a transaction-scoped GUC so DO blocks can read it
SET LOCAL seed.email = :seed_email;

-- ─── Resolve or create profile ─────────────────────────────────────────────
-- If a profile with this email already exists (from a real Auth0 login),
-- use it. Otherwise create a dev profile. This avoids the two-profile problem
-- where seed data is owned by a profile that Auth0 never resolves to.

DO $$
DECLARE
    v_email TEXT := current_setting('seed.email');
    v_pid   UUID;
BEGIN
    -- Check for existing profile by email (via auth links, where Auth0 stores it)
    SELECT p.id INTO v_pid
      FROM kb_profiles p
      JOIN kb_profile_auth_links al ON al.profile_id = p.id
     WHERE al.email = v_email
     LIMIT 1;

    -- Fall back to profile table email
    IF v_pid IS NULL THEN
        SELECT id INTO v_pid FROM kb_profiles WHERE email = v_email LIMIT 1;
    END IF;

    -- Create a new dev profile if none found
    IF v_pid IS NULL THEN
        v_pid := '00000000-0000-0000-00aa-000000000001'::uuid;
        INSERT INTO kb_profiles (id, display_name, email, slug)
        VALUES (v_pid, 'Dev User', v_email, 'dev-user')
        ON CONFLICT (id) DO UPDATE SET email = EXCLUDED.email;

        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id, email, is_default)
        VALUES (gen_random_uuid(), v_pid, 'auth0', 'auth0|dev-seed-user', v_email, true)
        ON CONFLICT DO NOTHING;

        RAISE NOTICE 'Created dev profile %', v_pid;
    ELSE
        RAISE NOTICE 'Using existing profile % for %', v_pid, v_email;
    END IF;

    -- Stash the resolved profile ID for the rest of the transaction
    PERFORM set_config('seed.profile_id', v_pid::text, true);
END;
$$;

-- ─── Well-known IDs ────────────────────────────────────────────────────────

-- Doc type IDs (from migrations)
\set dt_task     '''00000000-0000-0000-0001-000000000008'''
\set dt_goal     '''00000000-0000-0000-0001-000000000009'''
\set dt_session  '''00000000-0000-0000-0001-000000000002'''
\set dt_research '''00000000-0000-0000-0001-000000000004'''
\set dt_decision '''00000000-0000-0000-0001-00000000000b'''
\set dt_concept  '''00000000-0000-0000-0001-000000000006'''

-- Context IDs
\set ctx_acme    '''00000000-0000-0000-00bb-000000000001'''
\set ctx_infra   '''00000000-0000-0000-00bb-000000000002'''
\set ctx_learn   '''00000000-0000-0000-00bb-000000000003'''

-- ─── Contexts (owned by resolved profile) ──────────────────────────────────

DO $$
DECLARE v_pid UUID := current_setting('seed.profile_id')::uuid;
BEGIN
    INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id) VALUES
        ('00000000-0000-0000-00bb-000000000001', 'acme-app',       'kb_profiles', v_pid),
        ('00000000-0000-0000-00bb-000000000002', 'infrastructure', 'kb_profiles', v_pid),
        ('00000000-0000-0000-00bb-000000000003', 'learning',       'kb_profiles', v_pid)
    ON CONFLICT (id) DO UPDATE SET kb_owner_id = EXCLUDED.kb_owner_id;
END;
$$;

-- ─── Helper: insert resource + manifest in one shot ────────────────────────

CREATE OR REPLACE FUNCTION _seed_resource(
    p_id           UUID,
    p_context_id   UUID,
    p_doc_type_id  UUID,
    p_title        TEXT,
    p_slug         TEXT,
    p_managed_meta JSONB DEFAULT '{}'::jsonb,
    p_days_ago     INT DEFAULT 0
) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE
    v_pid UUID := current_setting('seed.profile_id')::uuid;
    ts TIMESTAMPTZ := now() - (p_days_ago || ' days')::interval;
BEGIN
    INSERT INTO kb_resources
        (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
         originator_profile_id, owner_profile_id, is_active, created, updated)
    VALUES
        (p_id, p_context_id, p_doc_type_id,
         'seed://' || p_slug, p_title, p_slug,
         v_pid, v_pid, true, ts, ts)
    ON CONFLICT (id) DO UPDATE
        SET title = EXCLUDED.title, slug = EXCLUDED.slug,
            originator_profile_id = v_pid, owner_profile_id = v_pid,
            updated = ts;

    INSERT INTO kb_resource_manifests (resource_id, managed_meta, updated)
    VALUES (p_id, p_managed_meta, ts)
    ON CONFLICT (resource_id) DO UPDATE
        SET managed_meta = EXCLUDED.managed_meta, updated = ts;
END;
$$;

-- ─── Goals ─────────────────────────────────────────────────────────────────

SELECT _seed_resource(
    'a0000000-0001-0000-0000-000000000001', :ctx_acme, :dt_goal,
    'Launch MVP', 'launch-mvp',
    '{"temper-type":"goal","temper-status":"active","temper-seq":1}'::jsonb, 14
);
SELECT _seed_resource(
    'a0000000-0001-0000-0000-000000000002', :ctx_acme, :dt_goal,
    'API v2 Migration', 'api-v2-migration',
    '{"temper-type":"goal","temper-status":"active","temper-seq":2}'::jsonb, 10
);
SELECT _seed_resource(
    'a0000000-0001-0000-0000-000000000003', :ctx_infra, :dt_goal,
    'Zero-Downtime Deploys', 'zero-downtime-deploys',
    '{"temper-type":"goal","temper-status":"planning","temper-seq":1}'::jsonb, 20
);

-- ─── Tasks ─────────────────────────────────────────────────────────────────

SELECT _seed_resource(
    'a0000000-0002-0000-0000-000000000001', :ctx_acme, :dt_task,
    'Auth middleware implementation', 'auth-middleware-implementation',
    '{"temper-type":"task","temper-stage":"in-progress","temper-mode":"build","temper-effort":"medium","temper-seq":100,"temper-goal":"launch-mvp"}'::jsonb, 2
);
SELECT _seed_resource(
    'a0000000-0002-0000-0000-000000000002', :ctx_acme, :dt_task,
    'Design user onboarding flow', 'design-user-onboarding-flow',
    '{"temper-type":"task","temper-stage":"backlog","temper-mode":"plan","temper-effort":"large","temper-seq":110,"temper-goal":"launch-mvp"}'::jsonb, 5
);
SELECT _seed_resource(
    'a0000000-0002-0000-0000-000000000003', :ctx_acme, :dt_task,
    'Set up rate limiting', 'set-up-rate-limiting',
    '{"temper-type":"task","temper-stage":"backlog","temper-mode":"build","temper-effort":"small","temper-seq":120}'::jsonb, 7
);
SELECT _seed_resource(
    'a0000000-0002-0000-0000-000000000004', :ctx_acme, :dt_task,
    'REST endpoint audit', 'rest-endpoint-audit',
    '{"temper-type":"task","temper-stage":"done","temper-mode":"plan","temper-effort":"small","temper-seq":90,"temper-goal":"api-v2-migration"}'::jsonb, 12
);
SELECT _seed_resource(
    'a0000000-0002-0000-0000-000000000005', :ctx_acme, :dt_task,
    'Migrate billing endpoints to v2', 'migrate-billing-endpoints-to-v2',
    '{"temper-type":"task","temper-stage":"in-progress","temper-mode":"build","temper-effort":"large","temper-seq":130,"temper-goal":"api-v2-migration"}'::jsonb, 3
);
SELECT _seed_resource(
    'a0000000-0002-0000-0000-000000000006', :ctx_infra, :dt_task,
    'Evaluate blue-green deployment tooling', 'evaluate-blue-green-deployment-tooling',
    '{"temper-type":"task","temper-stage":"in-progress","temper-mode":"plan","temper-effort":"medium","temper-seq":100,"temper-goal":"zero-downtime-deploys"}'::jsonb, 4
);
SELECT _seed_resource(
    'a0000000-0002-0000-0000-000000000007', :ctx_infra, :dt_task,
    'Set up staging environment', 'set-up-staging-environment',
    '{"temper-type":"task","temper-stage":"done","temper-mode":"build","temper-effort":"medium","temper-seq":90}'::jsonb, 15
);
SELECT _seed_resource(
    'a0000000-0002-0000-0000-000000000008', :ctx_learn, :dt_task,
    'Work through WASM tutorial', 'work-through-wasm-tutorial',
    '{"temper-type":"task","temper-stage":"backlog","temper-mode":"build","temper-effort":"small","temper-seq":100}'::jsonb, 8
);

-- ─── Sessions ──────────────────────────────────────────────────────────────

SELECT _seed_resource(
    'a0000000-0003-0000-0000-000000000001', :ctx_acme, :dt_session,
    'Auth middleware — JWT validation and JWKS caching', 'auth-middleware-jwt-validation-and-jwks-caching',
    '{"temper-type":"session","temper-seq":200}'::jsonb, 2
);
SELECT _seed_resource(
    'a0000000-0003-0000-0000-000000000002', :ctx_acme, :dt_session,
    'API v2 planning — identified breaking changes in billing', 'api-v2-planning-identified-breaking-changes',
    '{"temper-type":"session","temper-seq":190}'::jsonb, 5
);
SELECT _seed_resource(
    'a0000000-0003-0000-0000-000000000003', :ctx_acme, :dt_session,
    'Onboarding flow brainstorm with design team', 'onboarding-flow-brainstorm-with-design-team',
    '{"temper-type":"session","temper-seq":180}'::jsonb, 8
);
SELECT _seed_resource(
    'a0000000-0003-0000-0000-000000000004', :ctx_acme, :dt_session,
    'REST audit complete — 14 endpoints need versioning', 'rest-audit-complete-14-endpoints-need-versioning',
    '{"temper-type":"session","temper-seq":170}'::jsonb, 12
);
SELECT _seed_resource(
    'a0000000-0003-0000-0000-000000000005', :ctx_infra, :dt_session,
    'Blue-green evaluation — Argo Rollouts vs Flagger', 'blue-green-evaluation-argo-rollouts-vs-flagger',
    '{"temper-type":"session","temper-seq":200}'::jsonb, 4
);
SELECT _seed_resource(
    'a0000000-0003-0000-0000-000000000006', :ctx_infra, :dt_session,
    'Staging env provisioned with Terraform', 'staging-env-provisioned-with-terraform',
    '{"temper-type":"session","temper-seq":190}'::jsonb, 15
);
SELECT _seed_resource(
    'a0000000-0003-0000-0000-000000000007', :ctx_learn, :dt_session,
    'Rust ownership model deep dive', 'rust-ownership-model-deep-dive',
    '{"temper-type":"session","temper-seq":200}'::jsonb, 6
);

-- ─── Research ──────────────────────────────────────────────────────────────

SELECT _seed_resource(
    'a0000000-0004-0000-0000-000000000001', :ctx_acme, :dt_research,
    'OAuth provider comparison — Auth0 vs Clerk vs WorkOS', 'oauth-provider-comparison',
    '{"temper-type":"research","temper-seq":300}'::jsonb, 18
);
SELECT _seed_resource(
    'a0000000-0004-0000-0000-000000000002', :ctx_acme, :dt_research,
    'Token refresh patterns and edge cases', 'token-refresh-patterns-and-edge-cases',
    '{"temper-type":"research","temper-seq":310}'::jsonb, 11
);
SELECT _seed_resource(
    'a0000000-0004-0000-0000-000000000003', :ctx_infra, :dt_research,
    'Kubernetes rollout strategies survey', 'kubernetes-rollout-strategies-survey',
    '{"temper-type":"research","temper-seq":300}'::jsonb, 20
);
SELECT _seed_resource(
    'a0000000-0004-0000-0000-000000000004', :ctx_learn, :dt_research,
    'Comparing Rust async runtimes — Tokio vs async-std vs smol', 'comparing-rust-async-runtimes',
    '{"temper-type":"research","temper-seq":300}'::jsonb, 9
);

-- ─── Decisions ─────────────────────────────────────────────────────────────

SELECT _seed_resource(
    'a0000000-0005-0000-0000-000000000001', :ctx_acme, :dt_decision,
    'JWT rotation over session tokens', 'jwt-rotation-over-session-tokens',
    '{"temper-type":"decision","temper-seq":400}'::jsonb, 13
);
SELECT _seed_resource(
    'a0000000-0005-0000-0000-000000000002', :ctx_acme, :dt_decision,
    'REST over GraphQL for public API', 'rest-over-graphql-for-public-api',
    '{"temper-type":"decision","temper-seq":410}'::jsonb, 16
);
SELECT _seed_resource(
    'a0000000-0005-0000-0000-000000000003', :ctx_infra, :dt_decision,
    'Argo Rollouts for progressive delivery', 'argo-rollouts-for-progressive-delivery',
    '{"temper-type":"decision","temper-seq":400}'::jsonb, 3
);

-- ─── Concepts ──────────────────────────────────────────────────────────────

SELECT _seed_resource(
    'a0000000-0006-0000-0000-000000000001', :ctx_acme, :dt_concept,
    'Idempotency keys for safe retries', 'idempotency-keys-for-safe-retries',
    '{"temper-type":"concept","temper-seq":500}'::jsonb, 10
);
SELECT _seed_resource(
    'a0000000-0006-0000-0000-000000000002', :ctx_learn, :dt_concept,
    'Zero-copy deserialization patterns', 'zero-copy-deserialization-patterns',
    '{"temper-type":"concept","temper-seq":500}'::jsonb, 7
);
SELECT _seed_resource(
    'a0000000-0006-0000-0000-000000000003', :ctx_acme, :dt_concept,
    'Circuit breaker for external service calls', 'circuit-breaker-for-external-service-calls',
    '{"temper-type":"concept","temper-seq":510}'::jsonb, 19
);

-- ─── Resource contents (stored as chunks for the detail view) ──────────────
-- Content is stored in kb_chunks + kb_chunk_content. We use a zero vector for
-- the embedding since semantic search isn't needed on seed data.

CREATE OR REPLACE FUNCTION _seed_content(
    p_resource_id UUID,
    p_content     TEXT
) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE
    v_chunk_id    UUID := uuid_generate_v7();
    v_revision_id UUID := uuid_generate_v7();
    v_body_hash   TEXT := md5(p_content);
    v_zero_vec vector(768) := ('[' || repeat('0,', 767) || '0]')::vector;
BEGIN
    -- Remove existing chunks for this resource
    DELETE FROM kb_chunks WHERE resource_id = p_resource_id;

    -- Synthesize a revision row to satisfy kb_chunks.first_revision_id NOT NULL.
    -- audit_id is NULL because this is dev seed data, not an actual write.
    INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count)
    VALUES (v_revision_id, p_resource_id, NULL, v_body_hash, 1);

    -- Insert single chunk (no splitting needed for seed data)
    INSERT INTO kb_chunks
        (id, resource_id, chunk_index, version, header_path, content_hash,
         embedding, is_current, first_revision_id)
    VALUES
        (v_chunk_id, p_resource_id, 0, 1, '', v_body_hash,
         v_zero_vec, true, v_revision_id);

    INSERT INTO kb_chunk_content (chunk_id, content)
    VALUES (v_chunk_id, p_content)
    ON CONFLICT (chunk_id) DO UPDATE SET content = EXCLUDED.content;
END;
$$;

SELECT _seed_content('a0000000-0003-0000-0000-000000000001',
'## Goal

Implement JWT validation middleware for the Axum API server.

## What Happened

- Chose `jsonwebtoken` crate over `jwt-simple` — better JWKS support
- Implemented JWKS caching with 1-hour TTL and background refresh
- Added `AuthClaims` struct with provider, email, and profile resolution
- Integrated with Auth0 RS256 keys via `.well-known/jwks.json`

## Decisions

- **RS256 only** — no symmetric keys in production. Simpler JWKS logic.
- **Cache JWKS for 1 hour** — Auth0 rotates keys infrequently. Fallback: refetch on unknown kid.
- **Profile resolution in middleware** — every authed request resolves to a `Profile`. No lazy resolution.

## Next Steps

- Wire up refresh token rotation endpoint
- Add rate limiting per-profile (separate task)
- Integration tests with mock JWKS server');

SELECT _seed_content('a0000000-0005-0000-0000-000000000001',
'## Context

The API needs stateless authentication for horizontal scaling. Session tokens require a shared store (Redis/Postgres) across instances, adding latency and a failure point.

## Decision

JWT with short-lived access tokens (15 min) and rotating refresh tokens.

**Trade-off:** More complexity in token handling (rotation, revocation lists for compromised tokens), but no shared session store. Stateless auth means any instance can validate without a round-trip.

## Alternatives Considered

1. **Session tokens in Redis** — simpler, but adds Redis as a dependency and ~2ms per request for session lookup.
2. **Opaque tokens with introspection** — standard but requires an introspection endpoint, adding latency equivalent to session lookup.

## Consequences

- Need a `/auth/refresh` endpoint with token rotation
- Need a short-lived revocation list (in-memory, TTL = access token lifetime)
- Client SDKs must handle 401 → refresh → retry flow');

SELECT _seed_content('a0000000-0004-0000-0000-000000000001',
'## Summary

Evaluated three OAuth/identity providers for the acme-app authentication layer.

## Auth0

- **Pros:** Mature, extensive documentation, good Rust SDK support, flexible tenant configuration
- **Cons:** Pricing scales with MAU, some features locked behind Enterprise tier
- **Verdict:** Best fit for our scale and requirements

## Clerk

- **Pros:** Modern DX, beautiful prebuilt components, fast integration
- **Cons:** Svelte support is community-maintained, less control over token claims
- **Verdict:** Good for rapid prototyping, less suitable for API-first architecture

## WorkOS

- **Pros:** Enterprise SSO focus, clean API design, good directory sync
- **Cons:** Overkill for B2C, pricing model assumes enterprise contracts
- **Verdict:** Revisit if we add B2B/enterprise tier

## Recommendation

**Auth0** — best balance of maturity, API-first design, and cost at our current scale (~500 MAU). Supports device auth flow for CLI, RS256 JWTs for API, and has a free tier that covers development.');

DROP FUNCTION _seed_content;

-- ─── Relationships (edges) for graph visualization ─────────────────────────
-- Each edge source is a concept; target is a related task/research/session.
-- Edges are projections of `relationship_asserted` events — emit the event
-- and the row together.

DO $$
DECLARE
    v_pid uuid := current_setting('seed.profile_id')::uuid;
    v_asserted_type uuid := (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted');
    v_topic uuid := '019e3d6f-2300-7000-8000-000000000050';
    v_scope uuid := '019e3d6f-2300-7000-8000-000000000010';

    source_ids uuid[] := ARRAY[
        'a0000000-0006-0000-0000-000000000001'::uuid,
        'a0000000-0006-0000-0000-000000000001'::uuid,
        'a0000000-0006-0000-0000-000000000002'::uuid,
        'a0000000-0006-0000-0000-000000000002'::uuid,
        'a0000000-0006-0000-0000-000000000003'::uuid,
        'a0000000-0006-0000-0000-000000000003'::uuid,
        'a0000000-0006-0000-0000-000000000003'::uuid,
        'a0000000-0004-0000-0000-000000000002'::uuid
    ];
    target_ids uuid[] := ARRAY[
        'a0000000-0002-0000-0000-000000000005'::uuid,
        'a0000000-0004-0000-0000-000000000002'::uuid,
        'a0000000-0004-0000-0000-000000000004'::uuid,
        'a0000000-0003-0000-0000-000000000007'::uuid,
        'a0000000-0002-0000-0000-000000000001'::uuid,
        'a0000000-0004-0000-0000-000000000001'::uuid,
        'a0000000-0003-0000-0000-000000000001'::uuid,
        'a0000000-0004-0000-0000-000000000001'::uuid
    ];
    kinds      text[] := ARRAY['near','near','near','near','near','near','near','leads_to'];
    polarities text[] := ARRAY['forward','forward','forward','forward','forward','forward','forward','inverse'];
    labels     text[] := ARRAY['relates_to','relates_to','relates_to','relates_to','relates_to','relates_to','relates_to','depends_on'];

    v_event_id uuid;
BEGIN
    FOR i IN 1 .. array_length(source_ids, 1) LOOP
        v_event_id := public.uuid_generate_v7();

        INSERT INTO kb_events (
            id, event_type_id, profile_id, device_id, topic_id, scope_id,
            payload, metadata, "references", correlation_id, occurred_at, created
        ) VALUES (
            v_event_id, v_asserted_type, v_pid, 'seed', v_topic, v_scope,
            jsonb_build_object(
                'source_resource_id', source_ids[i],
                'target', jsonb_build_object('kind', 'resource', 'value', target_ids[i]),
                'edge_kind', kinds[i],
                'polarity',  polarities[i],
                'label',     labels[i],
                'weight',    1.0
            ),
            jsonb_build_object('source', 'seed'),
            '[]'::jsonb, v_event_id, now(), now()
        );

        INSERT INTO kb_resource_edges (
            id, source_resource_id, target_resource_id,
            edge_kind, polarity, label, weight,
            asserted_by_event_id, last_event_id, is_folded
        ) VALUES (
            public.uuid_generate_v7(), source_ids[i], target_ids[i],
            kinds[i]::edge_kind, polarities[i]::edge_polarity, labels[i], 1.0,
            v_event_id, v_event_id, false
        )
        ON CONFLICT DO NOTHING;
    END LOOP;
END;
$$;

-- ─── Cleanup ───────────────────────────────────────────────────────────────

DROP FUNCTION _seed_resource;

COMMIT;

-- ─── Summary ───────────────────────────────────────────────────────────────
\echo ''
\echo '✓ Seed data inserted successfully.'
\echo ''
\echo '  Contexts: acme-app, infrastructure, learning'
\echo '  Resources: 3 goals, 8 tasks, 7 sessions, 4 research, 3 decisions, 3 concepts'
\echo '  Edges: ~8 concept member edges for graph visualization'
\echo ''
\echo '  Profile email set to:' :seed_email
\echo '  Log in via Auth0 with that email and the vault browser will show seed data.'
\echo ''
