-- kb_blobs: immutable, content-addressed binary blobs — related to resources by edges, never
-- resources themselves (spec: binary blobs, 2026-09-01; temper-artifacts/specs/
-- 2026-09-01-binary-blobs-design.md, D1-D4, D8-D10; vault copy 01a05d01-648e-74d1-a6b4-345c9bde744b).
-- Read the spec before changing anything here — the reasoning for the shape is there, not in this file.
--
-- kb_blob_files is REPLACED in this same migration (D8): zero production readers, a status machine
-- describing the deprecated extract-to-resource flow, gen_random_uuid() and a single-resource FK
-- both wrong for the new model. It does not survive alongside its successor.

-- Bytes live EXTERNALLY (Vercel Blob, D1); this row is metadata only — no content column ON
-- kb_blobs. (Staged upload bytes DO sit in Postgres, in kb_blob_upload_segments.bytes —
-- 20260903000040, pre-ledger by design and declared in the personal-data surface manifest;
-- an erasure enumeration driven by this header must not stop at the blob row.) Every column
-- here is derivable from the event payload, so the table byte-diffs under replay (replay.rs
-- diffs it in FULL).
CREATE TABLE kb_blobs (
    -- No DEFAULT, deliberately: the id is minted by the caller and carried in the payload
    -- (identity-as-input, D2), so a server-side default would mint a different id on replay.
    id            UUID PRIMARY KEY,
    -- Bare sha256 hex of the raw bytes. D1's dedup made a constraint: same bytes committed twice
    -- is one row and one object. Erasure overrides dedup (its refusal keys on this column) — the
    -- erasure task owns that arm, not this migration.
    content_hash  TEXT NOT NULL UNIQUE,
    -- The content-addressed external path {hash[0:2]}/{hash}. The wrapper enforces the derivation
    -- rather than assuming it (D1: addressing is the branch-safety and dedup mechanism).
    blob_pathname TEXT NOT NULL,
    -- Nullable: the erasure pre-pass EMPTIES metadata and keeps the hash (D5.2 of the spec — the
    -- erasure task lands the arm; the nullability is stamped here so the shape is already right).
    content_type  TEXT,
    content_bytes BIGINT NOT NULL,
    -- Assert/fold linkage, as every sibling projection table carries. Nothing folds a blob in v1
    -- (blobs are immutable; revision is a new blob plus relation re-pointing, D10/D1) — the
    -- columns keep the family shape and give erasure its last_event_id.
    asserted_by_event_id UUID NOT NULL REFERENCES kb_events(id),
    last_event_id        UUID NOT NULL REFERENCES kb_events(id),
    is_folded            BOOLEAN NOT NULL DEFAULT false,
    created              TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A blob is homed in resource-terms exactly like a resource (D2: "homes per the substrate's
-- access terms"): one row per blob, anchor over (kb_contexts, kb_cogmaps), and the SAME read
-- gate as kb_resource_homes — a blob homed in a context or cogmap is visible to that home's
-- readers (blob-visibility-self-contained). The kb_blob_homes.id is a masked surrogate (no
-- inbound references), like kb_resource_homes.id.
CREATE TABLE kb_blob_homes (
    id                    UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    blob_id               UUID NOT NULL UNIQUE REFERENCES kb_blobs(id) ON DELETE CASCADE,
    anchor_table          VARCHAR(64) NOT NULL CHECK (anchor_table IN ('kb_contexts', 'kb_cogmaps')),
    anchor_id             UUID NOT NULL,
    originator_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    owner_profile_id      UUID NOT NULL REFERENCES kb_profiles(id),
    created               TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_kb_blob_homes_anchor ON kb_blob_homes(anchor_table, anchor_id);

-- D3: relations are ordinary edges — the endpoint CHECK admits 'kb_blobs' as a third endpoint
-- kind. What the CHECK does NOT do is admit blobs anywhere else: kb_properties.owner_table,
-- kb_cogmap_members.member_table, and the region member CHECK all stay closed (D10: a blob is
-- addressed by id, and the resource graph walk surfaces must never inherit blob endpoints by
-- accident — exclusion there is a deliberate decision, spec D3, not an omission).
ALTER TABLE kb_edges DROP CONSTRAINT kb_edges_source_table_check;
ALTER TABLE kb_edges ADD  CONSTRAINT kb_edges_source_table_check
    CHECK (source_table IN ('kb_resources', 'kb_cogmaps', 'kb_blobs'));
ALTER TABLE kb_edges DROP CONSTRAINT kb_edges_target_table_check;
ALTER TABLE kb_edges ADD  CONSTRAINT kb_edges_target_table_check
    CHECK (target_table IN ('kb_resources', 'kb_cogmaps', 'kb_blobs'));

-- D8: the deprecated table dies in the same migration that creates its replacement — never
-- extended, never read in production (only the identity graft test named it).
DROP TABLE kb_blob_files;

-- TYPED, with a published payload_schema — the data_artifact_committed precedent (20260820000020)
-- verbatim: the literal is the committed schemars snapshot,
-- crates/temper-substrate/tests/fixtures/payloads/blob_committed.v1.schema.json, byte for byte:
-- repo == registry == Rust types. Regenerate both halves together with
--   UPDATE_SCHEMA=1 cargo make test-schema-substrate
-- and paste the result here; hand-editing either half breaks the chain silently.
--
-- category is spelled explicitly because 20260719000010 dropped the column DEFAULT so an omitting
-- registration fails loudly rather than silently joining the trail allowlist.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('blob_committed', $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "BlobCommitted",
  "description": "Commit one blob (spec: binary blobs — external, content-addressed, related by edges,\n2026-09-01; D2/D4).\n\n**The bytes are NOT here and never were** — stricter than `DataArtifactCommitted`, which\ncarries a JSONB sidecar: a blob's bytes live in external object storage at the\ncontent-addressed pathname, so there is no sidecar at all and the ledger's business is\nprovenance only (`ledger-carries-hash-not-bytes`).\n\nDeliberately divergent from `DataArtifactCommitted` (spec D10, declared not omitted): no\n`intent` — a blob is addressed by id and its bytes are immutable, so there is no selection\nquestion to answer; no `supersedes` — revision is a new blob plus relation re-pointing,\nnever a fold.",
  "type": "object",
  "properties": {
    "blob_id": {
      "description": "Identity-as-input: minted by the caller and carried here so replay reproduces the same\nrow id. `kb_blobs.id` deliberately has no DEFAULT for this reason.",
      "$ref": "#/$defs/BlobId"
    },
    "blob_pathname": {
      "description": "The content-addressed external path: `{hash[0:2]}/{hash}`. The SQL wrapper refuses a\npayload whose pathname is anything else — addressing is enforced, not assumed (D1).",
      "type": "string"
    },
    "content_bytes": {
      "description": "Surfaced so a reader can decide whether to fetch. Never the bytes themselves.",
      "type": "integer",
      "format": "int64"
    },
    "content_hash": {
      "description": "Bare sha256 hex of the blob's raw bytes — the dedup key (`kb_blobs.content_hash` UNIQUE)\nand the erasure act's join key (its redacted set keys on content hash).",
      "type": "string"
    },
    "content_type": {
      "description": "The stored media type, allowlist-checked at commit; the refusal names the vocabulary (D9).",
      "type": "string"
    },
    "home": {
      "description": "The blob's home — polymorphic over `(kb_contexts, kb_cogmaps)`, carried resolved. A blob\nhomed in a context or cogmap is visible to that home's readers\n(`blob-visibility-self-contained`); relations never widen this — the edge's home gates\nthe edge, not the blob (spec D3).",
      "$ref": "#/$defs/AnchorRef"
    },
    "originator_profile_id": {
      "description": "The home's originator. Absent ⇒ the projector COALESCEs it to the owner\n(originator≡owner), the `ResourceCreated` pattern.",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "owner_profile_id": {
      "description": "The blob's owning profile (the homes row's owner), the `ResourceCreated` shape.",
      "$ref": "#/$defs/ProfileId"
    }
  },
  "required": [
    "blob_id",
    "home",
    "owner_profile_id",
    "content_hash",
    "blob_pathname",
    "content_type",
    "content_bytes"
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
        "kb_blobs",
        "kb_events"
      ]
    },
    "BlobId": {
      "description": "A `kb_blobs.id` value — one immutable, content-addressed binary blob, homed like a\nresource and related to resources by edges (spec: binary blobs, 2026-09-01).",
      "type": "string",
      "format": "uuid"
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}$JS$::jsonb, 1, 'domain')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema,
      schema_version = EXCLUDED.schema_version,
      category       = EXCLUDED.category;

-- Reads ONLY the payload — no bytes, no sidecar argument: the bytes are external and were
-- verified present by the Rust wrapper BEFORE this event was appended (D4). Get-or-create on
-- the hash (D2): the second commit's event is provenance, its projection a no-op, and the
-- FIRST home stands — dedup binds the blob to its first committer's home; reaching another
-- audience is a relation (D3), never a second home.
CREATE FUNCTION _project_blob_committed(p_event uuid, p_payload jsonb)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE v_id       uuid := (p_payload->>'blob_id')::uuid;
        v_hash     text := p_payload->>'content_hash';
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_inserted uuid;
BEGIN
    INSERT INTO kb_blobs (id, content_hash, blob_pathname, content_type, content_bytes,
                          asserted_by_event_id, last_event_id, created)
    VALUES (v_id, v_hash, p_payload->>'blob_pathname', p_payload->>'content_type',
            (p_payload->>'content_bytes')::bigint,
            p_event, p_event, v_occurred)
    ON CONFLICT (content_hash) DO NOTHING
    RETURNING id INTO v_inserted;

    IF v_inserted IS NOT NULL THEN
        INSERT INTO kb_blob_homes (blob_id, anchor_table, anchor_id,
                                   originator_profile_id, owner_profile_id, created)
        VALUES (v_inserted,
                p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid,
                COALESCE((p_payload->>'originator_profile_id')::uuid,
                         (p_payload->>'owner_profile_id')::uuid),
                (p_payload->>'owner_profile_id')::uuid,
                v_occurred);
    ELSE
        SELECT id INTO v_inserted FROM kb_blobs WHERE content_hash = v_hash;
    END IF;

    RETURN ARRAY[v_inserted];
END;
$$;

-- Wrapper: enforce what SQL can enforce (D1 addressing, D4 hash-not-bytes, D9 cap + allowlist
-- with a refusal that names its vocabulary), then append + project. The provider-existence gate
-- lives Rust-side (commit_blob_with) — SQL cannot ask the provider. Same shape as
-- data_artifact_commit: validate, resolve nothing (home and owner are identity-as-input in the
-- payload), _event_append, project.
--
-- max_bytes and allowlist are CONFIGURATION passed in by the caller (D7/D9: the cap is config,
-- not code) — so the enforcement and the refusal come from the same values the operator set.
CREATE FUNCTION blob_commit(p_payload jsonb, p_emitter uuid,
                            p_max_bytes bigint, p_allowlist text[],
                            p_metadata jsonb DEFAULT '{}'::jsonb,
                            p_invocation uuid DEFAULT NULL::uuid,
                            p_correlation uuid DEFAULT NULL::uuid)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE v_hash  text := p_payload->>'content_hash';
        v_type  text := p_payload->>'content_type';
        v_bytes bigint := (p_payload->>'content_bytes')::bigint;
        v_path  text := p_payload->>'blob_pathname';
        v_home  text := p_payload#>>'{home,table}';
        v_owner uuid := (p_payload->>'owner_profile_id')::uuid;
        v_ev    uuid;
BEGIN
    -- The split is enforced absolutely: data_artifact_commit at least carries a p_content
    -- sidecar; a blob's bytes ride NO argument at all. A payload that smuggled them would be
    -- written verbatim into kb_events by _event_append, silently defeating hash-not-bytes.
    IF p_payload ? 'content' OR p_payload ? '__content'
       OR p_payload ? 'bytes' OR p_payload ? '__bytes' THEN
        RAISE EXCEPTION 'blob_commit: the event payload carries the hash, never the bytes — '
                        'the bytes live at the content-addressed pathname in object storage';
    END IF;

    -- The commit's vocabulary: home (context or cogmap anchor) + owner. The homes INSERT would
    -- fail on a null with a constraint message that names none of this.
    -- N1 arm (2026-09-03 review, N6): the home half must be an OR-of-neither — the original
    -- `IS NOT DISTINCT FROM 'kb_contexts' AND IS NOT DISTINCT FROM 'kb_cogmaps'` is a
    -- contradiction, so this RAISE could never fire on the home axis and a present-but-wrong
    -- home (identity-as-input: any event writer can send one) fell through to the projector's
    -- DDL CHECK — a scrubbed 5xx instead of this vocabulary refusal. The Rust `parse_home` and
    -- the kb_blobs CHECK (20260903000060) stay the outer and inner layers; this is the middle
    -- one, and it has to be live for the direct-write path to hear the same voice.
    IF ((v_home IS DISTINCT FROM 'kb_contexts' AND v_home IS DISTINCT FROM 'kb_cogmaps')
        OR (p_payload#>>'{home,id}')::uuid IS NULL OR v_owner IS NULL) THEN
        RAISE EXCEPTION 'blob_commit: a blob needs a home (a kb_contexts or kb_cogmaps anchor) '
                        'and an owner_profile_id — got home table %, owner %',
                        COALESCE(v_home, '<null>'), COALESCE(v_owner::text, '<null>');
    END IF;

    IF v_hash IS NULL THEN
        RAISE EXCEPTION 'blob_commit: content_hash is required — the ledger carries the hash, '
                        'not the bytes';
    END IF;

    -- Content addressing is enforced, not assumed (D1): the pathname IS the hash's address.
    IF v_path IS DISTINCT FROM (left(v_hash, 2) || '/' || v_hash) THEN
        RAISE EXCEPTION 'blob_commit: blob_pathname must be the content-addressed path '
                        '"<hash[0:2]>/<hash>" — got %, expected % for hash %',
                        COALESCE(v_path, '<null>'), left(v_hash, 2) || '/' || v_hash, v_hash;
    END IF;

    -- D9: the refusal teaches its vocabulary — the cap first.
    IF v_bytes IS NULL OR v_bytes > p_max_bytes THEN
        RAISE EXCEPTION 'blob_commit: content_bytes % exceeds the configured per-blob cap % — '
                        'the cap is configuration; use the segmented upload path for larger '
                        'blobs', COALESCE(v_bytes, -1), p_max_bytes;
    END IF;

    -- D9: ... then the allowlist. Names what is in force, not merely that something failed.
    IF v_type IS NULL OR NOT (v_type = ANY (p_allowlist)) THEN
        RAISE EXCEPTION 'blob_commit: content_type % is not admitted — the allowlist in force '
                        'is %', COALESCE(v_type, '<null>'), array_to_string(p_allowlist, ', ');
    END IF;

    v_ev := _event_append('blob_committed', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_blob_committed(v_ev, p_payload);
END;
$$;

COMMENT ON FUNCTION blob_commit(jsonb, uuid, bigint, text[], jsonb, uuid, uuid) IS
'commit-wrapper for blobs: enforces hash-not-bytes (no bytes argument exists — a payload that
smuggled them would ride the ledger verbatim), D1 content addressing (blob_pathname IS
<hash[0:2]>/<hash>), and the D9 cap + content-type allowlist — p_max_bytes and p_allowlist are
CALLER-PASSED CONFIGURATION, the same operator values the service enforces Rust-side, not
constants. The home gate is a live OR-of-neither (20260903000020 header, N6 note). Provider
existence is verified Rust-side BEFORE this appends; this function appends the typed
blob_committed event and projects it.';

SELECT declare_migration(
    20260903000020,
    'additive',
    'kb_blobs (hash-UNIQUE, uuidv7 identity-as-input, metadata only — bytes external at the content-addressed pathname) + kb_blob_homes (the kb_resource_homes shape); kb_edges endpoint CHECKs admit kb_blobs (D3 — and ONLY there: properties, membership and region CHECKs stay closed); kb_blob_files DROPPED in the same migration (D8, zero production readers); the typed blob_committed event (domain, schemars payload_schema per the data_artifact_committed precedent) with _project_blob_committed (get-or-create on the hash; the first home stands) and blob_commit (hash-not-bytes enforced absolutely — no bytes argument exists; D9 cap + allowlist refusals name their vocabulary). Provider existence is verified Rust-side before the event is appended (D4). Design: temper-artifacts specs/2026-09-01-binary-blobs-design.md.'
);
