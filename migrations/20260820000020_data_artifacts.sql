-- Migration: resource-owned data artifacts — the storage substrate and its write path.
--
-- Design: internal/superpowers/specs/2026-08-20-resource-owned-data-artifacts-design.md
-- Goal:   01a02163-ba6a-7b00-91f5-5f416e43f4f6
-- Task:   01a02163-faba-7a71-b09a-45eade04baba  (Beat A)
--
-- ── Why this exists (measured, not argued, 2026-08-20) ──────────────────────────────────────────
-- Agents already persist structured data in temper — JSON and YAML written into fenced code blocks
-- inside resource bodies, read back by a LATER, unrelated agent session to ground its next stage.
-- The practice is load-bearing and has no home. Three failures of it were measured:
--
--   1. kb_properties (and therefore open_meta) REJECTS anything over 2704 bytes. uq_kb_properties_active
--      is a btree over (owner_table, owner_id, property_key, property_value):
--          index row size 3648 exceeds btree version 4 maximum 2704
--      A minimal query composition is ~450 bytes; one carrying a 768-float embedding is ~15KB.
--   2. Resource bodies SHRED fenced data. temper-ingest's collect_sections_with_stack applies
--      heading_re() (^(#{1,6})\s+(.+)$) with no fence-state tracking, so a YAML comment at column 0
--      is parsed as a markdown heading. An executed probe split a 12-line YAML fence into three
--      chunks, promoted the comment text into header_path, and orphaned both fence delimiters onto
--      neighbouring prose. MAX_CHARS ~= 1428 splits anything larger regardless of comments.
--   3. Every fragment is then embedded into a corpus built for prose.
--
-- ── The load-bearing constraint ─────────────────────────────────────────────────────────────────
-- Data artifacts are NOT queryable and are never made queryable. Their relationship to resources,
-- edges and properties is what makes them FINDABLE; the graph is the index and the artifact is the
-- payload at the end of it. kb_properties remains the key-value JSONB store with predicates and
-- facets — artifacts deliberately do not compete with it. Nothing here is ever searched or embedded.

-- ════════════════════════════════════════════════════════════════════════════════════════════════
-- A1 — the metadata/bytes split
-- ════════════════════════════════════════════════════════════════════════════════════════════════
--
-- Shaped after kb_block_content (20260714000002_block_content_verbatim.sql): the METADATA row holds
-- everything derivable from the event payload, and the BYTES live in a companion table keyed by the
-- artifact, carrying the content and its hash.
--
-- This split is what keeps replay honest WITHOUT inventing a new replay category (the design spec's
-- Replay section supersedes an earlier draft that wrongly claimed one was needed). Every column of
-- kb_data_artifacts is payload-derivable, so the table joins PROJECTION_DUMPS and diffs
-- byte-identically like any other projection. The bytes ride the sidecar and are proved by hash —
-- exactly as kb_block_content already does ("Re-supply the __blocks sidecar (verbatim block bytes,
-- PR 3) from kb_block_content", replay.rs:265).
--
-- The event payload therefore carries the HASH, never the body. Replay's purpose for the ledger is
-- PROVENANCE: resources are the replayable-difference core; artifacts are event-sourced for
-- governance and consistency. Consequence: the ledger stays light regardless of artifact size, and
-- replay fidelity imposes no size ceiling.
CREATE TABLE kb_data_artifacts (
    id                    UUID PRIMARY KEY,   -- identity-as-input: minted by the caller, carried in
                                              -- the payload, so replay reproduces it. Deliberately
                                              -- NO DEFAULT — a server-side default would mint a
                                              -- different id on replay and break the byte-diff.
                                              -- (_project_property_set reads property_id the same way.)
    resource_id           UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    -- The family this datum belongs to, as an OWNER-QUALIFIED name. A bare "query-plan" is never a
    -- complete reference: the pair (kind_owner, artifact_kind) is.
    --
    -- WHY QUALIFICATION IS STRUCTURAL AND NOT A LATER REFINEMENT. Registering a shape for a family
    -- does not merely describe it — it VALIDATES THE EXISTING BACKLOG and records a conformance
    -- verdict against every artifact of that family. With a flat global namespace, one tenant
    -- registering `query-plan` would stamp verdicts onto another tenant's `query-plan` artifacts —
    -- data it cannot even read. That is a principal writing outside its reach, not a naming
    -- inconvenience, and it is why the doc-type precedent (global, unscoped, deliberately so) does
    -- NOT transfer: a doc type is an inert label, and registering one writes nothing to anyone's
    -- data.
    --
    -- Artifacts are therefore BORN qualified. Adding the qualifier later would require backfilling
    -- an owner onto existing rows, and there is no correct value to backfill.
    --
    -- The qualifier is DEFAULTED, not demanded: the wrapper resolves it from the owning resource's
    -- home when the caller omits it (see _data_artifact_kind_owner). An agent writes "query-plan"
    -- and gets its own namespace without saying so; naming another owner's namespace is possible
    -- and explicit. This keeps the anti-friction property that the whole write-first/bind-later
    -- posture exists to protect.
    kind_owner_table      VARCHAR(64) NOT NULL
                              CHECK (kind_owner_table IN ('kb_profiles', 'kb_teams')),
    kind_owner_id         UUID NOT NULL,
    artifact_kind         TEXT NOT NULL,
    -- A7. The closed selection vocabulary. It answers exactly one question a reader cannot function
    -- without: given a collection, which do I take?
    --   current — replaces earlier artifacts of its kind. Take the newest live `current`.
    --   member  — a peer in a series, NOT a replacement. Take them all, ordered by precedence.
    --   pinned  — never auto-selected; addressable by explicit reference only.
    -- Enforced HERE and not only at the service edge, so a path that bypasses the service layer
    -- still cannot store a term outside the vocabulary.
    intent                TEXT NOT NULL CHECK (intent IN ('current', 'member', 'pinned')),
    -- Ordering among peers. Meaningful for `member`; carried for all so a reader never has to
    -- branch on intent to sort.
    precedence            DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    -- Bare sha256 hex of the content's raw bytes — the Rust `sha256_hex` twin, exactly as
    -- kb_block_content.content_hash. This is what the event payload carries in place of the body.
    content_hash          TEXT NOT NULL,
    content_bytes         BIGINT NOT NULL,    -- surfaced so a reader can decide whether to fetch
    -- Assert/fold, the incumbent trio (kb_properties, kb_edges, kb_content_blocks). Rows are never
    -- UPDATEd in place and never DELETEd: a revision folds the prior row and inserts a new one.
    --
    -- REVISION IS THE FOLDED CHAIN. There is deliberately no mutable `revised` column: `revised` is
    -- INFERRED from history on read. A mutable timestamp would be the one field in this table that
    -- is not payload-derivable, which would cost the byte-exact replay diff for nothing. (Decided
    -- with the frame owner, 2026-08-20.)
    asserted_by_event_id  UUID NOT NULL REFERENCES kb_events(id),
    last_event_id         UUID NOT NULL REFERENCES kb_events(id),
    is_folded             BOOLEAN NOT NULL DEFAULT false,
    created               TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ── A2 — the uniqueness index that is DELIBERATELY ABSENT ───────────────────────────────────────
--
-- Every sibling assert/fold table carries a partial unique index over its live rows
-- (uq_kb_properties_active, uq_kb_edges_assertion). THIS TABLE HAS NONE, ON PURPOSE.
--
-- A resource owns a HAS-MANY collection of artifacts. There is no one-live-per-kind rule because
-- THE STORE CANNOT KNOW WHETHER RUN #2 SUPERSEDES RUN #1 — only the writer can. A measurement run
-- does not replace the run before it; a recomputed extraction does. Encoding either as a uniqueness
-- constraint would make the store assert a relationship it has no basis for, so ordering, precedence
-- and replacement are things the artifact DECLARES about itself (see `intent` above), never things
-- an index decides on its behalf.
--
-- This is the goal clause `no-supersession-is-asserted-that-a-writer-did-not-declare`. If you are
-- reading this because a missing unique index looked like an oversight: it is not. Adding one here
-- breaks the design.
CREATE INDEX idx_kb_data_artifacts_resource ON kb_data_artifacts(resource_id) WHERE NOT is_folded;
-- Keyed on the QUALIFIED name — a lookup by bare `artifact_kind` would silently span namespaces.
CREATE INDEX idx_kb_data_artifacts_kind     ON kb_data_artifacts(resource_id, kind_owner_table, kind_owner_id, artifact_kind) WHERE NOT is_folded;

-- The bytes. Keyed by artifact id (1:1), cascading with it — the kb_block_content shape.
CREATE TABLE kb_data_artifact_content (
    artifact_id   UUID PRIMARY KEY REFERENCES kb_data_artifacts(id) ON DELETE CASCADE,
    content       JSONB NOT NULL,
    content_hash  TEXT  NOT NULL   -- bare sha256 hex of content's raw bytes (Rust `sha256_hex` twin)
);

-- NO GIN INDEX ON content, DELIBERATELY.
--
-- kb_properties carries `USING gin (property_value jsonb_path_ops)` because it EXISTS to be queried.
-- This column exists NOT to be queried: the goal clause is
-- `structured-data-is-never-found-by-resemblance` — an artifact is reached only through what points
-- at it. A GIN index here would be the first step toward a query surface the design has committed
-- not to have, and it would be added by someone who thought they were helping.

-- ════════════════════════════════════════════════════════════════════════════════════════════════
-- A3 — the event type
-- ════════════════════════════════════════════════════════════════════════════════════════════════
--
-- _event_append hard-fails on an unseeded type ("event_type % not seeded"), so registration is a
-- precondition of the write path, not a nicety.
--
-- category='domain', spelled explicitly. 20260719000010 dropped the column DEFAULT precisely so an
-- omitting registration fails loudly (NOT NULL) rather than silently joining the trail allowlist.
-- 'domain' is correct: committing an artifact is an ordinary knowledge-graph mutation, not admin
-- action and not system infra.
--
-- TYPED, with a published payload_schema — the subscription_delivery_disposed shape
-- (20260819000030), not the permissive webhook_received one. Committing an artifact is temper's own
-- act with a shape temper controls, so the permissive path does not apply.
--
-- The literal below is the committed schemars snapshot,
-- crates/temper-substrate/tests/fixtures/payloads/data_artifact_committed.v1.schema.json, verbatim.
-- repo == registry == Rust types (payload spec §6); tests/payload_schema.rs pins repo == types and
-- this INSERT pins registry == repo. Regenerate both together with
--   UPDATE_SCHEMA=1 cargo make test-schema
-- and paste the result here; a hand-edit of either half breaks the chain silently.
--
-- NOTE the payload has no content field, deliberately: the bytes ride data_artifact_commit's
-- p_content argument and land in kb_data_artifact_content. A schema that admitted a body would
-- describe a ledger this design does not want.
--
-- There is deliberately NO `data_artifact_folded` type. An earlier draft registered one before the
-- write path existed; folding turned out to be a clause of the commit act (the writer names what it
-- supersedes) rather than an act of its own, so the type had no emitter. Per the incumbent
-- discipline, an unused arm is state we do not need.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('data_artifact_committed', $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "DataArtifactCommitted",
  "description": "Commit one data artifact to a resource (spec: resource-owned data artifacts, 2026-08-20).\n\n**The body is NOT here, and that is the design.** The payload carries `content_hash`; the bytes\nride a sidecar into `kb_data_artifact_content` and are proved by that hash — the same shape\n`kb_block_content` already uses, where replay re-supplies verbatim bytes rather than\nreconstructing them from an event. Replay's purpose for the ledger is *provenance*: resources\nare the replayable-difference core, and artifacts are event-sourced for governance and\nconsistency. Consequence: the ledger stays light regardless of artifact size.",
  "type": "object",
  "properties": {
    "artifact_id": {
      "description": "Identity-as-input: minted by the caller and carried here so replay reproduces the same row\nid. `kb_data_artifacts.id` deliberately has no DEFAULT for this reason.",
      "$ref": "#/$defs/DataArtifactId"
    },
    "artifact_kind": {
      "description": "The bare family name, qualified by `kind_owner`. Carries no implication that a shape has\nbeen registered — persistence never requires a prior declaration.",
      "type": "string"
    },
    "content_bytes": {
      "type": "integer",
      "format": "int64"
    },
    "content_hash": {
      "description": "Bare sha256 hex of the content's raw bytes — the `sha256_hex` twin, exactly as\n`kb_block_content.content_hash`.",
      "type": "string"
    },
    "intent": {
      "$ref": "#/$defs/ArtifactIntent"
    },
    "kind_owner": {
      "description": "The namespace half of the family name. Resolved at COMMIT (defaulting from the owning\nresource's home) rather than at projection: a context's owner can change, so re-resolving\nduring replay would qualify an old artifact with today's owner and break the byte-exact\ndiff. Identity-as-input applies to the namespace too.",
      "$ref": "#/$defs/KindOwner"
    },
    "precedence": {
      "description": "Ordering among peers. Meaningful for `Member`; carried for all so a reader never has to\nbranch on intent to sort.",
      "type": "number",
      "format": "double"
    },
    "resource_id": {
      "$ref": "#/$defs/ResourceId"
    },
    "supersedes": {
      "description": "The artifacts THIS one replaces, named explicitly by the writer.\n\nEmpty is the common case and means \"this replaces nothing\". The store never infers\nsupersession from recency, ordering or family: it cannot know whether run #2 supersedes run\n#1, and only the writer can. This field is that knowledge, made explicit.",
      "type": "array",
      "items": {
        "$ref": "#/$defs/DataArtifactId"
      }
    }
  },
  "required": [
    "artifact_id",
    "resource_id",
    "kind_owner",
    "artifact_kind",
    "intent",
    "precedence",
    "content_hash",
    "content_bytes"
  ],
  "$defs": {
    "ArtifactIntent": {
      "description": "How a reader should select among a resource's artifacts of one family. The **closed** selection\nvocabulary, and the only question a reader cannot function without: given a collection, which do\nI take?\n\nIt deliberately carries no ordering (that is `precedence`) and no timing (that is the row's\n`created` and its folded chain). Mixing those in was the earlier draft's mistake — an intent\nthat also implied an order would have two ways to say the same thing and they would disagree.\n\nThe SQL side enforces the same three terms with a CHECK constraint, so a caller that bypasses\nthis type still cannot store a fourth. Keep the `serde` renames pinned to that CHECK.",
      "oneOf": [
        {
          "description": "Replaces earlier artifacts of its family. A reader takes the newest live `Current`.\n\nNote what this does NOT do: declaring `Current` does not itself fold anything. The writer\nstill names what it supersedes, because the store cannot know whether this run replaces the\nlast one — see `DataArtifactCommitted::supersedes`.",
          "type": "string",
          "const": "current"
        },
        {
          "description": "A peer in a series, not a replacement. A reader takes them all, ordered by `precedence`, and\ncompares. Measurement runs live here.",
          "type": "string",
          "const": "member"
        },
        {
          "description": "Never auto-selected. Addressable by explicit reference only.",
          "type": "string",
          "const": "pinned"
        }
      ]
    },
    "DataArtifactId": {
      "description": "A `kb_data_artifacts.id` value — one schema-boundable structured datum owned by a resource.",
      "type": "string",
      "format": "uuid"
    },
    "KindOwner": {
      "description": "The owner whose namespace a family name lives in. A bare `\"query-plan\"` is never a complete\nreference; the pair `(kind_owner, artifact_kind)` is.\n\nRegistering a shape for a family validates the existing backlog and records a conformance\nverdict against every artifact of that family — so under a flat namespace one tenant's\nregistration would stamp verdicts on another tenant's data, which it cannot even read.\nQualification makes that impossible by construction rather than by a resolution rule.",
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
    "ResourceId": {
      "description": "A `kb_resources.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}$JS$::jsonb, 1, 'domain')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema,
      schema_version = EXCLUDED.schema_version,
      category       = EXCLUDED.category;

-- ════════════════════════════════════════════════════════════════════════════════════════════════
-- A4 — anchor resolution
-- ════════════════════════════════════════════════════════════════════════════════════════════════
--
-- Artifact events are homed, exactly as property and edge events are: the producing anchor gates
-- nothing by itself (every homed object carries its own gating) but it is the event's provenance and
-- must be resolvable or the write is meaningless.
--
-- An artifact has no home of its own — it inherits its owning resource's. This arm is carried
-- VERBATIM from _property_owner_anchor's kb_resources arm (20260727000030_edge_owned_properties.sql),
-- including the tiebreak, which is load-bearing: a resource homed in BOTH a context and a cogmap
-- anchors on the cogmap. Re-deriving that ordering rather than reusing it is how the two drift.
CREATE FUNCTION _data_artifact_anchor(p_resource uuid,
                                      OUT anchor_table text, OUT anchor_id uuid)
LANGUAGE plpgsql STABLE AS $$
BEGIN
    SELECT h.anchor_table, h.anchor_id INTO anchor_table, anchor_id
      FROM kb_resource_homes h
     WHERE h.resource_id = p_resource
     ORDER BY (h.anchor_table = 'kb_cogmaps') DESC
     LIMIT 1;

    IF anchor_table IS NULL THEN
        RAISE EXCEPTION 'data_artifact: resource % has no home to anchor the artifact event',
            p_resource;
    END IF;
END;
$$;

-- ════════════════════════════════════════════════════════════════════════════════════════════════
-- A5/A6 — projector and wrapper
-- ════════════════════════════════════════════════════════════════════════════════════════════════
--
-- The projector half. Reads ONLY the payload (payload-first design) so replay reproduces it.
--
-- A6 — WHAT THIS FUNCTION DELIBERATELY DOES NOT DO.
-- _project_property_set ends with:
--     IF v_owner_tbl = 'kb_resources' AND v_key IN ('keywords','descriptor','tags') THEN
--         PERFORM _rebuild_resource_search_vector(v_owner);
--     END IF;
-- There is no equivalent here and there must never be one. Committing an artifact changes NOTHING
-- about the corpus of searchable material: no chunk, no embedding, no FTS vector. That is the goal
-- clause `structured-data-is-never-found-by-resemblance`, and it is a STANDING boundary rather than
-- a not-yet — the plausible future change is "why don't we index artifact content too", which would
-- reintroduce precisely the shredding this table exists to escape.
-- The DEFAULT namespace a bare family name lands in: the owner of the resource the artifact hangs
-- on. For a TEAM-owned context that is the team, deliberately — if it defaulted to the writing
-- profile instead, every member of a team would mint kinds in their own personal namespace and the
-- team would never converge on a shared shape, which is the opposite of what a registry is for.
--
-- A cogmap-homed resource has no polymorphic owner to read (kb_cogmaps reaches teams only through
-- kb_team_cogmaps, which is a many-to-many and so names no single owner). Those fall back to the
-- home's owner_profile_id, which is NOT NULL and always present.
CREATE FUNCTION _data_artifact_kind_owner(p_resource uuid,
                                          OUT owner_table text, OUT owner_id uuid)
LANGUAGE plpgsql STABLE AS $$
DECLARE v_anchor_tbl text; v_anchor uuid; v_owner_profile uuid;
BEGIN
    SELECT h.anchor_table, h.anchor_id, h.owner_profile_id
      INTO v_anchor_tbl, v_anchor, v_owner_profile
      FROM kb_resource_homes h
     WHERE h.resource_id = p_resource
     ORDER BY (h.anchor_table = 'kb_cogmaps') DESC
     LIMIT 1;

    IF v_anchor_tbl IS NULL THEN
        RAISE EXCEPTION 'data_artifact: resource % has no home, so no default kind namespace',
            p_resource;
    END IF;

    IF v_anchor_tbl = 'kb_contexts' THEN
        SELECT c.owner_table, c.owner_id INTO owner_table, owner_id
          FROM kb_contexts c WHERE c.id = v_anchor;
    ELSE
        owner_table := 'kb_profiles';
        owner_id    := v_owner_profile;
    END IF;
END;
$$;

CREATE FUNCTION _project_data_artifact_committed(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid[]
LANGUAGE plpgsql AS $$
DECLARE v_id       uuid := (p_payload->>'artifact_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_kind     text := p_payload->>'artifact_kind';
        v_kind_tbl text := p_payload->>'kind_owner_table';
        v_kind_own uuid := (p_payload->>'kind_owner_id')::uuid;
        v_intent   text := p_payload->>'intent';
        v_prec     double precision := COALESCE((p_payload->>'precedence')::double precision, 0.0);
        v_hash     text := p_payload->>'content_hash';
        v_bytes    bigint := (p_payload->>'content_bytes')::bigint;

        v_supersedes uuid[] := COALESCE(
            (SELECT array_agg(x::uuid) FROM jsonb_array_elements_text(
                 COALESCE(p_payload->'supersedes', '[]'::jsonb)) x), '{}');
BEGIN
    -- Fold ONLY what the writer explicitly named. There is no "fold everything live of this kind"
    -- sweep here, and that absence is the whole point: see the A2 comment. An empty `supersedes`
    -- means this artifact replaces nothing, which is the common case for `member`.
    IF array_length(v_supersedes, 1) IS NOT NULL THEN
        UPDATE kb_data_artifacts SET is_folded = true, last_event_id = p_event
         WHERE id = ANY(v_supersedes)
           AND resource_id = v_resource      -- a writer may only fold artifacts of the resource it
           AND NOT is_folded;                -- is writing to; cross-resource folds are not a thing
    END IF;

    INSERT INTO kb_data_artifacts (id, resource_id, kind_owner_table, kind_owner_id, artifact_kind,
                                   intent, precedence, content_hash, content_bytes,
                                   asserted_by_event_id, last_event_id, created)
    VALUES (v_id, v_resource, v_kind_tbl, v_kind_own, v_kind, v_intent, v_prec, v_hash, v_bytes,
            p_event, p_event, v_occurred);

    -- The bytes arrive as a SEPARATE ARGUMENT, never inside p_payload — the same split
    -- resource_create/_project_resource_created uses. This is what makes "the payload carries the
    -- hash, never the body" true of the stored event rather than merely of the design doc: whatever
    -- is in p_payload is what _event_append wrote to kb_events.
    IF p_content IS NOT NULL AND jsonb_typeof(p_content) <> 'null' THEN
        INSERT INTO kb_data_artifact_content (artifact_id, content, content_hash)
        VALUES (v_id, p_content, v_hash);
    END IF;

    RETURN ARRAY[v_id];
END;
$$;

-- The wrapper. Same four moves as property_set, in the same order: validate, resolve anchor,
-- _event_append, _project_*.
CREATE FUNCTION data_artifact_commit(p_payload jsonb, p_content jsonb, p_emitter uuid,
                                     p_metadata jsonb DEFAULT '{}'::jsonb,
                                     p_invocation uuid DEFAULT NULL::uuid,
                                     p_correlation uuid DEFAULT NULL::uuid)
RETURNS uuid[]
LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_intent   text := p_payload->>'intent';
        v_kind_tbl text; v_kind_own uuid;
BEGIN
    -- The refusal that teaches its vocabulary (goal clause `a-declined-act-teaches-its-vocabulary`).
    -- The CHECK constraint would reject this anyway, but its message names a constraint rather than
    -- the vocabulary, and the caller learning the answer is the point of the refusal.
    IF v_intent IS NULL OR v_intent NOT IN ('current', 'member', 'pinned') THEN
        RAISE EXCEPTION 'data_artifact_commit: unrecognized intent %; the vocabulary is '
                        'current (replaces earlier artifacts of its kind), '
                        'member (a peer in a series), '
                        'pinned (never auto-selected, addressed by reference only)',
                        COALESCE(v_intent, '<null>');
    END IF;

    -- The split is enforced, not merely documented. A caller that put the bytes in the payload
    -- would have them written verbatim into kb_events by _event_append, silently defeating the
    -- hash-not-body property — and nothing downstream would notice.
    IF p_payload ? '__content' OR p_payload ? 'content' THEN
        RAISE EXCEPTION 'data_artifact_commit: content must ride the p_content argument, not the '
                        'payload — the event ledger carries the hash, never the body';
    END IF;

    IF p_payload->>'content_hash' IS NULL THEN
        RAISE EXCEPTION 'data_artifact_commit: content_hash is required — the event payload carries '
                        'the hash, never the body';
    END IF;

    SELECT a.anchor_table, a.anchor_id INTO v_anchor_tbl, v_anchor
      FROM _data_artifact_anchor(v_resource) a;

    -- Default the kind namespace INTO THE PAYLOAD, before the event is appended. Resolving it at
    -- projection time instead would re-read kb_contexts during replay — and a context's owner can
    -- change (context_reassigned), so replay would qualify an old artifact with today's owner and
    -- the byte-diff would fail. Identity-as-input applies to the namespace too, not just the id.
    IF p_payload->>'kind_owner_id' IS NULL THEN
        SELECT k.owner_table, k.owner_id INTO v_kind_tbl, v_kind_own
          FROM _data_artifact_kind_owner(v_resource) k;
        p_payload := p_payload
                   || jsonb_build_object('kind_owner_table', v_kind_tbl,
                                         'kind_owner_id',    v_kind_own);
    END IF;

    v_ev := _event_append('data_artifact_committed', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_data_artifact_committed(v_ev, p_payload, p_content);
END;
$$;

SELECT declare_migration(
    20260820000020,
    'additive',
    'Resource-owned data artifacts: kb_data_artifacts + kb_data_artifact_content, the data_artifact_committed event type (domain, TYPED with the committed schemars payload_schema), the _data_artifact_anchor and _data_artifact_kind_owner resolvers, and the _project_data_artifact_committed / data_artifact_commit projector+wrapper pair. Content rides data_artifact_commit''s p_content argument and never enters the event payload — the ledger carries the hash, so replay re-supplies bytes from the content table as a sidecar exactly as kb_block_content does. Family names are owner-qualified (kind_owner_table/kind_owner_id, defaulted from the owning resource''s home): registering a shape validates the backlog and stamps conformance verdicts, so a flat namespace would let one tenant stamp verdicts on another tenant''s unreadable data. Gives structured agent output (query plans, measurements, computation artifacts) a home of its own, instead of fenced code blocks in resource bodies which kb_properties rejects over 2704 bytes and the chunker shreds at its own YAML comment lines. Deliberately carries NO uniqueness index over (resource, kind) and NO GIN index on content: the store never asserts a supersession the writer did not declare, and artifacts are never findable by resemblance.'
);
