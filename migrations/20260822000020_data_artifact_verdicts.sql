-- Data-artifact verdict read-model: a per-artifact conformance verdict, keyed by the staleness
-- triple (shape_id, shape_version, content_hash), and the rewrite of the four read functions so
-- their hardcoded 'never_declared' literals become a real shape-state expression.
--
-- Design and rationale: internal/superpowers/specs/2026-08-21-data-artifact-shape-registry-design.md
-- §7.4 (disposable read-model, not event-sourced) and §7.5 (staleness must be unrepresentable as
-- conformance). Read those before changing anything here.
--
-- This is Beat C, Task 4. The table is NOT event-sourced: it is rebuildable from artifacts + shapes
-- at any time (spec §7.4). The staleness triple is the single most important line: a stored verdict
-- is trusted ONLY when all three of (shape_id, shape_version, content_hash) still match the
-- currently-governing shape and the artifact's current hash. Otherwise the artifact reports
-- declared_not_yet_checked. Where no shape is in force, never_declared. This makes
-- unchecked-never-reads-as-checked hold by construction, not by a worker running on time.

-- The verdict table. Not event-sourced. Rebuildable from artifacts + shapes at any time.
-- artifact_id is PK: one verdict per artifact (upserted on each conformance check).
CREATE TABLE kb_data_artifact_verdicts (
    artifact_id    UUID PRIMARY KEY REFERENCES kb_data_artifacts(id) ON DELETE CASCADE,
    shape_id       UUID NOT NULL REFERENCES kb_data_artifact_shapes(id) ON DELETE CASCADE,
    shape_version  INT  NOT NULL,
    content_hash   TEXT NOT NULL,
    satisfied      BOOLEAN NOT NULL,
    detail         JSONB,
    checked_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Private helper: the shape-state expression for one artifact. This is where the staleness triple
-- lives — the single most important line in this beat (spec §7.5). A stored verdict is trusted ONLY
-- when all three of (shape_id, shape_version, content_hash) still match the currently-governing
-- shape and the artifact's current hash. Otherwise declared_not_yet_checked. Where no shape is in
-- force, never_declared.
--
-- MUST call _data_artifact_shape_in_force (which calls _data_artifact_anchor and
-- _data_artifact_kind_owner) rather than re-deriving the home — the cogmap tiebreak is
-- load-bearing (20260820000020:186-188).
CREATE FUNCTION _data_artifact_shape_state(p_artifact_id uuid)
RETURNS text
LANGUAGE plpgsql STABLE AS $$
DECLARE v_resource   uuid;
        v_kind_tbl   text;
        v_kind_id    uuid;
        v_kind       text;
        v_hash       text;
        v_shape_id   uuid;
        v_shape_ver  int;
        v_satisfied  boolean;
BEGIN
    SELECT a.resource_id, a.kind_owner_table, a.kind_owner_id, a.artifact_kind, a.content_hash
      INTO v_resource, v_kind_tbl, v_kind_id, v_kind, v_hash
      FROM kb_data_artifacts a
     WHERE a.id = p_artifact_id;

    IF NOT FOUND THEN
        RETURN 'never_declared';
    END IF;

    -- Is a shape in force for this artifact's family in its home?
    SELECT s.shape_id, s.shape_version INTO v_shape_id, v_shape_ver
      FROM _data_artifact_shape_in_force(v_resource, v_kind_tbl, v_kind_id, v_kind) s;

    IF v_shape_id IS NULL THEN
        RETURN 'never_declared';
    END IF;

    -- Does a verdict exist that matches the staleness triple? ALL THREE must match.
    SELECT v.satisfied INTO v_satisfied
      FROM kb_data_artifact_verdicts v
     WHERE v.artifact_id   = p_artifact_id
       AND v.shape_id      = v_shape_id
       AND v.shape_version = v_shape_ver
       AND v.content_hash  = v_hash;

    IF NOT FOUND THEN
        RETURN 'declared_not_yet_checked';
    END IF;

    IF v_satisfied THEN
        RETURN 'declared_satisfied';
    ELSE
        RETURN 'declared_not_satisfied';
    END IF;
END;
$$;

-- Verdict upsert: called by the commit path and the reconciler to store a verdict. The staleness
-- triple is written here so the read-side _data_artifact_shape_state can match against it.
CREATE FUNCTION data_artifact_verdict_upsert(
    p_artifact_id  uuid,
    p_shape_id     uuid,
    p_shape_version int,
    p_content_hash text,
    p_satisfied    boolean,
    p_detail       jsonb DEFAULT NULL
)
RETURNS void
LANGUAGE sql AS $$
    INSERT INTO kb_data_artifact_verdicts (artifact_id, shape_id, shape_version, content_hash,
                                            satisfied, detail)
    VALUES (p_artifact_id, p_shape_id, p_shape_version, p_content_hash, p_satisfied, p_detail)
    ON CONFLICT (artifact_id) DO UPDATE
      SET shape_id      = EXCLUDED.shape_id,
          shape_version = EXCLUDED.shape_version,
          content_hash  = EXCLUDED.content_hash,
          satisfied     = EXCLUDED.satisfied,
          detail        = EXCLUDED.detail,
          checked_at    = now()
$$;

-- Rewrite the two read functions that had hardcoded 'never_declared' (lines 79 and 154 of
-- 20260820000030). The return shape is byte-identical — only the expression behind shape_state
-- changes. CREATE OR REPLACE is safe because the column list and types are unchanged.
--
-- artifacts_for_resource: full hydration.
CREATE OR REPLACE FUNCTION artifacts_for_resource(p_profile uuid, p_resource uuid,
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
           _data_artifact_shape_state(a.id),
           a.is_folded,
           a.created,
           c.content
      FROM _visible_artifacts(p_profile, p_resource, p_kind, p_intent, p_include_folded) va
      JOIN kb_data_artifacts a ON a.id = va.id
      LEFT JOIN kb_data_artifact_content c ON c.artifact_id = a.id
     ORDER BY a.created, a.id
$$;

-- artifact_by_id: single artifact, visibility-gated on owning resource.
CREATE OR REPLACE FUNCTION artifact_by_id(p_profile uuid, p_artifact_id uuid)
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
           _data_artifact_shape_state(a.id),
           a.is_folded,
           a.created,
           c.content
      FROM kb_data_artifacts a
      JOIN resources_visible_to(p_profile) v ON v.resource_id = a.resource_id
      LEFT JOIN kb_data_artifact_content c ON c.artifact_id = a.id
     WHERE a.id = p_artifact_id
$$;

SELECT declare_migration(
    20260822000020,
    'additive',
    'Data-artifact verdict read-model (Beat C, Task 4): kb_data_artifact_verdicts (not event-sourced, rebuildable from artifacts + shapes), _data_artifact_shape_state (the staleness triple — a stored verdict is trusted ONLY when shape_id + shape_version + content_hash all match the currently-governing shape and the artifact''s current hash; otherwise declared_not_yet_checked; where no shape is in force, never_declared), data_artifact_verdict_upsert (called by the commit path and reconciler), and CREATE OR REPLACE of artifacts_for_resource and artifact_by_id so their hardcoded ''never_declared'' literals become the real _data_artifact_shape_state expression. Return shapes are byte-identical — only the shape_state expression changes, so the migration is additive. The two count/ID read functions (artifact_counts_for_resource, artifact_ids_for_resource) do not carry shape_state and are unchanged. Design: internal/superpowers/specs/2026-08-21-data-artifact-shape-registry-design.md §7.4–§7.5.'
);
