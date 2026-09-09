-- The erasure act's vocabulary (spec 2026-08-31, temper-artifacts/specs/
-- 2026-08-31-erasure-act-design.md; task 01a0577c-f115-7aa0-8113-ba91e6c455ce, Beat 1).
-- Registers the three event types, creates the hash-keyed erased-content set, and adds the
-- kb_profiles tombstone marker. Execution lands with the later beats; this migration only
-- makes the vocabulary and the refusal set EXIST.
--
-- THE CONSTRAINTS A LATER EDIT MUST NOT BREAK:
--
--   * CATEGORY IS SPELLED HERE, ONCE. `kb_events_category_matches_type` is ON UPDATE RESTRICT
--     and the append-only trigger refuses reclassification (the 20260720000020 precedent):
--     `principal_erased` / `principal_erasure_refused` are `admin` — NULL-anchored by
--     `kb_events_admin_is_unanchored`, the cognition firewall — and `blob_erased` is
--     `domain`, the only category the `blob_delete` wrapper's guard admits. One shot.
--   * THE EMPTYING SHAPE IS SHARED, THE VOCABULARY IS NOT. `blob_erased` fires ONLY through
--     the substrate's `blob_delete(p_event_type, …)` wrapper and `_project_blob_deleted`
--     projector, untouched here. The emptied row keeps the D5.2 shape alone:
--     `content_type IS NULL` stays the ONLY row-shape marker of emptiness — any second
--     marker of WHICH act emptied a row is a violation; the ledger alone tells delete from
--     erasure. kb_blobs gains NO column.
--   * kb_erased_content IS HASH-KEYED AND REBUILDABLE FROM `principal_erased` PAYLOADS ALONE
--     (derive-don't-remember, D4): the content_hash is the key every reader already shares,
--     the write path refuses through it, and no row id or join-key shape may enter it.
--     Membership is the fact; the FIRST admitting event is attributed (the projector's
--     ON CONFLICT DO NOTHING), which is what makes a ledger-order rebuild byte-identical.
--   * tombstoned_at IS AN ACT MARKER, NOT AN IDENTIFIER (D5): it records WHEN the pseudonym
--     broke. The nulling/sentelling of the identifiers themselves happens at erase time in
--     the erasure execution, never in this migration — and never by this column.
--
-- The payload JSON below is GENERATED, NOT AUTHORED — copied byte-for-byte from
-- crates/temper-substrate/tests/fixtures/payloads/*.v1.schema.json, emitted by
-- `UPDATE_SCHEMA=1 cargo make test-schema` (package-scoped -p temper-substrate). Editing it
-- here desynchronizes repo, registry, and Rust types; the pairing test
-- (payload_schema.rs::the_migration_literal_matches_the_committed_fixture) pins the seam.
--
-- Additive: a new table, a nullable column, and new registry rows — no existing column,
-- constraint or function is altered, and an old binary reads every pre-existing row unchanged.

INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('principal_erased', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "PrincipalErased",
  "description": "`principal_erased` — the ONE admin event of a completed erasure (erasure spec D1).\n\nONE TYPE for identity erasure and content erasure: the distinction rides the per-target\noutcomes as data, never the type boundary — two types would cost the pairing (one request,\none operator, one reference) and double registration for no additional guarantee. The\nsubject is the PSEUDONYM the act itself broke; the event never re-identifies — no name, no\nemail, no case description. The request reference does not ride here: it lives on\n`kb_events.\"references\"`, the apparatus this act owns.\n\nThe redacted set is keyed on CONTENT HASH (D2) — never row ids, which would collide with\nthe trail functions' join-key shapes. The erased-content set (D4) rebuilds from these\npayloads alone.",
  "type": "object",
  "properties": {
    "actor": {
      "description": "The acting operator, distinguishable from the subject — self-serve vs administrative\nis a comparison the auditor needs. `None` only where no actor exists to name; inventing\none would put a fabricated attribution on the ledger (the standing-event precedent).",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "propagated_to_clients": {
      "description": "The propagation fact, DISTINCT from the server fact (erasure spec, payload\nrequirements): this event records \"gone from the server\"; `false` must never read as\n\"gone from the clients\". `false` until the propagation protocol exists (D3) — the\nfield reserves the fact; the wire task specifies its vocabulary.",
      "type": "boolean",
      "default": false
    },
    "redacted_hashes": {
      "description": "The redacted set (D2): bare sha256 hex, exactly as `content_hash` carries it — the key\nevery reader already shares and no trail join-key shape can match.",
      "type": "array",
      "items": {
        "type": "string"
      }
    },
    "subject_id": {
      "description": "The pseudonym — `kb_profiles.id`. It survives; its identifying power does not (D5).",
      "type": "string",
      "format": "uuid"
    },
    "subject_table": {
      "$ref": "#/$defs/AnchorTable"
    },
    "targets": {
      "description": "Per-target outcomes and the named remainder (D6's accepted-in-part arm): anything\nunhonourable is named here. Partial completion is data, never a silent success.",
      "type": "array",
      "items": {
        "$ref": "#/$defs/ErasureTargetOutcome"
      }
    }
  },
  "required": [
    "subject_table",
    "subject_id"
  ],
  "$defs": {
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
    "ErasureTargetOutcome": {
      "description": "One target of a completed erasure and what happened to it (erasure spec, \"per-target\noutcomes\"). The target names itself the way the personal-data manifest does — `table` or\n`table.column`; the outcome is the act's own record of what redaction applied. Deliberately\nopen-textured in v1: ceilings are DATA, not types (D1), and the per-target vocabulary is the\nexecution build's to pin. `unhonourable_scope` outcomes land here, never silent.",
      "type": "object",
      "properties": {
        "outcome": {
          "description": "What the act did to it (erased / sentinel-scrubbed / accepted-in-part / …).",
          "type": "string"
        },
        "target": {
          "description": "Manifest identity of the target (`kb_profiles.display_name`, `kb_teams.slug`, …).",
          "type": "string"
        }
      },
      "required": [
        "target",
        "outcome"
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}
$JS$::jsonb, 1, 'admin'),
  ('principal_erasure_refused', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "PrincipalErasureRefused",
  "description": "`principal_erasure_refused` — the negative face of the erasure act (erasure spec D6).\n\nSame subject spelling as [`PrincipalErased`] (`subject_table` / `subject_id`, never the\ntrail's join-key shapes); the operator is distinguishable from the subject exactly as\nthere. One type with a reason code — three outcomes, not three types.",
  "type": "object",
  "properties": {
    "actor": {
      "description": "Who ATTEMPTED the erasure — an attempt leaves a trail too.",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "detail": {
      "description": "The named unhonourable part, or the obligation held — the reason's evidence.",
      "type": [
        "string",
        "null"
      ]
    },
    "reason": {
      "$ref": "#/$defs/ErasureRefusalReason"
    },
    "subject_id": {
      "type": "string",
      "format": "uuid"
    },
    "subject_table": {
      "$ref": "#/$defs/AnchorTable"
    }
  },
  "required": [
    "subject_table",
    "subject_id",
    "reason"
  ],
  "$defs": {
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
    "ErasureRefusalReason": {
      "description": "The closed refusal vocabulary for `principal_erasure_refused` (erasure spec D6). A refused\nattempt to erase a person is exactly the event an operator later needs, and the reason code\nis the WHY the subject receives.",
      "oneOf": [
        {
          "description": "The caller lacks erasure standing.",
          "type": "string",
          "const": "unauthorized"
        },
        {
          "description": "The scope cannot be honoured in full; the unhonourable part is named in `detail`\n(accepted-in-part lands here, never silent).",
          "type": "string",
          "const": "unhonourable_scope"
        },
        {
          "description": "The system holds the data under an obligation, named in `detail`.",
          "type": "string",
          "const": "independent_obligation"
        }
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}
$JS$::jsonb, 1, 'admin'),
  ('blob_erased', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "BlobErased",
  "description": "Erase one blob — the erasure act's OWN strike vocabulary, fired through the SAME\n`blob_delete` wrapper and `_project_blob_deleted` projector as [`BlobDeleted`] (ruled\n2026-09-06: the emptying shape and the byte fate are shared, the vocabulary never is).\nIdentity-only, the exact `BlobDeleted` shape: the envelope carries the home (producing\nanchor), the actor (`emitter_entity_id`), and the time (`occurred_at`).\n\nThe row empties into the IDENTICAL D5.2 shape — there is deliberately no row-shape marker\nof which act emptied it, and the ledger alone tells delete from erasure. Attribution dies\nhere (never at a delete): the pseudonym break is the erasure act's work, not this event's\nrow write. The hash stays — it is the erased-content set's key (D4), which refuses\nre-admission of the erased bytes.",
  "type": "object",
  "properties": {
    "blob_id": {
      "$ref": "#/$defs/BlobId"
    }
  },
  "required": [
    "blob_id"
  ],
  "$defs": {
    "BlobId": {
      "description": "A `kb_blobs.id` value — one immutable, content-addressed binary blob, homed like a\nresource and related to resources by edges (spec: binary blobs, 2026-09-01).",
      "type": "string",
      "format": "uuid"
    }
  }
}
$JS$::jsonb, 1, 'domain')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema,
      schema_version = EXCLUDED.schema_version,
      category       = EXCLUDED.category;

-- The erased-content set (spec D4): the materialized, projector-maintained refusal set the
-- content-admitting paths consult BEFORE writing prose or re-embedding. Hash-keyed — the
-- content_hash is the join key every reader already shares (D2) — and rebuildable from
-- `principal_erased` payloads alone: replay walks the ledger in event order and the FIRST
-- admitting event keeps attribution (ON CONFLICT DO NOTHING), so a rebuild is byte-identical
-- to the live set. The erasure execution (a later beat of 01a0577c) maintains it; nothing
-- reads or writes it before then.
CREATE TABLE kb_erased_content (
    content_hash        TEXT PRIMARY KEY,
    erased_by_event_id  UUID NOT NULL REFERENCES kb_events(id)
);

COMMENT ON TABLE kb_erased_content IS
'The erased-content set (erasure spec D4, created 20260909000010): one row per content hash
the erasure act has redacted — the refusal a stale-laptop sync hits (a re-admitting write
refuses THROUGH this set, by hash; erasure is a refusal, not an absence). Derive-don''t-
remember: rebuildable from `principal_erased` payloads alone, first admitting event
attributed, so a ledger-order replay reproduces it exactly. Maintained by the erasure
execution; the hash is the same sha256 hex `kb_blobs.content_hash` and the chunk/block
content rows carry.';

-- The profile tombstone marker (spec D5): WHEN the pseudonym broke. The act marker only —
-- the nulling/sentelling of handle/display_name/email/preferences happens at erase time in
-- the erasure execution; this column never carries it. The UUID stays: it IS the pseudonym,
-- and every reference-class column pointing at it stops identifying anyone the moment the
-- mapping breaks.
ALTER TABLE kb_profiles ADD COLUMN tombstoned_at TIMESTAMPTZ;

COMMENT ON COLUMN kb_profiles.tombstoned_at IS
'Erasure act marker (spec D5, added 20260909000010): set at erase time when the pseudonym
breaks. Not an identifier and not the redaction itself — the identifier nulling happens in
the same act, by the erasure execution; this column records only that it happened, so the
pseudonym''s surviving references stay resolvable and non-identifying.';

SELECT declare_migration(
    20260909000010,
    'additive',
    'The erasure act''s vocabulary (spec 2026-08-31, Beat 1 of task 01a0577c): registers principal_erased + principal_erasure_refused (category admin, NULL-anchored — the cognition firewall) and blob_erased (category domain — the only vocabulary the blob_delete wrapper''s guard admits) with their generated payload schemas, one shot at category per the RESTRICT/append-only precedent; creates kb_erased_content, the hash-keyed erased-content set (spec D4 — projector-maintained by the later execution beat, rebuildable from principal_erased payloads alone); adds kb_profiles.tombstoned_at, the act marker (spec D5 — the identifier nulling itself happens at erase time, not here). Additive: new table, nullable column, new registry rows; nothing existing is altered.'
);
