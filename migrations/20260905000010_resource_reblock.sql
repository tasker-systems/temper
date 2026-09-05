-- The re-block substrate: `resource_reblocked`, the event-sourced re-partition operation.
-- Re-cuts one resource's blocks along heading-section boundaries: fold the superseded blocks,
-- insert the created ones at their payload seqs, reparent the EXISTING chunk rows to them,
-- apply kept blocks' seq moves. Pure metadata — no content rewrite, no re-embed, no chunk row
-- inserted or deleted — so `body = concat(blocks ORDER BY seq)` composes identically before
-- and after.
--
-- THE CONSTRAINT (task pin): fold → insert → reparent must stage inside ONE transaction. The
-- partial `UNIQUE (resource_id, seq) WHERE NOT is_folded` (20260629000001) forbids a created
-- block at a seq a live block still holds, and forbids two live blocks sharing a seq — which a
-- naive kept-block seq move also violates MID-TRANSACTION (a kept block moving into a seq
-- another kept block is vacating). Kept moves are therefore two-phase: park every kept block
-- at a distinct negative seq (`-(seq+1)`, injective, cannot collide with any live or created
-- seq), then apply the payload finals. The negatives never survive the transaction.
--
-- The mapping (kept ids, created ids + chunk assignments, seqs, attribution delta) rides in
-- the event payload — replay re-derives nothing. Created blocks' `block_body_hash` derives
-- from the payload's ordered chunk hashes by the create-path formula
-- (encode(sha256(...)), 20260714000002 `_project_blocks`); chunks are never rewritten, so the
-- derived hash equals what a fresh ingest of the same partition produces — that equality is
-- the identity-preservation contract, witnessed Rust-side.
--
-- Attribution: additive `kb_block_provenance.is_carried` marks a split COPY (carried = true) —
-- the same source now attributed to a block holding only part of the content it once covered —
-- distinguishable at row grain from asserted attribution, never readable as direct. A merge
-- union writes carried = false (the content IS in the block). The payload carries the DELTA
-- only: a kept block's own provenance rides along untouched, never re-inserted under the new
-- event id. `_insert_block_provenance` gains a defaulted `p_carried` used when an entry does
-- not carry its own flag — every existing caller's entries lack the key, so their rows are
-- unchanged (20260704000007 body otherwise verbatim).
--
-- Tail order is load-bearing: `_recompute_resource_body_hash` → `_recompute_body_storage` →
-- `_rebuild_resource_search_vector` (20260714000002:59-64 — storage inherits the body-hash
-- lock's serialization). Registered TYPED per the blob_committed posture (20260903000020): the
-- literal below is the committed schemars snapshot,
-- crates/temper-substrate/tests/fixtures/payloads/resource_reblocked.v1.schema.json, byte for
-- byte — regenerate both halves together and re-paste; hand-editing either half breaks the
-- chain silently.

-- ── the carried marking (additive column; default false = every existing row reads asserted) ──
ALTER TABLE kb_block_provenance ADD COLUMN is_carried BOOLEAN NOT NULL DEFAULT false;

-- ── AMEND _insert_block_provenance: stamp the carried flag ─────────────────────
-- Body verbatim from 20260704000007 with ONE change: the INSERT now also stamps `is_carried`,
-- taken from the entry's own `carried` key when present, else from the new defaulted
-- `p_carried` parameter. Existing callers pass no flag and their entries carry no key — their
-- rows read carried = false exactly as before (additive signature).
CREATE OR REPLACE FUNCTION _insert_block_provenance(p_block uuid, p_event uuid, p_incorporated jsonb, p_carried boolean DEFAULT false)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_inc jsonb; v_kind text; v_val text; v_source_id uuid;
BEGIN
    IF p_incorporated IS NULL OR jsonb_typeof(p_incorporated) <> 'array' THEN
        RETURN;
    END IF;
    FOR v_inc IN SELECT jsonb_array_elements(p_incorporated) LOOP
        v_kind := v_inc #>> '{source,kind}';
        v_val  := v_inc #>> '{source,value}';
        IF v_kind = 'remote' THEN
            v_source_id := _upsert_remote_source(v_val);   -- URL → minted/looked-up kb_remote_sources id
        ELSE
            v_source_id := v_val::uuid;                    -- resource/event: the value IS the id
        END IF;
        INSERT INTO kb_block_provenance
            (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq, is_carried)
        VALUES (p_block, v_kind::provenance_source_kind, v_source_id, p_event, (v_inc ->> 'seq')::int,
                coalesce((v_inc ->> 'carried')::boolean, p_carried))
        ON CONFLICT (block_id, source_kind, source_id, contributed_by_event_id) DO NOTHING;
    END LOOP;
END;
$$;

-- TYPED, with a published payload_schema — the data_artifact_committed/blob_committed posture:
-- the literal is the committed schemars snapshot, byte for byte: repo == registry == Rust types.
-- Regenerate both halves together with UPDATE_SCHEMA=1 and paste the result here.
-- category is spelled explicitly (20260719000010 dropped the column DEFAULT so an omitting
-- registration fails loudly rather than silently joining the trail allowlist).
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('resource_reblocked', $JS${
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
          "description": "Identity-as-input: minted by the operation and carried here so replay reproduces the\nsame row id. `kb_content_blocks.id` has no DEFAULT for this reason.",
          "$ref": "#/$defs/BlockId"
        },
        "chunks": {
          "description": "The EXISTING chunk rows reassigned to this block, in order, with renumbered\n`chunk_index`. Chunks are never inserted, deleted, or rewritten by a re-block — the\n`content_hash` here must equal the live row's, and the embedding rides the row through\nthe reparent untouched. The projector derives the block's `block_body_hash` from these\nordered hashes, the create-path derivation.",
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
}$JS$::jsonb, 1, 'domain')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema,
      schema_version = EXCLUDED.schema_version,
      category       = EXCLUDED.category;

-- ── the projector half (replay-stable; arity of `_project_block_mutated`) ─────────────────
-- Guards raise in block_mutate's voice: the payload's chunk assignments must cover EXACTLY the
-- live is_current chunks of the resource's non-folded blocks (count + membership), every folded
-- and kept id must name a live non-folded block of THIS resource, and every created block must
-- carry >= 1 chunk. There is deliberately NO no-op re-check here — the partition is computed in
-- Rust (the chunker); a duplicate SQL opinion would be a second decision point, not a guard.
CREATE FUNCTION _project_resource_reblocked(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_resource uuid := (p_payload->>'resource_id')::uuid;
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
    v_block_json jsonb; v_chunk_json jsonb; v_raw jsonb;
    v_block uuid; v_chunk uuid; v_revision uuid;
    v_chunk_hashes text; v_chunk_count int;
    v_blocks jsonb;
    v_live_count bigint; v_payload_count bigint;
    v_folded uuid[];
    v_kept uuid[];
BEGIN
    IF v_resource IS NULL OR NOT EXISTS (SELECT 1 FROM kb_resources WHERE id = v_resource) THEN
        RAISE EXCEPTION '_project_resource_reblocked: resource % not found', v_resource;
    END IF;

    -- Guard: chunk assignments cover exactly the incumbent live chunk set. Count first...
    SELECT count(*) INTO v_live_count
        FROM kb_chunks c
        JOIN kb_content_blocks b ON b.id = c.block_id
       WHERE b.resource_id = v_resource AND NOT b.is_folded AND c.is_current;
    v_payload_count := 0;
    FOR v_block_json IN SELECT jsonb_array_elements(coalesce(p_payload->'created', '[]'::jsonb)) LOOP
        IF jsonb_array_length(v_block_json->'chunks') = 0 THEN
            RAISE EXCEPTION '_project_resource_reblocked: created block % has no chunks',
                v_block_json->>'block_id';
        END IF;
        v_payload_count := v_payload_count + jsonb_array_length(v_block_json->'chunks');
    END LOOP;
    IF v_payload_count <> v_live_count THEN
        RAISE EXCEPTION '_project_resource_reblocked: payload assigns % chunk(s) but resource % has % live',
            v_payload_count, v_resource, v_live_count;
    END IF;
    -- ...then membership: every assigned chunk must be one of those live chunks (of THIS resource).
    FOR v_block_json IN SELECT jsonb_array_elements(p_payload->'created') LOOP
        FOR v_chunk_json IN SELECT jsonb_array_elements(v_block_json->'chunks') LOOP
            v_chunk := (v_chunk_json->>'chunk_id')::uuid;
            IF NOT EXISTS (
                SELECT 1 FROM kb_chunks c
                JOIN kb_content_blocks b ON b.id = c.block_id
                WHERE c.id = v_chunk AND b.resource_id = v_resource
                  AND NOT b.is_folded AND c.is_current
            ) THEN
                RAISE EXCEPTION '_project_resource_reblocked: chunk % is not a live chunk of resource %',
                    v_chunk, v_resource;
            END IF;
        END LOOP;
    END LOOP;

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

    -- 1. fold the superseded blocks (the `_project_charter_set` arm).
    UPDATE kb_content_blocks SET is_folded = true, last_event_id = p_event
     WHERE id = ANY(v_folded) AND NOT is_folded;

    -- 2. park kept blocks at distinct negative seqs so their finals can never collide
    --    mid-transaction with each other or with the created blocks' seqs.
    UPDATE kb_content_blocks SET seq = (-(seq::bigint + 1))::int
     WHERE id = ANY(v_kept) AND NOT is_folded;

    -- 3. insert the created blocks, reparent their chunks, derive the block hash, store bytes.
    --    Reserved key: the raw block bytes, keyed by BLOCK ID (the mutate-path keying — a
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
            -- reparent, never rewrite: version, is_current, content_hash, embedding, and the
            -- heading metadata all ride the row untouched.
            UPDATE kb_chunks SET block_id = v_block, chunk_index = (v_chunk_json->>'chunk_index')::int
             WHERE id = v_chunk;
            v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
            v_chunk_count := v_chunk_count + 1;
        END LOOP;
        -- the create-path derivation, applied to existing chunks: sha256 over the ordered
        -- content_hash concatenation.
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

    -- 4. apply kept blocks' final seqs, only where the seq changed.
    FOR v_block_json IN SELECT jsonb_array_elements(coalesce(p_payload->'kept', '[]'::jsonb)) LOOP
        UPDATE kb_content_blocks
           SET seq = (v_block_json->>'seq')::int
         WHERE id = (v_block_json->>'block_id')::uuid
           AND seq IS DISTINCT FROM (v_block_json->>'seq')::int;
    END LOOP;

    -- 5. tail, in the pinned order (20260714000002:59-64).
    PERFORM _recompute_resource_body_hash(v_resource, v_occurred);
    PERFORM _recompute_body_storage(v_resource);
    PERFORM _rebuild_resource_search_vector(v_resource);
    RETURN v_resource;
END;
$$;

-- ── the entry function (event + projection, one txn) ──────────────────────────
-- Mirrors block_annotate's anchor resolution + correlation params, with the content sidecar of
-- block_mutate. Refuses a manifest that changes nothing (no created AND no folded) — the
-- caller-error mirror of block_annotate's empty source set; the full no-op dedup is the Rust
-- op's decision, not a second SQL opinion.
CREATE FUNCTION resource_reblock(p_payload jsonb, p_content jsonb, p_emitter uuid,
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
       AND coalesce(jsonb_array_length(coalesce(p_payload->'folded', '[]'::jsonb)), 0) = 0 THEN
        RAISE EXCEPTION 'resource_reblock: manifest changes nothing for resource % (no created, no folded blocks)', v_resource;
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

COMMENT ON FUNCTION resource_reblock(jsonb, jsonb, uuid, jsonb, uuid, uuid) IS
're-block entry: re-cuts one resource''s blocks along section boundaries by folding the
superseded blocks, inserting the created ones (reparenting EXISTING chunk rows — no content
rewrite, no re-embed), and applying kept blocks'' seq moves, one transaction. The payload IS
the re-partition manifest: kept ids, created ids + chunk assignments, seqs, and the
attribution delta, so replay re-derives nothing. Created blocks'' block_body_hash derives
from the payload''s ordered chunk hashes; the derived-hash equality with an incumbent block
is what makes a section KEEP that block row. is_carried marks split copies of attribution,
never a merge union. The full no-op dedup (identical partition ⇒ no event) is the Rust op''s
decision, not a SQL one; this wrapper only refuses a manifest that names no structural
change. Chunk assignments must cover exactly the resource''s live chunks — a payload that
over- or under-covers raises and nothing is written.';

SELECT declare_migration(
    20260905000010,
    'additive',
    'the re-block substrate: typed resource_reblocked event (domain, schemars payload_schema per the blob_committed precedent) with _project_resource_reblocked (fold → insert → reparent, one transaction against the partial live-seq unique index; created blocks derive block_body_hash from the payload''s ordered chunk hashes; kept-block seq moves stage through a negative parking seq) and resource_reblock (anchor-resolving entry, no SQL-side no-op re-check — the partition is computed in Rust). Additive kb_block_provenance.is_carried marks split copies of attribution at row grain; _insert_block_provenance gains a defaulted p_carried (existing callers unchanged).'
);
