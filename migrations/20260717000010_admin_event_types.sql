-- Admin-ledger event types (spec 2026-07-16 §9 step 3).
--
-- `grant_created`/`grant_revoked` were seeded 2026-06-24 (canonical_seed.sql:51-52) with NULL
-- payload_schema and zero events. They are this arc's types; here they finally get their schemas.
-- `admin_ledger_opened` is the epoch marker's type (its event is born in 20260717000020).
--
-- ALL THREE ARE NOW TYPED (payloads.rs: AdminLedgerOpened, GrantCreated, GrantRevoked +
-- TYPED_EVENT_NAMES). The schemas below are inlined VERBATIM from the committed
-- tests/fixtures/payloads/<name>.v1.schema.json, which are generated from those structs
-- (UPDATE_SCHEMA=1 cargo make test-schema) — the same repo==registry==Rust-types chain the
-- boot-seed's other 15 typed names ride (canonical_seed.sql:27). So `bootseed_publishes_payload_
-- schemas` (stamped set == TYPED_EVENT_NAMES) stays green, and `verify_ledger_roundtrip` validates
-- every admin payload against its struct — including the SQL-built grant payloads Task 5 writes.
--
-- The grant payloads spell the subject `subject_table`/`subject_id`, NEVER `resource_id`/`owner`:
-- element_trail_node/_edge match on payload key shape with no type filter, so those keys would leak
-- the grant into any reader's element trail (spec §5, tested: admin_ledger_test.rs Task 3).
--
-- NOTE the two stale registry rows (region_materialized/lens_created, task 019f6b48) are NOT touched
-- here. NULLing a TYPED_EVENT_NAME would break bootseed_publishes_payload_schemas (stamped set must
-- equal the typed set); the invariant-consistent fix for their staleness is a re-stamp from the
-- current fixtures, which is its own registration task (re-filed).

INSERT INTO kb_event_types (name, payload_schema, schema_version) VALUES
  ('admin_ledger_opened', $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "AdminLedgerOpened",
  "description": "`admin_ledger_opened` — the ledger's epoch marker. The time is the event's own `occurred_at`\n(`ledger_epoch` reads it there); the payload carries only the human note that says why nothing\nprecedes it. No `opened_at` field — a timestamp in a payload duplicates `occurred_at` and this\nmodule's rule is that derived/carried state (timestamps included) never lives in the payload.",
  "type": "object",
  "properties": {
    "note": {
      "type": "string"
    }
  },
  "required": [
    "note"
  ]
}$JS$::jsonb, 1),
  ('grant_created', $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "GrantCreated",
  "description": "`grant_created` — a capability grant recorded on the admin ledger. Carries `previous` only when\nthe upsert replaced an existing grant (an upsert that changes capabilities returns inserted=false,\nso the before/after is what makes a real authority change legible).",
  "type": "object",
  "properties": {
    "can_delete": {
      "type": "boolean"
    },
    "can_grant": {
      "type": "boolean"
    },
    "can_read": {
      "type": "boolean"
    },
    "can_write": {
      "type": "boolean"
    },
    "granted_by": {
      "$ref": "#/$defs/ProfileId"
    },
    "previous": {
      "anyOf": [
        {
          "$ref": "#/$defs/GrantCapabilities"
        },
        {
          "type": "null"
        }
      ]
    },
    "principal_id": {
      "type": "string",
      "format": "uuid"
    },
    "principal_table": {
      "$ref": "#/$defs/AnchorTable"
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
    "principal_table",
    "principal_id",
    "can_read",
    "can_write",
    "can_delete",
    "can_grant",
    "granted_by"
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
    "GrantCapabilities": {
      "description": "The four capability bits of a grant. Reused for `GrantCreated::previous`.",
      "type": "object",
      "properties": {
        "can_delete": {
          "type": "boolean"
        },
        "can_grant": {
          "type": "boolean"
        },
        "can_read": {
          "type": "boolean"
        },
        "can_write": {
          "type": "boolean"
        }
      },
      "required": [
        "can_read",
        "can_write",
        "can_delete",
        "can_grant"
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}$JS$::jsonb, 1),
  ('grant_revoked', $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "GrantRevoked",
  "description": "`grant_revoked` — a capability revocation recorded on the admin ledger. The grant row is DELETEd\n(it is the current-state projection); this event is the temporal record.",
  "type": "object",
  "properties": {
    "principal_id": {
      "type": "string",
      "format": "uuid"
    },
    "principal_table": {
      "$ref": "#/$defs/AnchorTable"
    },
    "revoked_by": {
      "$ref": "#/$defs/ProfileId"
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
    "principal_table",
    "principal_id",
    "revoked_by"
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
}$JS$::jsonb, 1)
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema, schema_version = EXCLUDED.schema_version;
