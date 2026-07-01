-- Generalized access-capability arc — Deliverable 2: the seam + table, ALONGSIDE (no behavior change).
-- Design: docs/superpowers/specs/2026-06-30-generalized-access-capability-model-design.md §3.3, §3.5, §4 step 1.
--
-- This migration is PURELY ADDITIVE: it introduces `kb_access_grants` (dual-polymorphic) and the
-- `can()` capability seam NEXT TO the existing access functions. Nothing reads the new table on the
-- existing read/write paths yet (that is Deliverable 4), and no existing function is replaced here —
-- the three-function lockstep flip (D4) and the `cogmap_authorable_by_profile` rewrite (D3) are
-- deliberately NOT in this migration. So every current predicate keeps its exact behavior; the only
-- new capability is whatever calls `can()` (today: nothing).
--
-- Namespace-free by construction (no `SET search_path`): every name resolves against the connection's
-- search_path — `public` everywhere (prod/dev/e2e and the ephemeral artifact-test DBs).

-- ============================================================================
-- §3.3 — kb_access_grants: dual-polymorphic (subject) × (principal) × rwx grants.
-- ----------------------------------------------------------------------------
-- CONFORM to the house polymorphic-anchor idiom (kb_resource_homes / kb_resource_access in
-- 20260624000001_canonical_schema.sql): VARCHAR(64) discriminator + CHECK + NO real FK on the
-- polymorphic columns ("integrity is the CHECK + the granting path"), four rwx booleans, the
-- `write|delete|grant ⇒ read` coherence CHECK carried verbatim from kb_resource_access (2026-06-02 OQ-1).
-- AMEND (Q-C): the single resource_id→kb_resources FK becomes a *dual*-polymorphic (subject_table,
-- subject_id), extending the grantable subject set to contexts + cogmaps (inexpressible in the
-- resources-only kb_resource_access). Eventually subsumes kb_resource_access (D4/D5).
CREATE TABLE kb_access_grants (
    id                    UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    subject_table         VARCHAR(64) NOT NULL CHECK (subject_table   IN ('kb_resources','kb_contexts','kb_cogmaps')),
    subject_id            UUID NOT NULL,
    principal_table       VARCHAR(64) NOT NULL CHECK (principal_table IN ('kb_teams','kb_profiles')),
    principal_id          UUID NOT NULL,
    can_read              BOOLEAN NOT NULL DEFAULT false,
    can_write             BOOLEAN NOT NULL DEFAULT false,
    can_delete            BOOLEAN NOT NULL DEFAULT false,
    can_grant             BOOLEAN NOT NULL DEFAULT false,
    granted_by_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    granted_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (subject_table, subject_id, principal_table, principal_id),
    -- §2 coherence: you cannot mutate or re-share what you cannot read.
    CHECK ((can_write OR can_delete OR can_grant) <= can_read)
);
CREATE INDEX idx_kb_access_grants_subject   ON kb_access_grants(subject_table, subject_id);
CREATE INDEX idx_kb_access_grants_principal ON kb_access_grants(principal_table, principal_id);

-- ============================================================================
-- §3.5 — profile_explicit_grant: a profile's explicit-grant reach for ACTION on SUBJECT.
-- ----------------------------------------------------------------------------
-- The subject-polymorphic generalization of resources_visible_to's two grant UNION branches: a
-- DIRECT profile-anchored grant, OR a team-anchored grant on a REACHABLE (self-or-ancestor) team.
-- This is the union-up Profile axis. Reads kb_access_grants only (never kb_resource_access) — the
-- two stores coexist until D4 migrates kb_resource_access in.
CREATE FUNCTION profile_explicit_grant(
    p_profile uuid, p_action text, p_subject_table text, p_subject_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    SELECT EXISTS (
        SELECT 1 FROM kb_access_grants g
        WHERE g.subject_table = p_subject_table AND g.subject_id = p_subject_id
          AND CASE p_action WHEN 'read'   THEN g.can_read
                            WHEN 'write'  THEN g.can_write
                            WHEN 'delete' THEN g.can_delete
                            WHEN 'grant'  THEN g.can_grant
                            ELSE false END
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'
                   AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    );
$$;

-- derived_access_profile: the per-subject DERIVED floor (the non-explicit-grant reach), delegating to
-- the EXISTING read/write predicates. A thin shim for D2 (so can() lands with no behavior change);
-- D4 inlines these to read the unified store. Untouched existing functions ⇒ untouched behavior.
CREATE FUNCTION derived_access_profile(
    p_profile uuid, p_action text, p_subject_table text, p_subject_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN p_subject_table = 'kb_resources' AND p_action = 'read'  THEN
            p_subject_id IN (SELECT resource_id FROM resources_visible_to(p_profile))
        WHEN p_subject_table = 'kb_resources' AND p_action = 'write' THEN
            can_modify_resource(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_cogmaps'  AND p_action = 'read'  THEN
            cogmap_readable_by_profile(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_cogmaps'  AND p_action = 'write' THEN
            cogmap_authorable_by_profile(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_contexts' AND p_action = 'read'  THEN
            context_visible_to(p_profile, p_subject_id)
        ELSE false
    END;
$$;

-- ============================================================================
-- §3.5 — can(principal, action, subject): the unified capability seam.
-- ----------------------------------------------------------------------------
-- Subject-polymorphic {kb_resources,kb_contexts,kb_cogmaps}; action {read,write,delete,grant}.
-- Axis-dispatched on the principal sum type (CS-1, mirrors resources_readable_by):
--   • Profile (consumer): union-up — explicit grant OR the derived floor.
--   • Cogmap (producer): intersection / least-privilege, RESOURCE subjects + READ only, and takes
--     NO explicit grants (Q-B leak-safety: a profile-axis grant never enters the producer intersection).
-- Nothing calls this yet (D2 is alongside-only); surfaces migrate to it in later deliverables.
CREATE FUNCTION can(
    p_principal_table text, p_principal_id uuid, p_action text,
    p_subject_table text, p_subject_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE p_principal_table
        WHEN 'kb_profiles' THEN
            profile_explicit_grant(p_principal_id, p_action, p_subject_table, p_subject_id)
            OR derived_access_profile(p_principal_id, p_action, p_subject_table, p_subject_id)
        WHEN 'kb_cogmaps' THEN
            p_subject_table = 'kb_resources' AND p_action = 'read'
            AND p_subject_id IN (SELECT resource_id FROM resources_accessible_to_cogmap(p_principal_id))
        ELSE false
    END;
$$;
