-- Present-truth wording for three erasure payload schemas. Data only: descriptions move, no
-- property, type, required key or enum value does.
--
-- 1. `blob_erased` (registered by 20260909000015) said the erased-content set refuses
--    re-admission. That refusal was retired by 20260911000000; the record now names both erasing
--    acts that write it, person erasure and resource erasure.
-- 2. `principal_erased` (newest registration 20261023100000) described `redacted_hashes` in terms
--    that read as a ledger rewrite. It lists content hashes whose projection rows the act emptied
--    by hash; the ledger paths the act rewrote are in `redacted_fields`.
-- 3. `resource_scrubbed` (registered by 20261018100010) now says it is field-grain despite its
--    name: it never records a whole-resource scrub, and never a block.
--
-- Each payload_schema literal below is equal, as JSON, to its committed fixture
-- (`blob_erased.v1.schema.json`, `principal_erased.v1.schema.json`, then
-- `resource_scrubbed.v1.schema.json`), and
-- `payload_schema::the_migration_literal_matches_the_committed_fixture` pins all three, in this
-- order. A superseded literal is never edited; the next re-registration supersedes this one.

UPDATE kb_event_types
   SET payload_schema = $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "BlobErased",
  "description": "Erase one blob — the erasing acts' OWN strike vocabulary, fired through the SAME\n`blob_delete` wrapper and `_project_blob_deleted` projector as [`BlobDeleted`] (ruled\n2026-09-06: the emptying shape and the byte fate are shared, the vocabulary never is). Both\nerasing acts fire it: person erasure for the blobs homed in the subject's estate, resource\nerasure for each related blob its operator lists. Identity-only, the exact `BlobDeleted`\nshape: the envelope carries the home (producing anchor), the actor (`emitter_entity_id`),\nand the time (`occurred_at`).\n\nThe row empties into the IDENTICAL D5.2 shape — there is deliberately no row-shape marker\nof which act emptied it, and the ledger alone tells delete from erasure. This event's row\nwrite breaks no attribution: under person erasure the pseudonym break does that. The hash\nstays on the emptied row. Nothing refuses re-admission of the erased bytes: a later commit\nof identical bytes is an ordinary write that mints a fresh live row.",
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
$JS$::jsonb
 WHERE name = 'blob_erased';

UPDATE kb_event_types
   SET payload_schema = $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "PrincipalErased",
  "description": "`principal_erased` — the ONE admin event of a completed erasure (erasure spec D1).\n\nONE TYPE for identity erasure and content erasure: the distinction rides the per-target\noutcomes as data, never the type boundary — two types would cost the pairing (one request,\none operator, one reference) and double registration for no additional guarantee. The\nsubject is the PSEUDONYM the act itself broke; the event never re-identifies — no name, no\nemail, no case description. The request reference does not ride here: it lives on\n`kb_events.\"references\"`, the apparatus this act owns.\n\nThe redacted set is keyed on CONTENT HASH (D2) — never row ids, which would collide with\nthe trail functions' join-key shapes. The erased-content set (D4) rebuilds from these\npayloads alone; it is a projection the replay diffs, and no write path consults it.",
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
    "charters_held": {
      "description": "The estate's charters, emptied and held until map-grain erasure (D3).",
      "type": "array",
      "items": {
        "type": "string",
        "format": "uuid"
      }
    },
    "estate_contexts": {
      "description": "The estate the act reached (20261021100000, person-erasure design D1): the subject's @me\ncontexts and their personal team's contexts, in id order. The redaction reads it back from\nhere, so replay reaches the set live reached. Required, and written even when empty: the\nredaction raises without it, and no `principal_erased` predates it (the migration guards).",
      "type": "array",
      "items": {
        "type": "string",
        "format": "uuid"
      }
    },
    "redacted_fields": {
      "description": "The estate contexts' ledger copies of their names and slugs this act rewrote to the\nsentinels (20261022100000, R7): `context_renamed`, `context_retired` and\n`context_restored` paths, in the shape `resource_erased` uses. Paths only, never values.\nRequired, and written even when empty: no `principal_erased` predates it (the migration\nguards).",
      "type": "array",
      "items": {
        "$ref": "#/$defs/RedactedEventFields"
      }
    },
    "redacted_hashes": {
      "description": "The redacted set (D2): the content hashes whose projection rows this act emptied by\nhash. Bare sha256 hex, exactly as `content_hash` carries it — the key every reader\nalready shares and no trail join-key shape can match. \"Redacted\" here means emptied by\nhash, never a ledger rewrite: the ledger paths this act rewrote are in `redacted_fields`\nbelow, and those of each estate resource on its own `resource_erased` record.",
      "type": "array",
      "items": {
        "type": "string"
      }
    },
    "resource_erasures": {
      "description": "The resource erasures the act ran over the estate, in the order it ran them (D4).",
      "type": "array",
      "items": {
        "$ref": "#/$defs/EstateResourceErasureRef"
      }
    },
    "retained": {
      "description": "The categories the deployment's retention policy kept (20261023100000, R4), as the policy\nstood when the act ran. The redaction reads them back from here, as it reads the estate, so\nreplay keeps what live kept. Required, and written even when empty: no `principal_erased`\npredates it (the migration guards).",
      "type": "array",
      "items": {
        "$ref": "#/$defs/RetainedCategory"
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
    "subject_id",
    "estate_contexts",
    "resource_erasures",
    "charters_held",
    "redacted_fields",
    "retained"
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
    "EstateResourceErasureKind": {
      "description": "Whether that resource erasure was a first erasure or a completion pass (D2's `complete`).",
      "type": "string",
      "enum": [
        "erasure",
        "completion"
      ]
    },
    "EstateResourceErasureRef": {
      "description": "One resource erasure a person act ran: its subject and its `resource_erased` event. Keyed\n`resource` and `event`, never `resource_id`: the completion payload carries no key the trail\nfunctions join on.",
      "type": "object",
      "properties": {
        "event": {
          "type": "string",
          "format": "uuid"
        },
        "kind": {
          "$ref": "#/$defs/EstateResourceErasureKind"
        },
        "resource": {
          "type": "string",
          "format": "uuid"
        }
      },
      "required": [
        "resource",
        "event",
        "kind"
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
    "RetainedCategory": {
      "description": "One category a person erasure kept under the deployment's retention policy, as the act's record\ncarries it: the policy as it stood when the act ran, so the record stays true after the policy\nchanges and replay keeps exactly what the live act kept.",
      "type": "object",
      "properties": {
        "basis": {
          "description": "The stated basis, e.g. `GDPR 17(3)(e) / CCPA 105(d)(1)`.",
          "type": "string"
        },
        "category": {
          "$ref": "#/$defs/RetentionCategory"
        },
        "period": {
          "description": "The stated period, as Postgres renders the interval (`1 year`).",
          "type": "string"
        },
        "policy": {
          "description": "The policy's name.",
          "type": "string"
        }
      },
      "required": [
        "category",
        "policy",
        "basis",
        "period"
      ]
    },
    "RetentionCategory": {
      "description": "What a deployment's retention policy may keep through a person erasure (R4 of the 2026-10-10\nerasure inventory). The act removes every identifier of the subject and every free-text field\nabout them by default; a category here is kept only when an organisation names it in a policy\nwith a stated basis and period, and the record then names it `retained under policy <name>`.\n\nFree text and replay evidence only. Identifiers and live credentials are not retainable: a\ndeployment cannot keep a usable credential or a direct identifier of an erased person, so no\nvalue here names one. The SQL CHECK on `kb_erasure_retention_policies.category` is the same\nclosed set.",
      "oneOf": [
        {
          "description": "`kb_principal_standing_events.reason` and its ledger copy `principal_standing_changed.reason`.",
          "type": "string",
          "const": "standing-reasons"
        },
        {
          "description": "The ledger's `principal_governance_changed.reason`.",
          "type": "string",
          "const": "governance-reasons"
        },
        {
          "description": "`kb_join_requests.message` and `.decision_note` of the subject's requests.",
          "type": "string",
          "const": "join-request-notes"
        },
        {
          "description": "`kb_principal_review_requests.message` and `.decision_note` of the subject's requests.",
          "type": "string",
          "const": "review-request-notes"
        },
        {
          "description": "`kb_oauth_refresh_replays`: the evidence that one of the subject's refresh tokens was\ncopied. Kept with its tokens, which stay revoked with their claims and hash erased.",
          "type": "string",
          "const": "credential-replay-evidence"
        },
        {
          "description": "The ledger's `principal_erasure_refused.detail`: the operator's stated reason an earlier\nerasure of the subject was refused (the obligation held, the part that could not be\nhonoured).",
          "type": "string",
          "const": "erasure-refusal-details"
        }
      ]
    }
  }
}
$JS$::jsonb
 WHERE name = 'principal_erased';

UPDATE kb_event_types
   SET payload_schema = $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ResourceScrubbed",
  "description": "`resource_scrubbed` — the field scrub (field-grain scrub spec S1, S4): every prior value of a\nresource's title, origin URI or one property family (or every family's prior history) redacted\nfrom the ledger and the projection, on a resource that survives. The act is field-grain\ndespite the event's name: it never scrubs a whole resource, and never touches a block. With\n`cleared`, today's value was cleared too, by ordinary events appended before this one in the\nact's transaction.\n\nKeyed `subject_table` (`kb_resources`) / `subject_id`, never `resource_id`, and the family by\n`field.family`, never `event_id` (the D1 join-key rule). It never carries a top-level `owner`:\n`_resource_erasure_trail_scope` joins on `payload->'owner'->>'id'`. Paths only, never values or\nkey text: the record of a redaction must not carry what was redacted.",
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
$JS$::jsonb
 WHERE name = 'resource_scrubbed';

SELECT declare_migration(
    20261130000010,
    'additive',
    'Re-registers the blob_erased, principal_erased and resource_scrubbed payload schemas with '
    'present-truth descriptions. Every property, type, required key and enum value is unchanged, '
    'so every payload an older or newer binary writes validates the same. No table, column, '
    'function or index changes.'
);
