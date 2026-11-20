-- The erasure record's six admin event types (erasure estate-membership spec D6).
--
--   erasure_declared        an erasure declared: kind, subject, request, manifest, the member map,
--                           the estate counts and checksum, the holder snapshot; `legacy` absent
--                           means false
--   erasure_member_arrived  an item mapped after the declaration, and what found it
--   erasure_scope_sealed    the closing survey sealed the scope: the late count and digest
--   erasure_reopened        an arrival after completion, naming the arrival event
--   erasure_batch_applied   the drain's non-authority summary of one batch over one member table
--   erasure_completed       the completion step ran: the redacted hashes and the late digest
--
-- Each is `admin`, NULL-anchored and typed. A kind is registered with the change that first
-- projects it (spec R20): their projectors arrive in 20261201000030, before any writer, so
-- registering them turns nothing on. _event_append (20260719000010) validates no payload, and
-- nothing appends these kinds yet. They precede the projectors' migration, which names them.
--
-- The payload schema literals below are the committed fixtures
-- crates/temper-substrate/tests/fixtures/payloads/{erasure_declared,erasure_member_arrived,
-- erasure_scope_sealed,erasure_reopened,erasure_batch_applied,erasure_completed}.v1.schema.json,
-- in that order, and `the_migration_literal_matches_the_committed_fixture` pins each pair.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('erasure_declared', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ErasureDeclared",
  "description": "`erasure_declared` — an erasure declared: its kind, subject, request and manifest, the map of\nevery item it covers, and the holders who could read them. Projected into `kb_erasures`,\n`kb_erasure_members`, `kb_erasure_former_holders` and, for an estate, `kb_erasure_estate_snapshots`.\n\nThe payload carries `erasure_id` because `_event_append` mints the event's own id and a payload\ncannot name it. A map entry carries no status: an entry with `prior_erasure` and `prior_event`\nis born `erased` (an earlier erasure already finished the item), and every other is born\n`pending`.\n\n`legacy` marks an erasure recorded from a door that names no erasure. A legacy declaration is\nalways of kind `resource` and carries no request, no manifest, no counts, no checksum and no\nholders. Absent means false.\n\nErasure records are never redacted, and no key is a trail join key: an event is named by\n`prior_event`, never `event_id`.",
  "type": "object",
  "properties": {
    "checksum": {
      "description": "The map's checksum (`erasure_map_checksum`): required for an estate, optional for any other\nnon-legacy erasure, and verified by the projector whenever present. Absent for a legacy\nerasure.",
      "type": [
        "string",
        "null"
      ]
    },
    "counts": {
      "description": "The declared member count per table, for an estate. Absent for a legacy erasure.",
      "type": [
        "object",
        "null"
      ],
      "properties": {
        "kb_blobs": {
          "type": "integer",
          "format": "uint64",
          "minimum": 0
        },
        "kb_cogmaps": {
          "type": "integer",
          "format": "uint64",
          "minimum": 0
        },
        "kb_contexts": {
          "type": "integer",
          "format": "uint64",
          "minimum": 0
        },
        "kb_edges": {
          "type": "integer",
          "format": "uint64",
          "minimum": 0
        },
        "kb_resources": {
          "type": "integer",
          "format": "uint64",
          "minimum": 0
        }
      },
      "additionalProperties": false
    },
    "erasure_id": {
      "type": "string",
      "format": "uuid"
    },
    "holders": {
      "description": "The profiles that held the erasure's items when it was declared. Empty for a legacy\nerasure.",
      "type": "array",
      "items": {
        "type": "string",
        "format": "uuid"
      }
    },
    "kind": {
      "$ref": "#/$defs/ErasureKind"
    },
    "legacy": {
      "description": "True for an erasure recorded from a door that names no erasure. Absent means false.",
      "type": "boolean"
    },
    "manifest_digest": {
      "description": "The sha256 (lowercase hex) of the canonical parsed erasure policy. Absent only when\n`legacy`.",
      "type": [
        "string",
        "null"
      ]
    },
    "manifest_version": {
      "description": "The release semver of the erasure policy the erasure was declared under. Absent only when\n`legacy`.",
      "type": [
        "string",
        "null"
      ]
    },
    "map": {
      "description": "Every item the erasure covers when declared, containers before the leaves they hold.",
      "type": "array",
      "items": {
        "$ref": "#/$defs/ErasureMapEntry"
      }
    },
    "request_ref": {
      "description": "The declaring request's idempotency key. Absent only when `legacy`.",
      "type": [
        "string",
        "null"
      ],
      "format": "uuid"
    },
    "subject_id": {
      "description": "The subject's id. Absent exactly when `subject_table` is.",
      "type": [
        "string",
        "null"
      ],
      "format": "uuid"
    },
    "subject_table": {
      "description": "The subject's table, as the DDL spells it: `kb_profiles` for an estate, `kb_resources` for\na resource or a block erasure. Absent for a selection.",
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "erasure_id",
    "kind",
    "map"
  ],
  "$defs": {
    "ErasureKind": {
      "description": "What an erasure was declared over, as `kb_erasures.kind` spells it (its\n`kb_erasures_kind_check`). Carried by `erasure_declared`.",
      "oneOf": [
        {
          "description": "A person's estate; the subject is a profile.",
          "type": "string",
          "const": "estate"
        },
        {
          "description": "One resource; the subject is the resource. Every legacy erasure is of this kind.",
          "type": "string",
          "const": "resource"
        },
        {
          "description": "A selection of items; there is no subject.",
          "type": "string",
          "const": "selection"
        },
        {
          "description": "A resource's block history; the subject is the resource.",
          "type": "string",
          "const": "block"
        }
      ]
    },
    "ErasureMapEntry": {
      "description": "One item of an erasure's map, as `erasure_declared` declares it.",
      "type": "object",
      "properties": {
        "id": {
          "type": "string",
          "format": "uuid"
        },
        "parent": {
          "description": "The container member (a context or a cogmap) of the same erasure that holds this item.",
          "anyOf": [
            {
              "$ref": "#/$defs/ErasureMemberRef"
            },
            {
              "type": "null"
            }
          ]
        },
        "prior_erasure": {
          "description": "The earlier erasure that already finished this item. Present with `prior_event`, and the\nrow is then born `erased`.",
          "type": [
            "string",
            "null"
          ],
          "format": "uuid"
        },
        "prior_event": {
          "description": "The event that erased this item in the earlier erasure.",
          "type": [
            "string",
            "null"
          ],
          "format": "uuid"
        },
        "table": {
          "$ref": "#/$defs/ErasureMemberTable"
        }
      },
      "required": [
        "table",
        "id"
      ]
    },
    "ErasureMemberRef": {
      "description": "An item an erasure member names: its table, as the DDL spells it, and its id.",
      "type": "object",
      "properties": {
        "id": {
          "type": "string",
          "format": "uuid"
        },
        "table": {
          "$ref": "#/$defs/ErasureMemberTable"
        }
      },
      "required": [
        "table",
        "id"
      ]
    },
    "ErasureMemberTable": {
      "description": "A table whose rows an erasure member can name. Serializes as the DDL spells the table.\nOrdered by declaration, so it can key a map that serializes with stable bytes.",
      "type": "string",
      "enum": [
        "kb_resources",
        "kb_contexts",
        "kb_cogmaps",
        "kb_blobs",
        "kb_edges"
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin'),
  ('erasure_member_arrived', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ErasureMemberArrived",
  "description": "`erasure_member_arrived` — an item mapped into an erasure after its declaration, by the arrival\ntrigger, the closing survey or the straggler detector. Projected as a `late` member row.",
  "type": "object",
  "properties": {
    "erasure_id": {
      "type": "string",
      "format": "uuid"
    },
    "member": {
      "$ref": "#/$defs/ErasureMemberRef"
    },
    "parent": {
      "description": "The container member of the same erasure that holds the item.",
      "anyOf": [
        {
          "$ref": "#/$defs/ErasureMemberRef"
        },
        {
          "type": "null"
        }
      ]
    },
    "source": {
      "$ref": "#/$defs/ErasureArrivalSource"
    }
  },
  "required": [
    "erasure_id",
    "member",
    "source"
  ],
  "$defs": {
    "ErasureArrivalSource": {
      "description": "What found the item an [`ErasureMemberArrived`] record maps.",
      "oneOf": [
        {
          "description": "The arrival trigger, as the item was written into the erasure's scope.",
          "type": "string",
          "const": "trigger"
        },
        {
          "description": "The closing survey, which seals the erasure's scope.",
          "type": "string",
          "const": "closing_survey"
        },
        {
          "description": "The straggler detector, after the scope was sealed.",
          "type": "string",
          "const": "straggler"
        }
      ]
    },
    "ErasureMemberRef": {
      "description": "An item an erasure member names: its table, as the DDL spells it, and its id.",
      "type": "object",
      "properties": {
        "id": {
          "type": "string",
          "format": "uuid"
        },
        "table": {
          "$ref": "#/$defs/ErasureMemberTable"
        }
      },
      "required": [
        "table",
        "id"
      ]
    },
    "ErasureMemberTable": {
      "description": "A table whose rows an erasure member can name. Serializes as the DDL spells the table.\nOrdered by declaration, so it can key a map that serializes with stable bytes.",
      "type": "string",
      "enum": [
        "kb_resources",
        "kb_contexts",
        "kb_cogmaps",
        "kb_blobs",
        "kb_edges"
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin'),
  ('erasure_scope_sealed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ErasureScopeSealed",
  "description": "`erasure_scope_sealed` — the closing survey sealed an erasure's scope. A container erasure\ncannot complete before it. Carries the count and the digest of the late members so far.",
  "type": "object",
  "properties": {
    "erasure_id": {
      "type": "string",
      "format": "uuid"
    },
    "late_count": {
      "description": "How many members arrived after the declaration.",
      "type": "integer",
      "format": "uint64",
      "minimum": 0
    },
    "late_digest": {
      "description": "The digest of the late members, taken over their `erasure_member_arrived` records.",
      "type": "string"
    }
  },
  "required": [
    "erasure_id",
    "late_count",
    "late_digest"
  ]
}
$JS$::jsonb, 1, 'admin'),
  ('erasure_reopened', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ErasureReopened",
  "description": "`erasure_reopened` — an item arrived after an erasure completed, so the completion no longer\nholds. Its projector clears `kb_erasures.completed_event_id`, and completion runs again.",
  "type": "object",
  "properties": {
    "arrival_event": {
      "description": "The `erasure_member_arrived` event that reopened the erasure.",
      "type": "string",
      "format": "uuid"
    },
    "erasure_id": {
      "type": "string",
      "format": "uuid"
    }
  },
  "required": [
    "erasure_id",
    "arrival_event"
  ]
}
$JS$::jsonb, 1, 'admin'),
  ('erasure_batch_applied', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ErasureBatchApplied",
  "description": "`erasure_batch_applied` — the drain's summary of one batch over one member table, and the\nreaper's record of a `batch_fatal` item. NOT an authority: each erased item's authority is its\nown event, named by `authority_event`. Projected as the refusals it records.",
  "type": "object",
  "properties": {
    "entries": {
      "type": "array",
      "items": {
        "$ref": "#/$defs/ErasureBatchEntry"
      }
    },
    "member_table": {
      "$ref": "#/$defs/ErasureMemberTable"
    }
  },
  "required": [
    "member_table",
    "entries"
  ],
  "$defs": {
    "ErasureBatchEntry": {
      "description": "One item's outcome in an [`ErasureBatchApplied`] record. The outcome is the item's, written to\nevery open row of it, in every erasure `erasures` names.",
      "type": "object",
      "properties": {
        "authority_event": {
          "description": "The item's authorising event, when it was erased.",
          "type": [
            "string",
            "null"
          ],
          "format": "uuid"
        },
        "erasures": {
          "description": "The erasures whose open rows of the item the batch locked.",
          "type": "array",
          "items": {
            "type": "string",
            "format": "uuid"
          }
        },
        "member": {
          "description": "The item's id, in the batch's member table.",
          "type": "string",
          "format": "uuid"
        },
        "outcome": {
          "$ref": "#/$defs/ErasureBatchOutcome"
        },
        "reason": {
          "description": "Why the item was set aside, when it was refused.",
          "anyOf": [
            {
              "$ref": "#/$defs/ErasureMemberRefusalReason"
            },
            {
              "type": "null"
            }
          ]
        },
        "routine": {
          "description": "The routine that raised the refusal: a function name, never a message.",
          "type": [
            "string",
            "null"
          ]
        },
        "sqlstate": {
          "description": "The SQLSTATE of the error that refused the item.",
          "type": [
            "string",
            "null"
          ]
        }
      },
      "required": [
        "member",
        "erasures",
        "outcome"
      ]
    },
    "ErasureBatchOutcome": {
      "description": "What a batch did with one item, in an [`ErasureBatchEntry`].",
      "oneOf": [
        {
          "description": "The item was erased, under its own authority event.",
          "type": "string",
          "const": "erased"
        },
        {
          "description": "The item was set aside with a reason.",
          "type": "string",
          "const": "refused"
        }
      ]
    },
    "ErasureMemberRefusalReason": {
      "description": "The drain's closed refusal vocabulary: why an erasure member was set aside rather than erased.\nMirrored by the `kb_erasure_refusal_reasons` table, `policy/erasure/estate.toml`'s\n`[refusal_reasons]` and the `erasure_refusal_reasons` seeds in `system.yaml`; a test holds the\nfour copies equal. Distinct from [`RecordedRefusalReason`], which is the single-resource doors'\nledger vocabulary.",
      "oneOf": [
        {
          "description": "The item is a cogmap's telos or charter resource, which only map-grain erasure erases.",
          "type": "string",
          "const": "charter_resource"
        },
        {
          "description": "A verifier raised on the item, so only a code or data fix lets it finish.",
          "type": "string",
          "const": "defect"
        },
        {
          "description": "The item failed its batch alone until the reaper set it aside.",
          "type": "string",
          "const": "batch_fatal"
        },
        {
          "description": "The item's unit raised an error that no other code names.",
          "type": "string",
          "const": "unclassified"
        }
      ]
    },
    "ErasureMemberTable": {
      "description": "A table whose rows an erasure member can name. Serializes as the DDL spells the table.\nOrdered by declaration, so it can key a map that serializes with stable bytes.",
      "type": "string",
      "enum": [
        "kb_resources",
        "kb_contexts",
        "kb_cogmaps",
        "kb_blobs",
        "kb_edges"
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin'),
  ('erasure_completed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ErasureCompleted",
  "description": "`erasure_completed` — the completion step ran for an erasure: every member was closed and the\nby-hash half applied. It may repeat, after an `erasure_reopened`; `kb_erasures` keeps the\nlatest. Carries the hashes the by-hash half redacted and the late-member digest.",
  "type": "object",
  "properties": {
    "erasure_id": {
      "type": "string",
      "format": "uuid"
    },
    "late_digest": {
      "description": "The digest of the late members, as `erasure_scope_sealed` takes it. Absent for an erasure\nthat was never sealed.",
      "type": [
        "string",
        "null"
      ]
    },
    "redacted_hashes": {
      "description": "The hashes the completion step redacted.",
      "type": "array",
      "items": {
        "type": "string"
      }
    }
  },
  "required": [
    "erasure_id"
  ]
}
$JS$::jsonb, 1, 'admin');

SELECT declare_migration(
    20261201000020,
    'additive',
    'The erasure record''s six admin event types: erasure_declared, erasure_member_arrived, '
    'erasure_scope_sealed, erasure_reopened, erasure_batch_applied and erasure_completed, each '
    'typed with its payload schema. Registry rows only: nothing appends these kinds yet, and no '
    'existing row, table or function changes, so a binary without this change is unaffected.'
);
