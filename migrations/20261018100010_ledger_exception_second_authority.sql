-- The ledger exception learns a second authority: resource_scrubbed (field-grain scrub, build Task 3).
--
-- Spec: temper-artifacts specs/2026-10-09-field-grain-scrub-design.md S1, S3, S4, S6, S7, read beside
-- specs/2026-09-28-resource-erasure-design.md D3 and D4. Plan:
-- plans/2026-10-09-field-grain-scrub.md, Task 3.
--
-- Cut 2 (20261009100000) made kb_events_append_only a verifier that admits exactly one rewrite: a
-- resource erasure's sentinel redaction, authorised by the erasure's own record in its own
-- transaction. This migration adds the field scrub as a second authorising event type, with its own
-- precondition (the subject is NOT erased) and its own renamed-key sentinels. Nothing the verifier
-- admitted before changes, and nothing is admitted without an authorising event of the row's own
-- authority, written in this transaction, that lists the event and the path.
--
--   1. kb_event_field_redactions gains `authority` ('erasure' | 'scrub'), and its key becomes
--      (event_id, path, authority) under the SAME constraint name (S6.1).
--   2. resource_scrubbed is registered; resource_erasure_refused is re-registered from the fixture
--      Task 1 regenerated (the `field_scrub` act and the two new recorded reasons).
--   3. _erasure_sentinel_admits takes the authority (S6.2, S7).
--   4. The redaction-row projector is one body for both authorities;
--      _project_resource_scrubbed_redactions is its scrub entry (S6.4).
--   5. The field scrub's family and prior predicates, defined ONCE (S1, S3). The act's plan (Task 4)
--      calls them; the verifier below calls them; neither restates them.
--   6. kb_events_append_only admits a change when ANY row for that (event, path) authorises it (S6.2).
--   7. kb_events_redaction_in_trail adds the scrub's family-and-prior check per statement (S6.3).

-- ---------------------------------------------------------------------------
-- Section 1. kb_event_field_redactions records its authority (S6.1).
-- ---------------------------------------------------------------------------
-- The DEFAULT is the backfill: ADD COLUMN with a constant default stamps every existing row without
-- an UPDATE, which the table's own append-only trigger would refuse. It stays the default: every row
-- written before this migration, and every INSERT that names no authority (a binary that predates
-- this migration names none), is an erasure's. Each projector writes its authority explicitly.
--
-- The key keeps its NAME, kb_event_field_redactions_pkey: resource_erasure_service.rs matches it by
-- name (REDACTIONS_KEY) to retry a concurrent completion (erasure, cut 2 PR 3, ruling 9). A second
-- erasure of the same path still collides on it; an erasure of a path a scrub already rewrote does
-- not (S6.1).
ALTER TABLE kb_event_field_redactions
    ADD COLUMN authority text NOT NULL DEFAULT 'erasure'
        CONSTRAINT kb_event_field_redactions_authority_check CHECK (authority IN ('erasure', 'scrub'));

ALTER TABLE kb_event_field_redactions
    DROP CONSTRAINT kb_event_field_redactions_pkey,
    ADD CONSTRAINT kb_event_field_redactions_pkey PRIMARY KEY (event_id, path, authority);

COMMENT ON TABLE kb_event_field_redactions IS
$c$Which path of which event an authorising admin act rewrote (resource erasure spec D3; field-grain
scrub spec S6.1): one row per (event, path) of the act's redacted_fields, redacted_by the act's
event, under its authority: `erasure` for resource_erased, `scrub` for resource_scrubbed. The ledger
authorizes its own redaction: kb_events_append_only admits an UPDATE only at paths these rows name,
and only under a row whose event is of its authority's type. Keyed (event_id, path, authority), so
an erasure of a path a scrub already rewrote records its own row. Append-only; a projection, rebuilt
by replay.$c$;

COMMENT ON COLUMN kb_event_field_redactions.authority IS
$c$The authorising event's type: `erasure` (resource_erased, whose subject must be erased) or `scrub`
(resource_scrubbed, whose subject must not be). Written by that type's projector. A structural
token, never a value.$c$;

-- ---------------------------------------------------------------------------
-- Section 2. The event types.
-- ---------------------------------------------------------------------------
-- The payload JSON below is GENERATED, NOT AUTHORED: copied byte for byte from
-- crates/temper-substrate/tests/fixtures/payloads/{resource_scrubbed,resource_erasure_refused}.v1.schema.json,
-- emitted by `UPDATE_SCHEMA=1 cargo make test-schema` (package-scoped -p temper-substrate). The
-- pairing test (payload_schema.rs::the_migration_literal_matches_the_committed_fixture) pins the
-- seam, pairing this file's literals with its fixture list in order.
--
-- resource_scrubbed is `admin`, NULL-anchored by kb_events_admin_is_unanchored, as every act of the
-- erasure family is (20260929000010). Its category is spelled here once: kb_events_category_matches_type
-- is ON UPDATE RESTRICT.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('resource_scrubbed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ResourceScrubbed",
  "description": "`resource_scrubbed` — the field scrub (field-grain scrub spec S1, S4): every prior value of a\nresource's title, origin URI or one property family (or every family's prior history) redacted\nfrom the ledger and the projection, on a resource that survives. With `cleared`, today's value\nwas cleared too, by ordinary events appended before this one in the act's transaction.\n\nKeyed `subject_table` (`kb_resources`) / `subject_id`, never `resource_id`, and the family by\n`field.family`, never `event_id` (the D1 join-key rule). It never carries a top-level `owner`:\n`_resource_erasure_trail_scope` joins on `payload->'owner'->>'id'`. Paths only, never values or\nkey text: the record of a redaction must not carry what was redacted.",
  "type": "object",
  "properties": {
    "actor": {
      "description": "The acting system admin.",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "cleared": {
      "description": "True when the act also cleared today's value. Absent means today's value was kept.",
      "type": "boolean"
    },
    "field": {
      "description": "The field scrubbed, and the property family's handle when the field is one family.",
      "$ref": "#/$defs/ScrubbedField"
    },
    "redacted_fields": {
      "description": "The ledger paths this act redacted to their sentinels, in the shape `resource_erased` uses.",
      "type": "array",
      "items": {
        "$ref": "#/$defs/RedactedEventFields"
      }
    },
    "subject_id": {
      "type": "string",
      "format": "uuid"
    },
    "subject_table": {
      "description": "Always `kb_resources`.",
      "$ref": "#/$defs/AnchorTable"
    }
  },
  "required": [
    "subject_table",
    "subject_id",
    "field"
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
    "EventId": {
      "description": "A `kb_events.id` value. Always UUIDv7 (time-sortable).",
      "type": "string",
      "format": "uuid"
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "RedactedEventFields": {
      "description": "The ledger paths of one event: redacted (`redacted_fields`) or named-and-unreached\n(`ledger_remainder`). ONE shape for both, so the cut-2 completion pass derives what it redacts\nfrom what cut 1 recorded without translating (resource erasure spec D12). The event is keyed\n`event`, never `event_id` — no trail join-key shape rides an admin payload (D1). Paths only,\nnever values: the record of a redaction must not carry what was redacted.",
      "type": "object",
      "properties": {
        "event": {
          "$ref": "#/$defs/EventId"
        },
        "paths": {
          "description": "JSON paths within that event's `payload` (or `metadata`), e.g. `title`, `origin_uri`.",
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "event",
        "paths"
      ]
    },
    "ScrubFieldKind": {
      "description": "The field a field scrub names. A field, never a value: no request or record carries the text\nbeing scrubbed.",
      "oneOf": [
        {
          "description": "The resource's title.",
          "type": "string",
          "const": "title"
        },
        {
          "description": "The resource's origin URI.",
          "type": "string",
          "const": "origin_uri"
        },
        {
          "description": "One resource-owned property family, named by its handle.",
          "type": "string",
          "const": "property"
        },
        {
          "description": "Every resource-owned property family's prior history. Never cleared.",
          "type": "string",
          "const": "properties"
        }
      ]
    },
    "ScrubbedField": {
      "description": "The field a field scrub acted on. `family` is the handle of a property family: the id of the\nfirst event that still carries the family's key text on the resource. It is present only for\n[`ScrubFieldKind::Property`]. Keyed `family`, never `event_id`: no trail join-key shape rides\nan admin payload (resource erasure D1).",
      "type": "object",
      "properties": {
        "family": {
          "anyOf": [
            {
              "$ref": "#/$defs/EventId"
            },
            {
              "type": "null"
            }
          ]
        },
        "kind": {
          "$ref": "#/$defs/ScrubFieldKind"
        }
      },
      "required": [
        "kind"
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin');

-- resource_erasure_refused gains the `field_scrub` act and the recorded reasons sentinel_collision
-- and projection_disagrees (field-grain scrub spec S4). Optional values only: every payload valid
-- before stays valid, and the version stays 1. The precedent is 20261003000210 Section 6.
UPDATE kb_event_types
   SET payload_schema = $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ResourceErasureRefused",
  "description": "`resource_erasure_refused` — the negative face of resource erasure and of the block history\nscrub (resource erasure spec D5, D11). Keyed on the resource, spelled as [`ResourceErased`]\nspells it: every reason is a fact about the resource, whichever act was refused.",
  "type": "object",
  "properties": {
    "act": {
      "description": "Which act was refused. Absent means [`ErasureAct::Erasure`].",
      "anyOf": [
        {
          "$ref": "#/$defs/ErasureAct"
        },
        {
          "type": "null"
        }
      ]
    },
    "actor": {
      "description": "Who attempted the act.",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "blocks": {
      "description": "The blocks a refused block history scrub named. Carried here, never in `subject_ids` or\n`detail`; empty for an erasure refusal.",
      "type": "array",
      "items": {
        "type": "string",
        "format": "uuid"
      }
    },
    "detail": {
      "description": "The reason's evidence, e.g. the task that owns map-grain charter erasure.",
      "type": [
        "string",
        "null"
      ]
    },
    "reason": {
      "$ref": "#/$defs/RecordedRefusalReason"
    },
    "subject_id": {
      "type": "string",
      "format": "uuid"
    },
    "subject_table": {
      "description": "Always `kb_resources`.",
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
    "ErasureAct": {
      "description": "Which act a `resource_erasure_refused` event refuses. Absent on the payload reads as\n[`ErasureAct::Erasure`].",
      "oneOf": [
        {
          "description": "The resource erasure act.",
          "type": "string",
          "const": "erasure"
        },
        {
          "description": "The block history scrub.",
          "type": "string",
          "const": "block_history_scrub"
        },
        {
          "description": "The field scrub: a resource's title, origin URI or property history redacted while the\nresource survives.",
          "type": "string",
          "const": "field_scrub"
        }
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "RecordedRefusalReason": {
      "description": "The closed refusal vocabulary `resource_erasure_refused` records (resource erasure spec D5,\nD11; field-grain scrub spec S4): every value either act family's doors answer with. A ledger\nvocabulary only; no door answers in it.",
      "oneOf": [
        {
          "description": "Retired: no path raises it. The value stays registered because removing one from a closed\nvocabulary is not additive.",
          "type": "string",
          "const": "unauthorized"
        },
        {
          "description": "A cogmap's telos/charter resource: map-grain erasure is its own act, named in `detail`.",
          "type": "string",
          "const": "charter_resource"
        },
        {
          "description": "Retired: no path raises it. The value stays registered because removing one from a closed\nvocabulary is not additive.",
          "type": "string",
          "const": "ingest_in_flight"
        },
        {
          "description": "The resource is already erased. Nothing in the projection changes and no second\n`resource_erased` is minted.",
          "type": "string",
          "const": "already_erased"
        },
        {
          "description": "The field scrub's sentinel text is already on the owner's ledger.",
          "type": "string",
          "const": "sentinel_collision"
        },
        {
          "description": "The field scrub found the latest title or origin URI event disagreeing with the projection.",
          "type": "string",
          "const": "projection_disagrees"
        }
      ]
    }
  }
}
$JS$::jsonb
 WHERE name = 'resource_erasure_refused';

-- ---------------------------------------------------------------------------
-- Section 3. The sentinel check takes the authority (S6.2, S7).
-- ---------------------------------------------------------------------------
-- DROP + CREATE: adding a DEFAULTed parameter is a new signature, and an overload left beside it
-- would make every six-argument call ambiguous. Its only caller is kb_events_append_only (Section 6),
-- a plpgsql body that resolves the name when it runs; no binary calls it.
DROP FUNCTION _erasure_sentinel_admits(text, uuid, jsonb, text[], jsonb, jsonb);

-- Whether p_new is the sentinel the verifier admits at one location (D3 condition 3), under one
-- authority. Under `erasure`, unchanged from 20261009100000: exact for every class D4 derives from
-- the event; by pattern for a property key (its n needs the owner's whole family; Witness 25 checks
-- it) and a remote source (its n needs the block's provenance, which the act's own rewrite is
-- changing as this runs), with the block id exact; a facet value by shape and by the original's
-- mark count.
--
-- Under `scrub` (S7): the title, origin URI, property value and doc type take erasure's exact
-- sentinels, so a later erasure finds those paths already done; a renamed key is
-- scrubbed-key-<handle>, a facet value's inner keys scrubbed-facet-<event id>-<position>, by
-- pattern. No other class is admitted under a scrub: the act never reaches a remote source, a label,
-- a reason, a scar or authorship. An erasure row never admits a scrub sentinel and a scrub row never
-- admits an erasure one. An unknown authority admits nothing.
CREATE FUNCTION _erasure_sentinel_admits(p_class text, p_event uuid, p_payload jsonb, p_at text[],
                                         p_old jsonb, p_new jsonb, p_authority text DEFAULT 'erasure')
RETURNS boolean
LANGUAGE plpgsql IMMUTABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_marks integer;
    v_scalar boolean;
BEGIN
    IF p_new IS NULL OR p_authority IS NULL OR p_authority NOT IN ('erasure', 'scrub') THEN
        RETURN false;
    END IF;
    IF p_authority = 'scrub'
       AND p_class NOT IN ('keep', 'title', 'origin-uri', 'property-value', 'doc-type',
                           'property-key', 'facet-value') THEN
        RETURN false;
    END IF;
    CASE p_class
        WHEN 'keep' THEN
            RETURN p_new = p_old;
        WHEN 'property-key' THEN
            IF p_authority = 'scrub' THEN
                RETURN jsonb_typeof(p_new) = 'string'
                   AND p_new #>> '{}' ~ '^scrubbed-key-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$';
            END IF;
            RETURN jsonb_typeof(p_new) = 'string' AND p_new #>> '{}' ~ '^erased-key-[1-9][0-9]*$';
        WHEN 'edge-label' THEN
            IF _erasure_label_is_kept(p_old) THEN
                RETURN p_new = p_old;
            END IF;
            RETURN jsonb_typeof(p_new) = 'string' AND p_new #>> '{}' ~ '^erased-label-[1-9][0-9]*$';
        WHEN 'remote-source-url' THEN
            IF NOT coalesce(_erasure_location_is_remote(p_payload, p_at), false) THEN
                RETURN p_new = p_old;
            END IF;
            RETURN jsonb_typeof(p_new) = 'string'
               AND p_new #>> '{}' ~ ('^erased:' || _erasure_location_block(p_payload, p_at)::text
                                     || ':[1-9][0-9]*$');
        WHEN 'facet-value' THEN
            SELECT count(*), bool_or(m.inner_key IS NULL)
              INTO v_marks, v_scalar
              FROM _facet_marks(p_old) m;
            IF v_scalar THEN
                RETURN p_new = to_jsonb('erased:' || p_event::text);
            END IF;
            RETURN jsonb_typeof(p_new) = 'object'
               AND (SELECT count(*) FROM jsonb_object_keys(p_new)) = v_marks
               AND NOT EXISTS (SELECT 1 FROM jsonb_each(p_new) kv
                                WHERE kv.key !~ CASE p_authority
                                          WHEN 'scrub'
                                          THEN '^scrubbed-facet-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}-[1-9][0-9]*$'
                                          ELSE '^erased-facet-[1-9][0-9]*$' END
                                   OR kv.value <> '"erased"'::jsonb);
        ELSE
            RETURN p_new = _erasure_sentinel_exact(p_class, p_event, p_payload);
    END CASE;
END;
$$;

-- ---------------------------------------------------------------------------
-- Section 4. The redaction-row projector: one body, two authorities (S6.1, S6.4).
-- ---------------------------------------------------------------------------
-- It refuses an event outside the subject's trail (the exception never crosses the resource
-- boundary, Q1) and a path the allowlist does not hold for that event's type, so the fence (D9), not
-- the event, decides what an act may name. Each row records the authority of the event projecting
-- it. One body, so the two acts' projections cannot drift; each refusal names the entry point the
-- caller called, so an erasure's messages are the ones 20261009100000 raised.
CREATE FUNCTION _project_field_redactions(p_event uuid, p_payload jsonb, p_authority text)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_name  text := CASE p_authority
                        WHEN 'erasure' THEN '_project_resource_erased_redactions'
                        WHEN 'scrub'   THEN '_project_resource_scrubbed_redactions'
                    END;
    v_entry jsonb;
    v_path  text;
    v_type  text;
    v_trail jsonb;
BEGIN
    IF v_name IS NULL THEN
        RAISE EXCEPTION '_project_field_redactions: event % names no known authority %', p_event, p_authority;
    END IF;
    -- The subject's trail, once: event id → true.
    SELECT coalesce(jsonb_object_agg(ts.event_id::text, true), '{}'::jsonb)
      INTO v_trail
      FROM _resource_erasure_trail_scope((p_payload ->> 'subject_id')::uuid) ts
     WHERE jsonb_array_length(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) > 0;
    FOR v_entry IN
        SELECT e FROM jsonb_array_elements(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) e
    LOOP
        SELECT et.name INTO v_type
          FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE ev.id = (v_entry ->> 'event')::uuid;
        IF v_type IS NULL THEN
            RAISE EXCEPTION '%: event % names a missing event %',
                v_name, p_event, v_entry ->> 'event';
        END IF;
        -- The replay walk projects the record as the ledger holds it, and a D14 order inversion
        -- can put a trail event's own block or edge after the act: checking the trail there would
        -- abort the whole replay. Live, kb_events_redaction_in_trail checks it again at the
        -- rewrite, so this check is the earlier of two.
        IF NOT v_trail ? (v_entry ->> 'event')
           AND coalesce(current_setting('temper.replaying', true), '') <> 'on' THEN
            RAISE EXCEPTION '%: event % names event %, which is not in its subject''s trail',
                v_name, p_event, v_entry ->> 'event';
        END IF;
        FOR v_path IN SELECT jsonb_array_elements_text(v_entry -> 'paths') LOOP
            IF NOT EXISTS (
                SELECT 1 FROM _erasure_redact_paths() r
                 WHERE r.path = v_path AND r.class <> 'keep'
                   AND (r.event_type = v_type
                        OR (r.event_type IS NULL AND v_path LIKE 'metadata.%'))) THEN
                RAISE EXCEPTION '%: event % names path % of a % event, which the allowlist does not hold',
                    v_name, p_event, v_path, v_type;
            END IF;
            INSERT INTO kb_event_field_redactions (event_id, path, redacted_by, authority)
            VALUES ((v_entry ->> 'event')::uuid, v_path, p_event, p_authority);
        END LOOP;
    END LOOP;
END;
$$;

-- Signature unchanged: the replay arm and resource_erasure_execute call it as before.
CREATE OR REPLACE FUNCTION _project_resource_erased_redactions(p_event uuid, p_payload jsonb)
RETURNS void
LANGUAGE sql
SET search_path = public, pg_temp
AS $$
    SELECT _project_field_redactions(p_event, p_payload, 'erasure');
$$;

CREATE FUNCTION _project_resource_scrubbed_redactions(p_event uuid, p_payload jsonb)
RETURNS void
LANGUAGE sql
SET search_path = public, pg_temp
AS $$
    SELECT _project_field_redactions(p_event, p_payload, 'scrub');
$$;

COMMENT ON FUNCTION _project_resource_scrubbed_redactions(uuid, jsonb) IS
$c$The resource_scrubbed projection (field-grain scrub spec S6.4, S8): one kb_event_field_redactions
row per (event, path) of redacted_fields, authority `scrub`. The same checks as the erasure's
projector, by the same body (_project_field_redactions): every named event is in the subject's
trail (skipped while temper.replaying is on), and every path is on the allowlist for its event's
type. Replay calls it at the event's position and applies nothing else.$c$;

-- ---------------------------------------------------------------------------
-- Section 5. The field scrub's family and prior predicates (S1, S3), defined once.
-- ---------------------------------------------------------------------------
-- p_keys, on each: the ORIGINAL property_key text by event id, for events whose kb_events row no
-- longer holds it. The act's plan (Task 4) reads the ledger before it rewrites anything and passes
-- nothing. The statement verifier (Section 7) runs after its statement's rewrite has renamed keys,
-- and passes each rewritten row's OLD key, so both evaluate the same predicate over the same
-- pre-rewrite ledger.

-- Every resource-owned property event of R, with its key text and its family's handle (S1): a
-- family is R's property events sharing one key text, and its handle is the lowest id still
-- carrying that text, stable as the family grows. resource_created belongs to the `doc_type` family
-- when it carries doc_type: its projector inserts the owner's doc_type row (20260626000001,
-- _project_resource_created), so R's doc_type family is first seen at create (erasure D4), and
-- clearing doc_type makes resource_created.doc_type prior (S2). Edge- and block-owned properties
-- are never a family (S1, *Declared limits*). Each arm reads an index of kb_events.
CREATE FUNCTION _field_scrub_property_events(p_resource uuid, p_keys jsonb DEFAULT NULL)
RETURNS TABLE(event_id uuid, event_type text, key_text text, handle uuid)
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    WITH ev AS (
        SELECT e.id, t.name AS event_type,
               coalesce(p_keys ->> e.id::text, e.payload ->> 'property_key') AS key_text
          FROM kb_events e
          JOIN kb_event_types t ON t.id = e.event_type_id
         WHERE ((e.payload -> 'owner') ->> 'id')::uuid = p_resource
           AND (e.payload -> 'owner') ->> 'table' = 'kb_resources'
           AND t.name IN ('property_set', 'property_asserted', 'property_unset')
        UNION ALL
        SELECT e.id, t.name, 'doc_type'
          FROM kb_events e
          JOIN kb_event_types t ON t.id = e.event_type_id
         WHERE (e.payload ->> 'resource_id')::uuid = p_resource
           AND t.name = 'resource_created'
           AND e.payload ->> 'doc_type' IS NOT NULL
    )
    SELECT ev.id, ev.event_type, ev.key_text,
           first_value(ev.id) OVER (PARTITION BY ev.key_text ORDER BY ev.id)
      FROM ev;
$$;

-- The events of one scrubbable field of R (S1):
--   title / origin_uri: every event of R carrying the field, the two event types the allowlist
--     holds it on (resource_created, resource_updated). A JSON null carries nothing:
--     _project_resource_updated COALESCEs it away;
--   property: the family whose handle is p_family. An id that is not a family's handle names no
--     family, and the set is empty;
--   properties: every family of R.
-- p_family is read only for `property`.
CREATE FUNCTION _field_scrub_family_events(p_resource uuid, p_kind text, p_family uuid,
                                           p_keys jsonb DEFAULT NULL)
RETURNS SETOF uuid
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT e.id
      FROM kb_events e
      JOIN kb_event_types t ON t.id = e.event_type_id
     WHERE p_kind IN ('title', 'origin_uri')
       AND (e.payload ->> 'resource_id')::uuid = p_resource
       AND t.name IN ('resource_created', 'resource_updated')
       AND e.payload ->> p_kind IS NOT NULL
    UNION ALL
    SELECT pe.event_id
      FROM _field_scrub_property_events(p_resource, p_keys) pe
     WHERE p_kind IN ('property', 'properties')
       AND (p_kind = 'properties' OR pe.handle = p_family);
$$;

-- Whether p_event's value at p_path is prior (S3), so a scrub of p_kind (and p_family) may redact
-- it. p_path NULL asks about the event's value as a whole. A path that is not the field's own
-- (title / origin_uri on their events; property_key and value on a property event; doc_type on
-- resource_created) is never prior: the scrub reaches only the field it names (S6.3).
CREATE FUNCTION _field_scrub_event_is_prior(p_resource uuid, p_kind text, p_family uuid, p_event uuid,
                                            p_path text DEFAULT NULL, p_keys jsonb DEFAULT NULL)
RETURNS boolean
LANGUAGE plpgsql STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_type     text;
    v_key      text;
    v_handle   uuid;
    v_boundary uuid;
BEGIN
    IF p_kind IN ('title', 'origin_uri') THEN
        -- S3, "Title and origin URI": the event producing today's value is the last event carrying
        -- the field in walk order (ORDER BY id); every earlier event carrying it is prior.
        RETURN (p_path IS NULL OR p_path = p_kind)
           AND EXISTS (SELECT 1 FROM _field_scrub_family_events(p_resource, p_kind, NULL) f
                        WHERE f = p_event)
           AND EXISTS (SELECT 1 FROM _field_scrub_family_events(p_resource, p_kind, NULL) f
                        WHERE f > p_event);
    END IF;
    IF p_kind IS NULL OR p_kind NOT IN ('property', 'properties') THEN
        RETURN false;
    END IF;

    SELECT pe.event_type, pe.key_text, pe.handle
      INTO v_type, v_key, v_handle
      FROM _field_scrub_property_events(p_resource, p_keys) pe
     WHERE pe.event_id = p_event;
    IF NOT FOUND OR (p_kind = 'property' AND v_handle IS DISTINCT FROM p_family) THEN
        RETURN false;
    END IF;
    IF p_path IS NOT NULL AND p_path <> ALL (CASE v_type
                                                 WHEN 'resource_created' THEN ARRAY['doc_type']
                                                 WHEN 'property_unset'   THEN ARRAY['property_key']
                                                 ELSE ARRAY['property_key', 'value']
                                             END) THEN
        RETURN false;
    END IF;

    IF p_path = 'property_key' THEN
        -- S3, "Property key text: unset first": key text is scrubbable for every event of the
        -- family at or before its latest property_unset, the unset included. A family never unset
        -- has no scrubbable key text.
        SELECT pe.event_id INTO v_boundary
          FROM _field_scrub_property_events(p_resource, p_keys) pe
         WHERE pe.handle = v_handle AND pe.event_type = 'property_unset'
         ORDER BY pe.event_id DESC
         LIMIT 1;
        RETURN v_boundary IS NOT NULL AND p_event <= v_boundary;
    END IF;

    -- S3, "Property values": an event is prior when none of the kb_properties rows it asserted
    -- (asserted_by_event_id) is live. A property_unset asserts none. Read as the projection stands
    -- now, after any clearing events (S6.3).
    IF EXISTS (SELECT 1 FROM kb_properties p
                WHERE p.owner_table = 'kb_resources' AND p.owner_id = p_resource
                  AND NOT p.is_folded AND p.asserted_by_event_id = p_event) THEN
        RETURN false;
    END IF;
    IF v_key = 'facet' THEN
        -- S3, "Facets": every mark folded (above) AND before the latest whole-facet fold, a
        -- property_set or property_unset of `facet`, both of which fold by property_key whatever
        -- the inner key. After it, an inner-key fold could leave a renamed mark live on replay.
        SELECT pe.event_id INTO v_boundary
          FROM _field_scrub_property_events(p_resource, p_keys) pe
         WHERE pe.handle = v_handle AND pe.event_type IN ('property_set', 'property_unset')
         ORDER BY pe.event_id DESC
         LIMIT 1;
        RETURN v_boundary IS NOT NULL AND p_event < v_boundary;
    END IF;
    RETURN true;
END;
$$;

COMMENT ON FUNCTION _field_scrub_event_is_prior(uuid, text, uuid, uuid, text, jsonb) IS
$c$Whether one event's value at one path is prior under field-grain scrub spec S3, for a scrub of
p_kind (title, origin_uri, property with its family handle, or properties): the title or origin URI
before the last event carrying it; a property value none of whose asserted rows is live; key text at
or before the family's latest property_unset; a facet value fully folded and before the latest
whole-facet fold. The ONE definition: the act's plan and kb_events_redaction_in_trail both call it.
p_keys supplies original key text for events a statement has already renamed.$c$;

-- ---------------------------------------------------------------------------
-- Section 6. The row verifier admits each authority under its own precondition (S6.2).
-- ---------------------------------------------------------------------------
-- 20261009100000's verifier, with one change: a changed location is admitted when ANY redaction row
-- for that (event, path) authorises it, where before there could be only one row. A row authorises
-- when its event is of the row's authority's type (resource_erased for `erasure`, resource_scrubbed
-- for `scrub`), was written in THIS transaction (created = now(), single-use, see 20261009100000),
-- lists this event and path in its own redacted_fields, meets its type's precondition (erasure: the
-- subject is erased; scrub: the subject is not), and the new value is the sentinel
-- _erasure_sentinel_admits admits under that authority. The rest is unchanged: DELETE raises; no
-- column but payload and metadata may change; every named location is set aside on both sides and
-- nothing else may differ; every refusal raises 'event ledger is append-only'.
CREATE OR REPLACE FUNCTION kb_events_append_only() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_type   text;
    v_path   text;
    v_loc    record;
    v_class  text;
    v_meta   boolean;
    v_rows   integer := 0;
    v_new    jsonb;
    v_old_p  jsonb := OLD.payload;
    v_new_p  jsonb;
    v_old_m  jsonb := OLD.metadata;
    v_new_m  jsonb;
BEGIN
    IF TG_OP <> 'UPDATE' THEN
        RAISE EXCEPTION 'event ledger is append-only';
    END IF;
    v_new_p := NEW.payload;
    v_new_m := NEW.metadata;
    IF (to_jsonb(OLD) - 'payload' - 'metadata') IS DISTINCT FROM (to_jsonb(NEW) - 'payload' - 'metadata') THEN
        RAISE EXCEPTION 'event ledger is append-only';
    END IF;
    SELECT et.name INTO v_type FROM kb_event_types et WHERE et.id = OLD.event_type_id;
    FOR v_path IN
        SELECT DISTINCT r.path FROM kb_event_field_redactions r WHERE r.event_id = OLD.id
    LOOP
        v_rows := v_rows + 1;
        v_class := _erasure_path_class(v_type, v_path, OLD.payload);
        IF v_class IS NULL THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
        v_meta := v_path LIKE 'metadata.%';
        FOR v_loc IN
            SELECT x.at, x.val
              FROM _erasure_expand(CASE WHEN v_meta THEN OLD.metadata ELSE OLD.payload END,
                                   CASE WHEN v_meta THEN substr(v_path, 10) ELSE v_path END) x
        LOOP
            v_new := (CASE WHEN v_meta THEN NEW.metadata ELSE NEW.payload END) #> v_loc.at;
            IF v_new IS DISTINCT FROM v_loc.val THEN
                IF NOT EXISTS (
                    SELECT 1
                      FROM kb_event_field_redactions r
                      JOIN kb_events e ON e.id = r.redacted_by
                      JOIN kb_event_types t ON t.id = e.event_type_id
                     WHERE r.event_id = OLD.id AND r.path = v_path
                       AND t.name = CASE r.authority
                                        WHEN 'erasure' THEN 'resource_erased'
                                        WHEN 'scrub'   THEN 'resource_scrubbed'
                                    END
                       AND e.created = now()
                       AND e.payload -> 'redacted_fields' @> jsonb_build_array(jsonb_build_object(
                               'event', OLD.id::text, 'paths', jsonb_build_array(v_path)))
                       AND EXISTS (SELECT 1 FROM kb_resources s
                                    WHERE s.id = (e.payload ->> 'subject_id')::uuid
                                      AND CASE r.authority
                                              WHEN 'erasure' THEN s.erased_at IS NOT NULL
                                              WHEN 'scrub'   THEN s.erased_at IS NULL
                                          END)
                       AND _erasure_sentinel_admits(v_class, OLD.id, OLD.payload, v_loc.at,
                                                    v_loc.val, v_new, r.authority)) THEN
                    RAISE EXCEPTION 'event ledger is append-only';
                END IF;
            END IF;
            IF v_meta THEN
                v_old_m := jsonb_set(v_old_m, v_loc.at, 'null'::jsonb, false);
                v_new_m := jsonb_set(v_new_m, v_loc.at, 'null'::jsonb, false);
            ELSE
                v_old_p := jsonb_set(v_old_p, v_loc.at, 'null'::jsonb, false);
                v_new_p := jsonb_set(v_new_p, v_loc.at, 'null'::jsonb, false);
            END IF;
        END LOOP;
    END LOOP;
    IF v_rows = 0
       OR v_old_p IS DISTINCT FROM v_new_p
       OR v_old_m IS DISTINCT FROM v_new_m THEN
        RAISE EXCEPTION 'event ledger is append-only';
    END IF;
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION kb_events_append_only() IS
$c$The ledger's guard (resource erasure spec D3; field-grain scrub spec S6.2). DELETE always raises.
An UPDATE is admitted only as an authorised redaction in the authorising act's own transaction: no
column but payload and metadata changes; every changed location lies under a path a
kb_event_field_redactions row names, and some row for that path authorises it. A row authorises
when its event is of its authority's type (resource_erased for `erasure`, resource_scrubbed for
`scrub`), was written in this transaction, lists the event and the path, meets its precondition
(erasure: the subject is erased; scrub: the subject is not erased), and the location holds the
sentinel _erasure_sentinel_admits admits under that authority; nothing else changed.
kb_events_redaction_in_trail adds, per statement, that each changed event is in the act's subject's
trail and, for a scrub, in the scrubbed field and prior. Anything else raises 'event ledger is
append-only', as it always has.$c$;

-- ---------------------------------------------------------------------------
-- Section 7. The statement verifier adds the scrub's family and prior check (S6.3).
-- ---------------------------------------------------------------------------
-- The first loop is 20261009100000's trail check, unchanged. It reads every redaction row this
-- transaction's acts wrote, whatever its authority, so it already holds a scrub to its subject's
-- trail (S6.3's first condition). The second loop runs once per resource_scrubbed record written in
-- this transaction: every row of the record on an event this statement changed must name a path
-- that _field_scrub_event_is_prior holds prior for the record's field, family and subject. That one
-- predicate is S6.3's remaining conditions: the event is in the named field or family (owner
-- kb_resources, the subject, and the family's key text), its path is the field's own, and it is
-- prior (title / origin URI: a later event carries the field; values: no live asserted row; key
-- text: at or before the family's latest unset; facets: before the latest whole-facet fold). It is
-- evaluated over the ledger as it stood before this statement: the statement's own OLD key texts are
-- passed in, because a renamed key no longer names its family.
CREATE OR REPLACE FUNCTION kb_events_redaction_in_trail() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_subject uuid;
    v_record  uuid;
    v_payload jsonb;
    v_keys    jsonb;
BEGIN
    FOR v_subject IN
        SELECT DISTINCT (e.payload ->> 'subject_id')::uuid
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        IF EXISTS (
            WITH trail AS MATERIALIZED (
                SELECT ts.event_id FROM _resource_erasure_trail_scope(v_subject) ts
            )
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r ON r.event_id = n.id
              JOIN kb_events e ON e.id = r.redacted_by
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND e.created = now()
               AND (e.payload ->> 'subject_id')::uuid = v_subject
               AND NOT EXISTS (SELECT 1 FROM trail t WHERE t.event_id = n.id)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;

    FOR v_record IN
        SELECT DISTINCT r.redacted_by
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id AND r.authority = 'scrub'
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        IF v_keys IS NULL THEN
            SELECT coalesce(jsonb_object_agg(o.id::text, o.payload -> 'property_key'), '{}'::jsonb)
              INTO v_keys
              FROM old_rows o
             WHERE o.payload ? 'property_key';
        END IF;
        SELECT e.payload INTO v_payload FROM kb_events e WHERE e.id = v_record;
        IF EXISTS (
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority = 'scrub' AND r.redacted_by = v_record
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND NOT coalesce(_field_scrub_event_is_prior(
                       (v_payload ->> 'subject_id')::uuid,
                       v_payload #>> '{field,kind}',
                       (v_payload #>> '{field,family}')::uuid,
                       n.id, r.path, v_keys), false)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;
    RETURN NULL;
END;
$$;

SELECT declare_migration(
    20261018100010,
    'additive',
    'The ledger exception learns a second authority, resource_scrubbed (field-grain scrub spec S6). kb_event_field_redactions gains authority text NOT NULL DEFAULT ''erasure'' with a CHECK (erasure | scrub): the constant default stamps every existing row without an UPDATE, and an INSERT that names no authority, as every binary without this migration does through _project_resource_erased_redactions, writes an erasure row as before. Its primary key widens from (event_id, path) to (event_id, path, authority) under the same name kb_event_field_redactions_pkey, so resource_erasure_service''s REDACTIONS_KEY retry still matches, and a second erasure of one path still collides. Registers resource_scrubbed (admin) and re-registers the resource_erasure_refused payload_schema with optional enum values only (act field_scrub; reasons sentinel_collision, projection_disagrees): every payload valid before stays valid. DROP + CREATE of _erasure_sentinel_admits adds p_authority DEFAULT ''erasure'' (its only caller is kb_events_append_only, in SQL; no binary calls it), and under erasure it answers exactly as before. CREATE OR REPLACE, with unchanged signatures, of _project_resource_erased_redactions (now the erasure entry of one body, _project_field_redactions, with the same checks and messages, writing authority erasure), kb_events_append_only (a change is admitted when any row for its path authorises it under its own authority; an erasure is admitted exactly as before, a scrub only on a subject that is not erased) and kb_events_redaction_in_trail (the trail check unchanged; adds the scrub''s family-and-prior check, which no existing act triggers). New: _project_field_redactions, _project_resource_scrubbed_redactions, _field_scrub_property_events, _field_scrub_family_events and _field_scrub_event_is_prior. No grant changes.'
);
