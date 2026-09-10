-- The whole-body replace shape for `resource_reblocked` (the ergonomics design pass, 2026-09-08).
-- The update path's whole-body arm no longer folds-then-hooks (block_mutated with
-- `replaces_body = true`, then a post-mutate re-block over one live block — 20260906000080): it
-- computes the replace-shaped partition against the PRE-update incumbents and fires ONE
-- `resource_reblocked` whose `replaces_body = true` marker selects these projector arms:
--   created blocks MINT their chunks (content/embedding ride the sidecar chunk map, the
--   `_insert_chunk` posture of `_project_block_mutated` 20260906000080:59-72; a fresh block
--   starts its own version count), folded incumbents' chunk generations retire (the replace
--   fold's posture, 20260906000080:51-52), and KEPT entries may carry NEW attribution
--   assertions (caller whole-body sources append across events; absorbed unions from folded
--   duplicates land carried = false — the delta-only rule is unchanged: a survivor's OWN rows
--   are never re-listed, and the Rust op skips unions whose source the survivor already holds).
-- Absent marker ⇒ byte-identical 20260905000010 behavior (reparent-only, coverage-guarded);
-- replay passes stored payloads as raw jsonb, so shipped events replay unchanged.
-- The entry guard admits kept-only manifests (section permutation moves seqs; assertion-only
-- manifests carry caller sources) and refuses only the true no-op — the full dedup stays the
-- Rust op's decision (20260905000010's own philosophy).
-- TRUST BOUNDARY: the marker is set ONLY by the gated write path (temper-substrate
-- `update_resource_in_tx`); no surface passes caller JSON to this payload.
-- Registry re-stamp: payload_schema re-pasted from the regenerated fixture (repo == registry ==
-- Rust types). `_event_append` does NOT validate payloads against this column (20260624000002),
-- so the re-stamp rides migration application, not a deploy gate.
CREATE OR REPLACE FUNCTION _project_resource_reblocked(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_resource uuid := (p_payload->>'resource_id')::uuid;
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
    v_block_json jsonb; v_chunk_json jsonb; v_raw jsonb; v_side jsonb;
    v_block uuid; v_chunk uuid; v_revision uuid;
    v_chunk_hashes text; v_chunk_count int;
    v_blocks jsonb;
    v_live_count bigint; v_payload_count bigint;
    v_assigned_ids text[];
    v_folded uuid[];
    v_kept uuid[];
    v_replaces boolean := coalesce((p_payload->>'replaces_body')::boolean, false);
BEGIN
    IF v_resource IS NULL OR NOT EXISTS (SELECT 1 FROM kb_resources WHERE id = v_resource) THEN
        RAISE EXCEPTION '_project_resource_reblocked: resource % not found', v_resource;
    END IF;

    -- Guard: folded and kept ids must name live non-folded blocks of THIS resource.
    v_folded := coalesce((SELECT array_agg((x->>0)::uuid) FROM jsonb_array_elements(coalesce(p_payload->'folded', '[]'::jsonb)) x), '{}');
    v_kept := coalesce((SELECT array_agg((x->>'block_id')::uuid) FROM jsonb_array_elements(coalesce(p_payload->'kept', '[]'::jsonb)) x), '{}');
    IF EXISTS (
        SELECT 1 FROM (SELECT unnest(v_folded) AS id UNION SELECT unnest(v_kept)) ids
        WHERE NOT EXISTS (
            SELECT 1 FROM kb_content_blocks b
            WHERE b.id = ids.id AND b.resource_id = v_resource AND NOT b.is_folded
        )
    ) THEN
        RAISE EXCEPTION '_project_resource_reblocked: folded/kept id is not a live block of resource %', v_resource;
    END IF;

    IF NOT v_replaces THEN
    -- Guard (op shape only): the created blocks' assignments cover exactly the chunk set the
    -- folded blocks lose. On the replace shape these checks are meaningless — created chunks are
    -- NEW (sidecar-backed), nobody inherits a folded block's chunks — and the sidecar-presence
    -- check in the insert loop takes over the guard duty.
    -- Count first... (v_payload_count counts OCCURRENCES; the distinct check below closes the
    -- double-assignment hole — a payload naming chunk X twice while omitting chunk Y would
    -- otherwise reparent X twice and silently strand Y on its folded row.)
    SELECT count(*) INTO v_live_count
        FROM kb_chunks c
        JOIN kb_content_blocks b ON b.id = c.block_id
       WHERE b.resource_id = v_resource AND NOT b.is_folded AND c.is_current
         AND b.id = ANY(v_folded);
    v_payload_count := 0;
    FOR v_block_json IN SELECT jsonb_array_elements(coalesce(p_payload->'created', '[]'::jsonb)) LOOP
        IF jsonb_array_length(v_block_json->'chunks') = 0 THEN
            RAISE EXCEPTION '_project_resource_reblocked: created block % has no chunks',
                v_block_json->>'block_id';
        END IF;
        v_payload_count := v_payload_count + jsonb_array_length(v_block_json->'chunks');
    END LOOP;
    SELECT coalesce(array_agg(x->>'chunk_id'), '{}') INTO v_assigned_ids
        FROM (SELECT jsonb_array_elements(blk->'chunks') AS x
                FROM jsonb_array_elements(coalesce(p_payload->'created', '[]'::jsonb)) AS blk) AS t;
    IF cardinality(v_assigned_ids) <> (SELECT count(DISTINCT u) FROM unnest(v_assigned_ids) u) THEN
        RAISE EXCEPTION '_project_resource_reblocked: payload assigns a chunk more than once (resource %)', v_resource;
    END IF;
    IF v_payload_count <> v_live_count THEN
        RAISE EXCEPTION '_project_resource_reblocked: payload assigns % chunk(s) but the folded blocks of resource % hold % live',
            v_payload_count, v_resource, v_live_count;
    END IF;
    -- ...then membership: every assigned chunk must be a live chunk of a FOLDED block (of THIS
    -- resource), with the payload's content_hash equal to the live row's — the projector derives
    -- block_body_hash from the payload hashes, so an unverified hash would let a hand-built
    -- payload forge an incumbent-impersonating merkle for a later re-block.
    FOR v_block_json IN SELECT jsonb_array_elements(p_payload->'created') LOOP
        FOR v_chunk_json IN SELECT jsonb_array_elements(v_block_json->'chunks') LOOP
            v_chunk := (v_chunk_json->>'chunk_id')::uuid;
            IF NOT EXISTS (
                SELECT 1 FROM kb_chunks c
                JOIN kb_content_blocks b ON b.id = c.block_id
                WHERE c.id = v_chunk AND b.resource_id = v_resource
                  AND NOT b.is_folded AND c.is_current AND b.id = ANY(v_folded)
                  AND c.content_hash = (v_chunk_json->>'content_hash')
            ) THEN
                RAISE EXCEPTION '_project_resource_reblocked: chunk % is not a live chunk of a folded block of resource % (or its content_hash disagrees)',
                    v_chunk, v_resource;
            END IF;
        END LOOP;
    END LOOP;
    END IF; -- NOT v_replaces

    -- 1. fold the superseded blocks (the `_project_charter_set` arm). On the replace shape their
    -- chunk generations retire (the `replaces_body` fold's posture, 20260906000080) — the chunks
    -- die with their block instead of being reparented.
    UPDATE kb_content_blocks SET is_folded = true, last_event_id = p_event
     WHERE id = ANY(v_folded) AND NOT is_folded;
    IF v_replaces THEN
        UPDATE kb_chunks SET is_current = false
         WHERE is_current AND block_id = ANY(v_folded);
    END IF;

    -- Guard: the parking phase maps each kept block to the distinct negative seq -(seq+1).
    -- That is injective and disjoint from every non-negative live or created seq, but a block
    -- ALREADY at a negative seq would park into the occupied positive band (kept -5 parks to 4,
    -- which kept 4 may hold). No shipped path writes a negative seq; refuse the state rather
    -- than assume its absence.
    IF EXISTS (SELECT 1 FROM kb_content_blocks
                WHERE resource_id = v_resource AND NOT is_folded AND seq < 0) THEN
        RAISE EXCEPTION '_project_resource_reblocked: resource % has a live block at a negative seq (parking-phase precondition)', v_resource;
    END IF;

    -- 2. park kept blocks at distinct negative seqs so their finals can never collide
    --    mid-transaction with each other or with the created blocks' seqs.
    UPDATE kb_content_blocks SET seq = (-(seq::bigint + 1))::int
     WHERE id = ANY(v_kept) AND NOT is_folded;

    -- 3. insert the created blocks, MINT or reparent their chunks, derive the block hash, store
    --    bytes. Reserved key: the raw block bytes, keyed by BLOCK ID (the mutate-path keying — a
    --    re-block addresses blocks by id; the create path keys by seq).
    v_blocks := coalesce(p_content->'__blocks', '{}'::jsonb);
    FOR v_block_json IN SELECT jsonb_array_elements(coalesce(p_payload->'created', '[]'::jsonb)) LOOP
        v_block := (v_block_json->>'block_id')::uuid;
        INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, last_event_id, created)
            VALUES (v_block, v_resource, (v_block_json->>'seq')::int, p_event, p_event, v_occurred);
        -- a block_role property is never fabricated: created blocks are born roleless (roles
        -- classify what a block IS; attribution records where content CAME FROM).
        v_chunk_hashes := '';
        v_chunk_count := 0;
        FOR v_chunk_json IN SELECT jsonb_array_elements(v_block_json->'chunks') LOOP
            v_chunk := (v_chunk_json->>'chunk_id')::uuid;
            IF v_replaces THEN
                -- Replace shape: the chunk is NEW — content/embedding ride the sidecar chunk
                -- map exactly as `_project_block_mutated` reads them (20260906000080:59-72).
                v_side := p_content -> (v_chunk_json->>'chunk_id');
                IF v_side IS NULL THEN
                    RAISE EXCEPTION '_project_resource_reblocked: content sidecar missing chunk %', v_chunk;
                END IF;
                PERFORM _insert_chunk(v_chunk, v_block, v_resource,
                    (v_chunk_json->>'chunk_index')::int,
                    coalesce((SELECT max(version) + 1 FROM kb_chunks WHERE block_id = v_block), 1),
                    v_chunk_json->>'content_hash', v_side->'embedding', true,
                    v_side->>'content', v_side->>'header_path',
                    NULLIF(v_side->>'heading_depth','')::smallint, v_occurred,
                    v_side->>'embedded_with');
            ELSE
                -- reparent, never rewrite: version, is_current, content_hash, embedding, and the
                -- heading metadata all ride the row untouched.
                UPDATE kb_chunks SET block_id = v_block, chunk_index = (v_chunk_json->>'chunk_index')::int
                 WHERE id = v_chunk;
            END IF;
            v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
            v_chunk_count := v_chunk_count + 1;
        END LOOP;
        -- the create-path derivation, applied to the block's ordered chunk hashes.
        INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
            VALUES (v_block, encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex'), v_chunk_count, v_occurred)
            RETURNING id INTO v_revision;
        UPDATE kb_content_blocks SET current_revision_id = v_revision WHERE id = v_block;
        v_raw := v_blocks -> v_block::text;
        IF v_raw IS NOT NULL THEN
            INSERT INTO kb_block_content (block_revision_id, content, content_hash)
                VALUES (v_revision, v_raw->>'content', v_raw->>'content_hash');
        END IF;
        -- attribution DELTA: entries carry their own `carried` flag (default false for an
        -- entry that lacks one). Kept blocks' own rows are deliberately not re-listed.
        PERFORM _insert_block_provenance(v_block, p_event, v_block_json->'attribution', false);
    END LOOP;

    -- 4. apply kept blocks' final seqs, only where the seq changed — and their NEW attribution
    -- assertions (2026-09-08 replace shape): caller whole-body sources (append across events,
    -- the incorporated-row semantic) and absorbed unions from folded duplicates (carried = false
    -- per entry; the Rust op has already skipped unions whose source the survivor holds). The
    -- delta-only rule is unchanged: a survivor's OWN rows are never re-listed here.
    FOR v_block_json IN SELECT jsonb_array_elements(coalesce(p_payload->'kept', '[]'::jsonb)) LOOP
        UPDATE kb_content_blocks
           SET seq = (v_block_json->>'seq')::int
         WHERE id = (v_block_json->>'block_id')::uuid
           AND seq IS DISTINCT FROM (v_block_json->>'seq')::int;
        PERFORM _insert_block_provenance((v_block_json->>'block_id')::uuid, p_event,
                                         v_block_json->'attribution', false);
    END LOOP;

    -- 5. tail, in the pinned order (20260714000002:59-64).
    PERFORM _recompute_resource_body_hash(v_resource, v_occurred);
    PERFORM _recompute_body_storage(v_resource);
    PERFORM _rebuild_resource_search_vector(v_resource);
    RETURN v_resource;
END;
$$;

-- ── the entry function: the no-op guard learns the replace shape ──────────────
-- A kept-only manifest is a REAL act on the replace path (a section permutation moves seqs; an
-- assertion-only manifest carries caller sources) — refusing "no created AND no folded" would
-- roll back a legal whole-body edit at the projector, post-append. The refusal narrows to the
-- true no-op: no created, no folded, AND no kept entry carrying new assertions. The full no-op
-- dedup (identical partition AND no sources ⇒ no event at all) stays the Rust op's decision.
CREATE OR REPLACE FUNCTION resource_reblock(p_payload jsonb, p_content jsonb, p_emitter uuid,
                                 p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                 p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    IF v_resource IS NULL OR NOT EXISTS (SELECT 1 FROM kb_resources WHERE id = v_resource) THEN
        RAISE EXCEPTION 'resource_reblock: resource % not found', v_resource;
    END IF;
    IF coalesce(jsonb_array_length(coalesce(p_payload->'created', '[]'::jsonb)), 0) = 0
       AND coalesce(jsonb_array_length(coalesce(p_payload->'folded', '[]'::jsonb)), 0) = 0
       AND NOT EXISTS (
            SELECT 1 FROM jsonb_array_elements(coalesce(p_payload->'kept', '[]'::jsonb)) k
            WHERE coalesce(jsonb_array_length(k->'attribution'), 0) > 0
               OR EXISTS (SELECT 1 FROM kb_content_blocks b
                           WHERE b.id = (k->>'block_id')::uuid
                             AND b.seq IS DISTINCT FROM (k->>'seq')::int)) THEN
        RAISE EXCEPTION 'resource_reblock: manifest changes nothing for resource % (no created, no folded, no new assertions, no kept seq moves)', v_resource;
    END IF;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'resource_reblock: resource % has no home to anchor the event', v_resource;
    END IF;
    v_ev := _event_append('resource_reblocked', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_resource_reblocked(v_ev, p_payload, p_content);
END;
$$;

-- ── carried is VISIBLE at the read grain (the same act, one migration) ────────
-- `resource_block_provenance` gains `is_carried` — without it a redistributed carried row is
-- indistinguishable from a direct assertion to every caller, the fabrication shape the register's
-- negative face forbids ("carried attribution is distinguishable from asserted attribution").
-- Additive column (20260905000010 defaulted the whole table to false); old readers ignore the
-- new field (BlockProvenanceRow carries no deny_unknown_fields — the decided older-client
-- posture, api.rs:338). (DROP first: CREATE OR REPLACE cannot change a return type — the
-- 20260704000007 precedent.)
DROP FUNCTION IF EXISTS resource_block_provenance(uuid, text, uuid);
CREATE OR REPLACE FUNCTION resource_block_provenance(p_resource uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(block_id uuid, block_seq integer, source_kind text, source_id uuid, source_uri text,
              accretion_seq integer, contributed_by_event_id uuid,
              created timestamp with time zone, is_carried boolean)
LANGUAGE sql STABLE
AS $function$
    SELECT b.id, b.seq, p.source_kind::text, p.source_id, r.uri, p.accretion_seq,
           p.contributed_by_event_id, p.created, p.is_carried
    FROM kb_content_blocks b
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    LEFT JOIN kb_remote_sources r ON p.source_kind = 'remote' AND r.id = p.source_id
    WHERE b.resource_id = p_resource AND NOT b.is_folded
      AND p_resource IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
    ORDER BY b.seq, p.accretion_seq;
$function$;

COMMENT ON FUNCTION resource_block_provenance(uuid, text, uuid) IS
  'Itemized per-block provenance for one resource, access-gated in SQL (empty set, never an '
  'error, for an unreadable resource). Gains is_carried in 20260908000010: true for split copies '
  'and absorbed-union rows written by a re-block or whole-body replace, distinguishable at row '
  'grain from direct assertions; pre-existing rows read carried = false (asserted).';

-- ── registry re-stamp: the regenerated fixture, byte for byte ─────────────────
UPDATE kb_event_types SET payload_schema = $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ResourceReblocked",
  "description": "`resource_reblocked` — re-cut one resource's blocks along section boundaries (the re-block\nsubstrate, task 2026-09-04).\n\nThe manifest IS the operation: the mapping rides in the payload and replay re-derives\nnothing. Three arms, mutually exhaustive over the resource's incumbent live blocks —\n`folded` (superseded in place, history intact), `created` (fresh blocks holding\nreassigned EXISTING chunks), and `kept` (rows that already carry exactly one section's\ncontent, named by derived-hash identity, never by a heuristic). Pure metadata: no content\nrewrite, no re-embed — `body = concat(blocks ORDER BY seq)` composes identically before\nand after, which is the invariant the payload exists to preserve.",
  "type": "object",
  "properties": {
    "created": {
      "description": "The blocks the partition creates, in seq order.",
      "type": "array",
      "items": {
        "$ref": "#/$defs/ReblockCreatedBlock"
      }
    },
    "folded": {
      "description": "The incumbent rows the partition supersedes.",
      "type": "array",
      "items": {
        "$ref": "#/$defs/BlockId"
      }
    },
    "kept": {
      "description": "The incumbent rows that survive unchanged (up to an explicit seq move).",
      "type": "array",
      "items": {
        "$ref": "#/$defs/ReblockKeptBlock"
      }
    },
    "replaces_body": {
      "description": "Whole-body replace semantics (the update path's arm): the sections chunk the caller's NEW\nbody, the created blocks MINT their chunks (the content sidecar carries them — the\nreparent-only premise does not hold), and folded incumbents' chunk generations retire.\n`false` — the serde default, so every pre-existing event replays identically — for the\nshipped op shape: created blocks reparent EXISTING CAS rows and nothing is inserted.",
      "type": "boolean"
    },
    "resource_id": {
      "$ref": "#/$defs/ResourceId"
    }
  },
  "required": [
    "resource_id"
  ],
  "$defs": {
    "BlockId": {
      "description": "A `kb_content_blocks.id` value — a resource's addressable interior unit.",
      "type": "string",
      "format": "uuid"
    },
    "ChunkId": {
      "description": "A `kb_chunks.id` value — one embedding window of a block's prose.",
      "type": "string",
      "format": "uuid"
    },
    "ChunkManifest": {
      "description": "Content-addressed chunk reference: structure + hash, NEVER prose (CAS rule, spec §0.1).",
      "type": "object",
      "properties": {
        "chunk_id": {
          "$ref": "#/$defs/ChunkId"
        },
        "chunk_index": {
          "type": "integer",
          "format": "int32"
        },
        "content_hash": {
          "type": "string"
        }
      },
      "required": [
        "chunk_id",
        "chunk_index",
        "content_hash"
      ]
    },
    "ProvenanceSource": {
      "description": "Tagged like the DDL's provenance_source_kind ({kind, value} sum — content-block spec).",
      "oneOf": [
        {
          "type": "object",
          "properties": {
            "kind": {
              "type": "string",
              "const": "event"
            },
            "value": {
              "type": "string",
              "format": "uuid"
            }
          },
          "required": [
            "kind",
            "value"
          ]
        },
        {
          "type": "object",
          "properties": {
            "kind": {
              "type": "string",
              "const": "resource"
            },
            "value": {
              "type": "string",
              "format": "uuid"
            }
          },
          "required": [
            "kind",
            "value"
          ]
        },
        {
          "description": "An external URL (e.g. a Linear issue, a GitHub PR, a doc). The value is the URL as supplied;\nthe projector normalizes + resolves it to a `kb_remote_sources.id` via `_upsert_remote_source`.",
          "type": "object",
          "properties": {
            "kind": {
              "type": "string",
              "const": "remote"
            },
            "value": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "value"
          ]
        }
      ]
    },
    "ReblockAttribution": {
      "description": "One attribution row a re-block writes — the DELTA, never a re-listing.\n\n`source` + `seq` mirror [`Incorporation`] (the `kb_block_provenance`\n`(block_id, source_kind, source_id, contributed_by_event_id)` UNIQUE grain carries the event\nid; `seq` is the accretion order). `carried` is the load-bearing marking: `false` for a merge\nunion (the content IS in the block, so the attribution is direct), `true` for a split copy —\nthe same source attributed to a block that holds only PART of the content it once covered,\ndistinguishable at row grain from asserted attribution and never readable as direct.",
      "type": "object",
      "properties": {
        "carried": {
          "type": "boolean"
        },
        "seq": {
          "type": "integer",
          "format": "int32"
        },
        "source": {
          "$ref": "#/$defs/ProvenanceSource"
        }
      },
      "required": [
        "source",
        "seq",
        "carried"
      ]
    },
    "ReblockCreatedBlock": {
      "description": "One block a re-block CREATES: fresh identity, a slot in the new partition, and its\nchunk assignments.",
      "type": "object",
      "properties": {
        "attribution": {
          "description": "The attribution DELTA to write for this block. Deliberately never re-lists sources\nalready on a kept row: the survivor's own provenance rides along untouched, and\nre-inserting it under a new `contributed_by_event_id` would double-count it in every\nstanding read. Empty for an unattributed block.",
          "type": "array",
          "items": {
            "$ref": "#/$defs/ReblockAttribution"
          }
        },
        "block_id": {
          "description": "Identity-as-input: minted by the operation and carried here so replay reproduces the\nsame row id (`kb_content_blocks.id` carries a column DEFAULT, but a re-partition's block\nidentity is chosen by the op, never minted by the projector — the create-path\nidentity-as-input rule applied to an existing table).",
          "$ref": "#/$defs/BlockId"
        },
        "chunks": {
          "description": "The chunk assignments for this block, in order. SHAPE-DEPENDENT: on the shipped op shape\nthese are EXISTING chunk rows reassigned — chunks are never inserted, deleted, or rewritten\nby a re-block, the `content_hash` here must equal the live row's, and the embedding rides\nthe row through the reparent untouched. On the whole-body replace shape\n(`ResourceReblocked::replaces_body`) these are NEW chunk ids minted by the operation, with\ntheir content/embedding riding the sidecar map (the `block_mutate` posture) — the same\nmanifest field carries both, discriminated by the marker. The projector derives the block's\n`block_body_hash` from these ordered hashes, the create-path derivation.",
          "type": "array",
          "items": {
            "$ref": "#/$defs/ChunkManifest"
          }
        },
        "seq": {
          "description": "The block's position in the NEW partition.",
          "type": "integer",
          "format": "int32"
        }
      },
      "required": [
        "block_id",
        "seq",
        "chunks"
      ]
    },
    "ReblockKeptBlock": {
      "description": "One block a re-block KEEPS: the row survives with its content, chunks, and provenance\nuntouched — only its `seq` may move (the projector applies the move only when it changed).",
      "type": "object",
      "properties": {
        "attribution": {
          "description": "NEW assertions only — the delta this event writes onto the survivor. Deliberately never a\nre-listing of the survivor's OWN rows (those ride along untouched; re-inserting them under\na new `contributed_by_event_id` would double-count them in every standing read): this list\ncarries the populations the replace-shaped partition adds — caller whole-body sources\n(appended across events, the same semantic as `block_mutate`'s incorporated rows and the\nannotate path) and absorbed unions from folded duplicates (skipping sources the survivor\nalready holds). Empty for a kept block with nothing new to assert.",
          "type": "array",
          "items": {
            "$ref": "#/$defs/ReblockAttribution"
          }
        },
        "block_id": {
          "$ref": "#/$defs/BlockId"
        },
        "seq": {
          "description": "The block's position in the NEW partition.",
          "type": "integer",
          "format": "int32"
        }
      },
      "required": [
        "block_id",
        "seq"
      ]
    },
    "ResourceId": {
      "description": "A `kb_resources.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}$JS$::jsonb
WHERE name = 'resource_reblocked';

COMMENT ON FUNCTION _project_resource_reblocked(uuid, jsonb, jsonb) IS
  'Projects resource_reblocked: folds the superseded blocks, parks kept blocks at negative seqs, '
  'inserts the created ones, applies kept seq finals + NEW kept attribution, then the pinned tail '
  'recompute. Two shapes: absent replaces_body (the 20260905000010 op) reparents EXISTING CAS '
  'chunks under coverage guards; replaces_body = true (the 2026-09-08 whole-body replace) MINTS '
  'created chunks from the sidecar chunk map (the _insert_chunk posture), retires folded '
  'generations, and skips the reparent coverage guards. Kept entries may carry NEW attribution '
  'assertions — caller whole-body sources and absorbed unions; a survivor''s own rows are never '
  're-listed. Amended by 20260908000010_whole_body_replace_reblock.sql.';

COMMENT ON FUNCTION resource_reblock(jsonb, jsonb, uuid, jsonb, uuid, uuid) IS
  're-block entry: anchor-resolving wrapper around _project_resource_reblocked. The no-op guard '
  'refuses only a manifest with no created, no folded, AND no kept attribution entries — a '
  'kept-only manifest with seq moves or new assertions is a real act (section permutation, '
  'assertion-only re-assert). The full no-op dedup is the Rust op''s decision. Amended by '
  '20260908000010_whole_body_replace_reblock.sql.';

SELECT declare_migration(
    20260908000010,
    'additive',
    'the whole-body replace shape for resource_reblocked: optional replaces_body marker selects '
    'mint-dont-reparent chunk handling (sidecar-backed, the block_mutate posture), retires folded '
    'chunk generations, and admits NEW attribution assertions on kept entries (caller whole-body '
    'sources append; absorbed unions land carried=false; a survivor''s own rows are never '
    're-listed). Absent marker = byte-identical 20260905000010 behavior; shipped events replay '
    'unchanged. The entry no-op guard narrows to the true no-op. Registry re-stamped from the '
    'regenerated fixture.'
);
