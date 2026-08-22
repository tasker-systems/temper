-- Data-artifact shape registry: a declarable JSON Schema per data-artifact family, homed
-- polymorphically over (kb_contexts, kb_cogmaps) and keyed per home so a shape never verdicts data
-- its declarer cannot read.
--
-- Design and rationale: internal/superpowers/specs/2026-08-21-data-artifact-shape-registry-design.md.
-- Read that before changing anything here — the reasoning for the shape is there, not in this file.
--
-- This is the registry substrate (Beat A, Task 1): the table, the shape_declared event type, the
-- shape-in-force resolver, and the declare act. Commit-time conformance verdicts (Beat B) and the
-- verdict read-model (Beat C) are separate migrations.

-- The registry table. Assert/fold, as kb_properties, kb_edges, and kb_data_artifacts: revision IS
-- the folded chain, and a mutable `revised` column would be the one non-payload-derivable column.
-- The column-comment density follows kb_data_artifacts (20260820000020:14-45) deliberately.
CREATE TABLE kb_data_artifact_shapes (
    -- No DEFAULT, deliberately: the id is minted by the caller and carried in the payload
    -- (identity-as-input), so a server-side default would mint a different id on replay.
    id                    UUID PRIMARY KEY,
    -- The home the shape is declared in — polymorphic over (kb_contexts, kb_cogmaps), the same
    -- pair _data_artifact_anchor resolves. A shape never verdicts data its declarer cannot read,
    -- because the home bounds the reach (ruling 2). The column pair follows the precedent at
    -- 20260712000030_region_anchor_expand.sql:14-17.
    home_anchor_table     VARCHAR(64) NOT NULL CHECK (home_anchor_table IN ('kb_contexts','kb_cogmaps')),
    home_anchor_id        UUID NOT NULL,
    -- The namespace half of the family name: (kind_owner, artifact_kind), never the bare name.
    -- Defaulted from the home by _data_artifact_kind_owner when the caller omits it, the same
    -- defaulting data_artifact_commit already does. A team is not a visibility boundary, so the
    -- key is per HOME not per owner — see the partial unique index below.
    kind_owner_table      VARCHAR(64) NOT NULL
                              CHECK (kind_owner_table IN ('kb_profiles', 'kb_teams')),
    kind_owner_id         UUID NOT NULL,
    artifact_kind         TEXT NOT NULL,
    -- The JSON Schema (draft 2020-12) governing this family. Validated Rust-side — there is no
    -- in-database JSON Schema validator, as the incumbent registry also does not (spec §7.2).
    schema                JSONB NOT NULL,
    -- The closed enforcement vocabulary (spec §6). The CHECK constraint AND the wrapper's refusal
    -- both name this set; keep the literals pinned to payloads::EnforcementMode's serde renames.
    enforcement           TEXT NOT NULL CHECK (enforcement IN ('advisory','enforcing')),
    -- The chain depth of the assert/fold lineage — 1 for the first declaration, N for the Nth
    -- amendment. Computed by the wrapper from the folded lineage, not a mutable counter.
    shape_version         INT  NOT NULL,
    asserted_by_event_id  UUID NOT NULL REFERENCES kb_events(id),
    last_event_id         UUID NOT NULL REFERENCES kb_events(id),
    is_folded             BOOLEAN NOT NULL DEFAULT false,
    created               TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The partial UNIQUE index is the whole ruling (spec §4): a shape in force is SINGULAR per family
-- per home. The `WHERE NOT is_folded` clause is what makes assert/fold work — a folded row vacates
-- the slot and the new declaration takes it, so there is at most one live shape per family per
-- home at any time.
--
-- NOTE THE DELIBERATE CONTRAST with kb_data_artifacts (20260820000020:36-41), which carries NO such
-- index and says so: artifacts are a has-many collection; only the writer knows whether one replaces
-- another. A shape in force is singular. Adding this index to kb_data_artifacts would break the
-- design; removing it from here would break ruling 2.
CREATE UNIQUE INDEX uq_kb_data_artifact_shapes_live
    ON kb_data_artifact_shapes (home_anchor_table, home_anchor_id, kind_owner_table, kind_owner_id, artifact_kind)
    WHERE NOT is_folded;

-- TYPED, with a published payload_schema (the committed schemars snapshot,
-- crates/temper-substrate/tests/fixtures/payloads/data_artifact_shape_declared.v1.schema.json,
-- verbatim: repo == registry == Rust types). Regenerate both halves together with
--   UPDATE_SCHEMA=1 cargo make test-schema
-- and paste the result here; hand-editing either half breaks the chain silently.
--
-- category is spelled explicitly because 20260719000010 dropped the column DEFAULT so an omitting
-- registration fails loudly rather than silently joining the trail allowlist.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('shape_declared', $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ShapeDeclared",
  "description": "Declare a shape for a data-artifact family within one home (spec: data-artifact shape registry,\n2026-08-21, §3–§4).\n\nThe shape is **homed polymorphically** over `(kb_contexts, kb_cogmaps)` and **keyed per home**,\nnot per owner: a shape declared in a context you cannot read must not verdict your data. The\n`(home_anchor, kind_owner, artifact_kind)` triple is unique among non-folded rows, so the\nshape in force is singular per family per home. Amending a shape folds the prior row and inserts\na new one; `shape_version` is the chain depth (assert/fold, the same revision pattern\n`kb_data_artifacts` keeps).\n\nThe `schema` is a JSON Schema (draft 2020-12) validated Rust-side — there is no in-database\nvalidator. `enforcement` carries the closed vocabulary the register closes over.",
  "type": "object",
  "properties": {
    "artifact_kind": {
      "description": "The bare family name, qualified by `kind_owner`. Carries no implication that a shape has been\nregistered — persistence never requires a prior declaration.",
      "type": "string"
    },
    "enforcement": {
      "description": "Whether a non-conforming commit is refused (`enforcing`) or merely recorded (`advisory`).",
      "$ref": "#/$defs/EnforcementMode"
    },
    "home_anchor": {
      "description": "The home the shape is declared in — polymorphic over `(kb_contexts, kb_cogmaps)`. A shape\nnever verdicts data its declarer cannot read, because the home bounds the reach.",
      "$ref": "#/$defs/AnchorRef"
    },
    "kind_owner": {
      "description": "The namespace half of the family name, `(kind_owner, artifact_kind)`. Resolved by the SQL\nwrapper from the home when the caller names none (the same defaulting\n`_data_artifact_kind_owner` already does for commits), so replay never re-derives a\nnamespace.",
      "$ref": "#/$defs/KindOwner"
    },
    "schema": {
      "description": "The JSON Schema (draft 2020-12) governing this family. Validated Rust-side."
    },
    "shape_id": {
      "description": "Identity-as-input: minted by the caller and carried here so replay reproduces the same row\nid. `kb_data_artifact_shapes.id` deliberately has no DEFAULT for this reason.",
      "$ref": "#/$defs/ShapeId"
    },
    "shape_version": {
      "description": "The chain depth of the assert/fold lineage — 1 for the first declaration, N for the Nth\namendment. A verdict recorded against a folded version stops matching, and the artifact\nreads as unchecked until reconciled.",
      "type": "integer",
      "format": "int64"
    }
  },
  "required": [
    "shape_id",
    "home_anchor",
    "kind_owner",
    "artifact_kind",
    "schema",
    "enforcement",
    "shape_version"
  ],
  "$defs": {
    "AnchorRef": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string",
          "format": "uuid"
        },
        "table": {
          "$ref": "#/$defs/AnchorTable"
        }
      },
      "required": [
        "table",
        "id"
      ]
    },
    "AnchorTable": {
      "description": "A polymorphic anchor/endpoint reference. Serializes table names exactly as the DDL spells them.",
      "type": "string",
      "enum": [
        "kb_contexts",
        "kb_cogmaps",
        "kb_resources",
        "kb_edges",
        "kb_content_blocks",
        "kb_teams",
        "kb_profiles",
        "kb_connections",
        "kb_machine_clients",
        "kb_events"
      ]
    },
    "EnforcementMode": {
      "description": "The closed enforcement vocabulary (spec §6). A shape is `advisory` by default: a non-conforming\ncommit is recorded, never refused. An `enforcing` shape refuses a non-conforming commit and the\nrefusal carries what failed. The SQL CHECK constraint and the wrapper's refusal both name this\nset; keep the `serde` renames pinned to the CHECK.",
      "oneOf": [
        {
          "description": "Default. A non-conforming commit succeeds and is recorded as non-conforming.",
          "type": "string",
          "const": "advisory"
        },
        {
          "description": "A non-conforming commit is refused, and the refusal carries what failed.",
          "type": "string",
          "const": "enforcing"
        }
      ]
    },
    "KindOwner": {
      "description": "The owner whose namespace a family name lives in. A bare `\"query-plan\"` is never a complete\nreference; the pair `(kind_owner, artifact_kind)` is.\n\nRegistering a shape for a family validates the existing backlog and records a conformance\nverdict against every artifact of that family — so under a flat namespace one tenant's\nregistration would stamp verdicts on another tenant's data, which it cannot even read.\nQualification makes that impossible by construction rather than by resolution rule.",
      "oneOf": [
        {
          "type": "object",
          "properties": {
            "kb_profiles": {
              "type": "string",
              "format": "uuid"
            }
          },
          "additionalProperties": false,
          "required": [
            "kb_profiles"
          ]
        },
        {
          "type": "object",
          "properties": {
            "kb_teams": {
              "type": "string",
              "format": "uuid"
            }
          },
          "additionalProperties": false,
          "required": [
            "kb_teams"
          ]
        }
      ]
    },
    "ShapeId": {
      "description": "A `kb_data_artifact_shapes.id` value — one declared JSON Schema governing a data-artifact\nfamily within a single home (spec: data-artifact shape registry, 2026-08-21).",
      "type": "string",
      "format": "uuid"
    }
  }
}$JS$::jsonb, 1, 'domain')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema,
      schema_version = EXCLUDED.schema_version,
      category       = EXCLUDED.category;

-- The shape in force for a resource's family. MUST CALL _data_artifact_anchor and
-- _data_artifact_kind_owner rather than restating them — the cogmap tiebreak in _data_artifact_anchor
-- is load-bearing (carried verbatim from _property_owner_anchor), and re-deriving it is how the two
-- drift (20260820000020:186-188). The home bounds the reach: a shape declared in a context you
-- cannot read must not verdict your data (ruling 2).
--
-- Returns at most one row: (shape_id, shape_version, schema, enforcement). Returns no row when no
-- live shape is in force for the resource's family in its home.
CREATE FUNCTION _data_artifact_shape_in_force(p_resource uuid,
                                              p_kind_owner_table text,
                                              p_kind_owner_id uuid,
                                              p_kind text)
RETURNS TABLE (shape_id uuid, shape_version int, schema jsonb, enforcement text)
LANGUAGE plpgsql STABLE AS $$
DECLARE v_anchor_tbl text; v_anchor uuid;
BEGIN
    -- Resolve the home via the incumbent resolver. Re-deriving it here is exactly the drift
    -- 20260820000020:186-188 warns about.
    SELECT a.anchor_table, a.anchor_id INTO v_anchor_tbl, v_anchor
      FROM _data_artifact_anchor(p_resource) a;

    RETURN QUERY
      SELECT s.id, s.shape_version, s.schema, s.enforcement
        FROM kb_data_artifact_shapes s
       WHERE s.home_anchor_table = v_anchor_tbl
         AND s.home_anchor_id    = v_anchor
         AND s.kind_owner_table  = p_kind_owner_table
         AND s.kind_owner_id     = p_kind_owner_id
         AND s.artifact_kind     = p_kind
         AND NOT s.is_folded
       LIMIT 1;
END;
$$;

-- Reads ONLY the payload. Assert/fold: fold the prior live row for the same family in the same
-- home, then insert the new row. The version is the chain depth (COUNT of prior rows + 1), not a
-- mutable counter — so a verdict recorded against a folded version stops matching, and the
-- artifact reads as unchecked until reconciled (spec §7.5).
CREATE FUNCTION _project_data_artifact_shape_declared(p_event uuid, p_payload jsonb)
RETURNS uuid[]
LANGUAGE plpgsql AS $$
DECLARE v_id           uuid := (p_payload->>'shape_id')::uuid;
        v_occurred    timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_home_tbl    text := p_payload->>'home_anchor_table';
        v_home_id     uuid := (p_payload->>'home_anchor_id')::uuid;
        v_kind_tbl    text := p_payload->>'kind_owner_table';
        v_kind_own    uuid := (p_payload->>'kind_owner_id')::uuid;
        v_kind        text := p_payload->>'artifact_kind';
        v_schema      jsonb := p_payload->'schema';
        v_enforcement text := p_payload->>'enforcement';
        v_version     int;
BEGIN
    -- Compute the chain depth BEFORE folding, so the new row's version is the prior count + 1.
    -- This makes shape_version = the position in the lineage (1, 2, 3, ...), not a mutable counter.
    SELECT count(*) + 1 INTO v_version
      FROM kb_data_artifact_shapes
     WHERE home_anchor_table = v_home_tbl
       AND home_anchor_id    = v_home_id
       AND kind_owner_table  = v_kind_tbl
       AND kind_owner_id     = v_kind_own
       AND artifact_kind     = v_kind;

    -- Fold the prior live row, if any. The partial UNIQUE index on `WHERE NOT is_folded` ensures
    -- there is at most one to fold. A first declaration folds nothing.
    UPDATE kb_data_artifact_shapes
       SET is_folded = true, last_event_id = p_event
     WHERE home_anchor_table = v_home_tbl
       AND home_anchor_id    = v_home_id
       AND kind_owner_table  = v_kind_tbl
       AND kind_owner_id     = v_kind_own
       AND artifact_kind     = v_kind
       AND NOT is_folded;

    INSERT INTO kb_data_artifact_shapes (id, home_anchor_table, home_anchor_id, kind_owner_table,
                                          kind_owner_id, artifact_kind, schema, enforcement,
                                          shape_version, asserted_by_event_id, last_event_id, created)
    VALUES (v_id, v_home_tbl, v_home_id, v_kind_tbl, v_kind_own, v_kind, v_schema, v_enforcement,
            v_version, p_event, p_event, v_occurred);

    RETURN ARRAY[v_id];
END;
$$;

-- Wrapper: validate, resolve anchor, append event, project. Same four moves as
-- data_artifact_commit (20260820000020). The refusal for an unrecognized enforcement term NAMES THE
-- VOCABULARY (goal clause `a-declined-act-teaches-its-vocabulary`): the CHECK constraint alone is
-- not enough — its message names a constraint rather than the vocabulary, and the caller learning
-- the answer is the point of the refusal.
CREATE FUNCTION data_artifact_shape_declare(p_payload jsonb, p_emitter uuid,
                                             p_metadata jsonb DEFAULT '{}'::jsonb,
                                             p_invocation uuid DEFAULT NULL::uuid,
                                             p_correlation uuid DEFAULT NULL::uuid)
RETURNS uuid[]
LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_home_tbl text := p_payload->>'home_anchor_table';
        v_home_id  uuid := (p_payload->>'home_anchor_id')::uuid;
        v_kind_tbl text; v_kind_own uuid;
        v_enforcement text := p_payload->>'enforcement';
        v_version int;
BEGIN
    -- The refusal that teaches its vocabulary (goal clause `a-declined-act-teaches-its-vocabulary`).
    -- The CHECK constraint would reject this anyway, but its message names a constraint rather than
    -- the vocabulary, and the caller learning the answer is the point of the refusal.
    IF v_enforcement IS NULL OR v_enforcement NOT IN ('advisory', 'enforcing') THEN
        RAISE EXCEPTION 'data_artifact_shape_declare: unrecognized enforcement %; the vocabulary is '
                        'advisory (default: a non-conforming commit is recorded, never refused), '
                        'enforcing (a non-conforming commit is refused and the refusal carries what failed)',
                        COALESCE(v_enforcement, '<null>');
    END IF;

    -- Validate the home anchor pair is present and names a valid home table.
    IF v_home_tbl IS NULL OR v_home_id IS NULL THEN
        RAISE EXCEPTION 'data_artifact_shape_declare: home_anchor (table, id) is required — a shape '
                        'is homed and keyed per home';
    END IF;

    IF v_home_tbl NOT IN ('kb_contexts', 'kb_cogmaps') THEN
        RAISE EXCEPTION 'data_artifact_shape_declare: unrecognized home_anchor_table %; the home '
                        'vocabulary is kb_contexts, kb_cogmaps',
                        COALESCE(v_home_tbl, '<null>');
    END IF;

    -- The schema must be present (a shape with no schema is not a shape).
    IF p_payload->'schema' IS NULL THEN
        RAISE EXCEPTION 'data_artifact_shape_declare: schema is required — a shape declares a JSON '
                        'Schema (draft 2020-12) governing its family';
    END IF;

    v_anchor_tbl := v_home_tbl;
    v_anchor    := v_home_id;

    -- Default the kind namespace INTO THE PAYLOAD, before the event is appended, exactly as
    -- data_artifact_commit does. Resolving it at projection time would re-read kb_contexts during
    -- replay — and a context's owner can change (context_reassigned), so replay would qualify an
    -- old shape with today's owner and the byte-diff would fail. Identity-as-input applies to the
    -- namespace too.
    IF p_payload->>'kind_owner_id' IS NULL THEN
        -- Resolve the default namespace from a RESOURCE homed in the declaring home. We need a
        -- resource to call _data_artifact_kind_owner; use any resource homed in this home.
        SELECT r.id INTO v_kind_own
          FROM kb_resources r
          JOIN kb_resource_homes h ON h.resource_id = r.id
         WHERE h.anchor_table = v_home_tbl AND h.anchor_id = v_home_id
         LIMIT 1;

        IF v_kind_own IS NULL THEN
            RAISE EXCEPTION 'data_artifact_shape_declare: no kind_owner named and no resource homed '
                            'in % to default from — name kind_owner explicitly', v_home_tbl;
        END IF;

        SELECT k.owner_table, k.owner_id INTO v_kind_tbl, v_kind_own
          FROM _data_artifact_kind_owner(v_kind_own) k;
        p_payload := p_payload
                   || jsonb_build_object('kind_owner_table', v_kind_tbl,
                                         'kind_owner_id',    v_kind_own);
    END IF;

    -- Compute the chain depth INTO THE PAYLOAD, before the event is appended, so the stored payload
    -- always carries the resolved version and replay never has to re-derive it.
    SELECT count(*) + 1 INTO v_version
      FROM kb_data_artifact_shapes
     WHERE home_anchor_table = v_home_tbl
       AND home_anchor_id    = v_home_id
       AND kind_owner_table  = p_payload->>'kind_owner_table'
       AND kind_owner_id     = (p_payload->>'kind_owner_id')::uuid
       AND artifact_kind     = p_payload->>'artifact_kind';
    p_payload := p_payload || jsonb_build_object('shape_version', v_version);

    v_ev := _event_append('shape_declared', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_data_artifact_shape_declared(v_ev, p_payload);
END;
$$;

SELECT declare_migration(
    20260822000010,
    'additive',
    'Data-artifact shape registry substrate (Beat A, Task 1): kb_data_artifact_shapes (homed polymorphically over kb_contexts/kb_cogmaps, keyed per home with a partial UNIQUE index WHERE NOT is_folded so the shape in force is singular per family per home — the deliberate contrast with kb_data_artifacts, which is a has-many collection with no such index), the shape_declared event type (domain, TYPED with the committed schemars payload_schema), the _data_artifact_shape_in_force resolver (which calls _data_artifact_anchor and _data_artifact_kind_owner rather than re-deriving them, so the cogmap tiebreak never drifts), and the _project_data_artifact_shape_declared / data_artifact_shape_declare projector+wrapper pair (validate, resolve anchor, append event, project — the same four moves as data_artifact_commit; the refusal for an unrecognized enforcement term names the vocabulary, as a-declined-act-teaches-its-vocabulary requires). Assert/fold revision: amending a shape folds the prior row and inserts a new one; shape_version is the chain depth. No conformance verdict yet — Beat B (Task 3) adds commit-time validation, and Beat C (Task 4) adds the verdict read-model. Design: internal/superpowers/specs/2026-08-21-data-artifact-shape-registry-design.md.'
);