-- Resource-owned data artifacts: schema-boundable JSONB owned by a resource, committed as an event.
--
-- Design and rationale: internal/superpowers/specs/2026-08-20-resource-owned-data-artifacts-design.md
-- (vault research 01a02163-8670-7cc2-96a6-1a520ec8a0f8). Read that before changing anything here —
-- the reasoning for the shape is there, not in this file.

-- Metadata and bytes are split, following kb_block_content (20260714000002). Every column here is
-- derivable from the event payload, so the table byte-diffs under replay; the bytes ride a sidecar
-- and are proved by content_hash.
CREATE TABLE kb_data_artifacts (
    -- No DEFAULT, deliberately: the id is minted by the caller and carried in the payload
    -- (identity-as-input), so a server-side default would mint a different id on replay.
    id                    UUID PRIMARY KEY,
    resource_id           UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    -- Family names are owner-qualified: (kind_owner, artifact_kind), never the bare name. Defaulted
    -- from the owning resource's home by _data_artifact_kind_owner when the caller omits it.
    kind_owner_table      VARCHAR(64) NOT NULL
                              CHECK (kind_owner_table IN ('kb_profiles', 'kb_teams')),
    kind_owner_id         UUID NOT NULL,
    artifact_kind         TEXT NOT NULL,
    -- The closed selection vocabulary. Enforced here as well as at the service edge so a path that
    -- bypasses the service layer cannot store a fourth term. Pinned to payloads::ArtifactIntent's
    -- serde renames.
    intent                TEXT NOT NULL CHECK (intent IN ('current', 'member', 'pinned')),
    precedence            DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    content_hash          TEXT NOT NULL,      -- bare sha256 hex of content's raw bytes
    content_bytes         BIGINT NOT NULL,    -- surfaced so a reader can decide whether to fetch
    -- Assert/fold, as kb_properties and kb_edges. No mutable `revised` column: revision IS the
    -- folded chain, and a mutable timestamp would be the one non-payload-derivable column.
    asserted_by_event_id  UUID NOT NULL REFERENCES kb_events(id),
    last_event_id         UUID NOT NULL REFERENCES kb_events(id),
    is_folded             BOOLEAN NOT NULL DEFAULT false,
    created               TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- NO partial unique index over (resource_id, kind), unlike every sibling assert/fold table. This is
-- deliberate and load-bearing: a resource owns a has-many collection, and only the writer knows
-- whether one artifact replaces another, so supersession is declared per-commit (see the projector's
-- `supersedes` handling) and never enforced here. Adding one breaks the design.
CREATE INDEX idx_kb_data_artifacts_resource ON kb_data_artifacts(resource_id) WHERE NOT is_folded;
-- Keyed on the QUALIFIED name — a lookup by bare artifact_kind would silently span namespaces.
CREATE INDEX idx_kb_data_artifacts_kind     ON kb_data_artifacts(resource_id, kind_owner_table, kind_owner_id, artifact_kind) WHERE NOT is_folded;

-- NO GIN index on content, deliberately. kb_properties indexes its JSONB because it exists to be
-- queried; artifacts are reached only through what points at them, never by resemblance.
CREATE TABLE kb_data_artifact_content (
    artifact_id   UUID PRIMARY KEY REFERENCES kb_data_artifacts(id) ON DELETE CASCADE,
    content       JSONB NOT NULL,
    content_hash  TEXT  NOT NULL   -- bare sha256 hex of content's raw bytes (Rust `sha256_hex` twin)
);

-- TYPED, with a published payload_schema (the subscription_delivery_disposed shape, 20260819000030
-- — not the permissive webhook_received one). The literal is the committed schemars snapshot,
-- crates/temper-substrate/tests/fixtures/payloads/data_artifact_committed.v1.schema.json, verbatim:
-- repo == registry == Rust types. Regenerate both halves together with
--   UPDATE_SCHEMA=1 cargo make test-schema
-- and paste the result here; hand-editing either half breaks the chain silently.
--
-- category is spelled explicitly because 20260719000010 dropped the column DEFAULT so an omitting
-- registration fails loudly rather than silently joining the trail allowlist.
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

-- An artifact has no home of its own; it inherits the owning resource's. The tiebreak is carried
-- verbatim from _property_owner_anchor (20260727000030) and is load-bearing: a resource homed in
-- both a context and a cogmap anchors on the cogmap. Re-deriving it is how the two drift.
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

-- The default namespace for a bare family name. For a TEAM-owned context this is the team, not the
-- writing profile — otherwise each member mints families privately and the team never converges on a
-- shared shape. A cogmap-homed resource has no polymorphic owner to read (kb_team_cogmaps is
-- many-to-many and names none), so it falls back to the home's owner_profile_id.
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

-- Reads ONLY the payload, plus the content sidecar. Note what is absent: _project_property_set ends
-- by calling _rebuild_resource_search_vector, and there is deliberately no equivalent here —
-- committing an artifact must change nothing about the searchable corpus.
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

-- Wrapper: validate, resolve anchor, append event, project. Same four moves as property_set.
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
    'Resource-owned data artifacts: kb_data_artifacts + kb_data_artifact_content, the data_artifact_committed event type (domain, TYPED with the committed schemars payload_schema), the _data_artifact_anchor and _data_artifact_kind_owner resolvers, and the _project_data_artifact_committed / data_artifact_commit projector+wrapper pair. Content rides data_artifact_commit''s p_content argument and never enters the event payload, so the ledger carries only the hash and replay re-supplies bytes from the content table as a sidecar (the kb_block_content shape). Family names are owner-qualified because registering a shape validates the backlog and stamps conformance verdicts, which under a flat namespace would let one tenant stamp verdicts on another tenant''s unreadable data. Deliberately carries NO uniqueness index over (resource, kind) and NO GIN index on content. Design: internal/superpowers/specs/2026-08-20-resource-owned-data-artifacts-design.md.'
);
