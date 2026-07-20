-- Register the two principal-admission event types (spec 2026-07-20 §10, D4).
--
-- CATEGORY IS SPELLED AT REGISTRATION, not stamped afterwards. 20260719000010 dropped the
-- `category` DEFAULT precisely so an omission fails 23502 at apply time naming the column, rather
-- than silently joining the trail allowlist. `registering_an_event_type_requires_an_explicit_category`
-- (crates/temper-services/tests/admin_ledger_test.rs:1002) pins that behaviour.
--
-- `admin` also buys two runtime guarantees for free: kb_events_admin_is_unanchored (admin implies a
-- NULL producing anchor, which the transition functions satisfy by passing NULL,NULL) and the
-- element-trail allowlist, which admits only `domain`. An admission act is an authority act; it has
-- no cognition home, and anchoring it would put it in front of every region producer and break the
-- "governance is traceable, but it isn't knowledge" boundary.
--
-- NOT ADDED TO system.yaml, deliberately. seed_migration_event_types_match_system_yaml
-- (bootseed.rs:117-124) requires every system.yaml name to also appear in canonical_seed.sql -- a
-- shipped, applied migration that must not be edited. `admin_ledger_opened` set the precedent.
--
-- THE JSON BELOW IS GENERATED, NOT AUTHORED. It is copied byte-for-byte from
-- crates/temper-substrate/tests/fixtures/payloads/*.v1.schema.json, emitted by
-- `UPDATE_SCHEMA=1 cargo make test-schema` (package-scoped -p temper-substrate; the workspace
-- invocation emits a different, unstamped shape). Editing it here desynchronizes repo, registry,
-- and Rust types.
--
-- Template: 20260719000020_slack_disconnect_event.sql.

INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('principal_standing_changed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "PrincipalStandingChanged",
  "description": "`principal_standing_changed` — one principal-admission transition (spec 2026-07-20 §10, D4).\n\nONE EVENT TYPE FOR ALL NINE ACTS, with the act in the payload. Nine types would mean nine\nschema snapshots, nine registrations and nine roundtrip arms for a distinction already carried\nin a field. The type boundary that IS worth drawing is standing-vs-governance, because that is\nthe boundary spec §2 separates: \"may you act\" and \"may you govern\" are two questions.\n\n`prior` is `None` exactly once per principal — the `provision` that created them.\n`actor` is `None` for the boot-seed genesis act and for backfilled rows: there is no actor to\nname, and inventing one would put a fabricated attribution on the ledger.",
  "type": "object",
  "properties": {
    "act": {
      "type": "string"
    },
    "actor": {
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "prior": {
      "type": [
        "string",
        "null"
      ]
    },
    "reason": {
      "type": [
        "string",
        "null"
      ]
    },
    "resulting": {
      "type": "string"
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
    "act",
    "resulting"
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
        "kb_machine_clients"
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
  ('principal_governance_changed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "PrincipalGovernanceChanged",
  "description": "`principal_governance_changed` — a principal gained or lost the authority to change the rules\n(spec 2026-07-20 D10).\n\nSeparate from `PrincipalStandingChanged` because governance is the separate question. A demote\nfired as a consequence of `Revoke`/`Deactivate` emits BOTH events in the same transaction, and\nthe pair is what makes the causal story legible in the ledger.",
  "type": "object",
  "properties": {
    "actor": {
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "change": {
      "description": "`granted` | `revoked`.",
      "type": "string"
    },
    "reason": {
      "type": [
        "string",
        "null"
      ]
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
    "change"
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
        "kb_machine_clients"
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}
$JS$::jsonb, 1, 'admin')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema,
      schema_version = EXCLUDED.schema_version,
      category       = EXCLUDED.category;
