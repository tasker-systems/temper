-- Data artifact read path — visibility-gated retrieval, counts, and ID enumeration.
--
-- Beat C of the artifact store. The write path (Beat A+B) shipped in 20260820000020;
-- this migration adds the read surface that makes committed artifacts retrievable
-- outside a test.
--
-- Design: internal/superpowers/specs/2026-08-20-resource-owned-data-artifacts-design.md
-- (vault research 01a02163-8670-7cc2-96a6-1a520ec8a0f8).
--
-- All four public functions gate through resources_visible_to(p_profile) — the same
-- visibility spine every other read in this codebase uses. An artifact is never visible
-- to a principal who cannot read its owning resource (goal clause
-- `data-visibility-never-exceeds-its-owners`).
--
-- The visibility gate is an INNER JOIN, not an array_agg into a NULL-means-unbounded
-- predicate. The array_agg-over-empty-scope-returns-NULL fall-open scar
-- (vault memory 019fc290-b5c6-7160-a9a5-db40f3fff2d2) does not apply to a JOIN — an
-- empty visible set produces zero joined rows, which is the correct answer (fail closed).
-- If a future change restructures these reads to collect IDs into an array, COALESCE
-- the aggregate to ARRAY[]::uuid[] or the gate falls open.

-- Private helper: the visibility-gated, filtered set of artifact ids for a resource.
-- Every public function SELECTs from this so the gate lives in one place.
--
-- p_include_folded defaults to false: the standard read path returns live artifacts.
-- Folded artifacts are retained (fold affects visibility, never existence) and a caller
-- that wants history passes p_include_folded := true.
CREATE FUNCTION _visible_artifacts(p_profile uuid, p_resource uuid,
                                   p_kind text DEFAULT NULL,
                                   p_intent text DEFAULT NULL,
                                   p_include_folded boolean DEFAULT false)
RETURNS TABLE(id uuid, is_folded boolean)
LANGUAGE sql STABLE AS $$
    SELECT a.id, a.is_folded
      FROM kb_data_artifacts a
      JOIN resources_visible_to(p_profile) v ON v.resource_id = a.resource_id
     WHERE a.resource_id = p_resource
       AND (p_include_folded OR NOT a.is_folded)
       AND (p_kind IS NULL OR a.artifact_kind = p_kind)
       AND (p_intent IS NULL OR a.intent = p_intent)
$$;

-- Full hydration: metadata + content for every visible artifact of a resource.
-- The content sidecar is a LEFT JOIN — an artifact may be committed with no bytes
-- (p_content was NULL), and that is legitimate, so content comes back as NULL
-- rather than the row being dropped.
CREATE FUNCTION artifacts_for_resource(p_profile uuid, p_resource uuid,
                                       p_kind text DEFAULT NULL,
                                       p_intent text DEFAULT NULL,
                                       p_include_folded boolean DEFAULT false)
RETURNS TABLE(
    artifact_id        uuid,
    resource_id        uuid,
    kind_owner_table   varchar,
    kind_owner_id      uuid,
    artifact_kind      text,
    intent             text,
    precedence         double precision,
    content_hash       text,
    content_bytes      bigint,
    shape_state        text,
    is_folded          boolean,
    created            timestamptz,
    content            jsonb
)
LANGUAGE sql STABLE AS $$
    SELECT a.id,
           a.resource_id,
           a.kind_owner_table,
           a.kind_owner_id,
           a.artifact_kind,
           a.intent,
           a.precedence,
           a.content_hash,
           a.content_bytes,
           -- No shape registry exists yet, so every artifact is permanently 'never_declared'.
           -- This is where `unchecked-never-reads-as-checked` gets its first purchase: the
           -- reader is told "unchecked" rather than shown an empty field.
           'never_declared'::text,
           a.is_folded,
           a.created,
           c.content
      FROM _visible_artifacts(p_profile, p_resource, p_kind, p_intent, p_include_folded) va
      JOIN kb_data_artifacts a ON a.id = va.id
      LEFT JOIN kb_data_artifact_content c ON c.artifact_id = a.id
     ORDER BY a.created, a.id
$$;

-- Counts only: no content hydration. Grouped by the qualified family name so a caller
-- sees "3 measurements, 1 query-plan" without fetching the payloads.
CREATE FUNCTION artifact_counts_for_resource(p_profile uuid, p_resource uuid,
                                             p_include_folded boolean DEFAULT false)
RETURNS TABLE(
    kind_owner_table   varchar,
    kind_owner_id      uuid,
    artifact_kind      text,
    count              bigint,
    total_bytes        bigint
)
LANGUAGE sql STABLE AS $$
    SELECT a.kind_owner_table,
           a.kind_owner_id,
           a.artifact_kind,
           count(*)::bigint,
           coalesce(sum(a.content_bytes), 0)::bigint
      FROM _visible_artifacts(p_profile, p_resource, NULL, NULL, p_include_folded) va
      JOIN kb_data_artifacts a ON a.id = va.id
     GROUP BY a.kind_owner_table, a.kind_owner_id, a.artifact_kind
     ORDER BY a.kind_owner_table, a.kind_owner_id, a.artifact_kind
$$;

-- IDs only: for fetch-by-id patterns where the caller wants to enumerate first and
-- hydrate later (or pass the IDs to another surface). Same visibility gate.
CREATE FUNCTION artifact_ids_for_resource(p_profile uuid, p_resource uuid,
                                          p_kind text DEFAULT NULL,
                                          p_intent text DEFAULT NULL,
                                          p_include_folded boolean DEFAULT false)
RETURNS TABLE(artifact_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT va.id
      FROM _visible_artifacts(p_profile, p_resource, p_kind, p_intent, p_include_folded) va
     ORDER BY va.id
$$;

-- Single artifact by ID: resolves the owning resource and gates on its visibility.
-- Never trusts the caller — even if the caller knows the artifact id, the owning
-- resource must be visible to the profile or the artifact is absent (fail closed).
CREATE FUNCTION artifact_by_id(p_profile uuid, p_artifact_id uuid)
RETURNS TABLE(
    artifact_id        uuid,
    resource_id        uuid,
    kind_owner_table   varchar,
    kind_owner_id      uuid,
    artifact_kind      text,
    intent             text,
    precedence         double precision,
    content_hash       text,
    content_bytes      bigint,
    shape_state        text,
    is_folded          boolean,
    created            timestamptz,
    content            jsonb
)
LANGUAGE sql STABLE AS $$
    SELECT a.id,
           a.resource_id,
           a.kind_owner_table,
           a.kind_owner_id,
           a.artifact_kind,
           a.intent,
           a.precedence,
           a.content_hash,
           a.content_bytes,
           'never_declared'::text,
           a.is_folded,
           a.created,
           c.content
      FROM kb_data_artifacts a
      JOIN resources_visible_to(p_profile) v ON v.resource_id = a.resource_id
      LEFT JOIN kb_data_artifact_content c ON c.artifact_id = a.id
     WHERE a.id = p_artifact_id
$$;

SELECT declare_migration(
    20260820000030,
    'additive',
    'Data artifact read path: _visible_artifacts (private visibility-gated helper) and four public read functions — artifacts_for_resource (full hydration), artifact_counts_for_resource (counts only, no content), artifact_ids_for_resource (IDs only for fetch-by-id), artifact_by_id (single artifact, visibility-gated on owning resource). All gate through resources_visible_to. Shape-state reports ''never_declared'' — no registry exists yet. Design: internal/superpowers/specs/2026-08-20-resource-owned-data-artifacts-design.md.'
);
