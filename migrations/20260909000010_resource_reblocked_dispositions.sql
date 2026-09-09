-- The per-folded-id disposition map on `resource_reblocked` (the defined-dangling-state
-- design, D-D2): `ResourceReblocked` gains `dispositions` — for each folded incumbent, the
-- surviving blocks holding its content (absorbers = full chunk-hash multiset, kept AND
-- created; carried = strict subsets) or the content-gone arm. Additive payload-schema change,
-- `serde(default)`: absent key = no mapping recorded, the defined `unrecorded` disposition on
-- every reading surface — never a guess. Both Rust arms (shipped op, whole-body replace)
-- populate it at computation time; both fold faces write through the same projector, which
-- does not read the map — no projector change rides this stamp. Registry re-stamp: the
-- regenerated fixture, byte for byte (repo == registry == Rust types). `_event_append` does
-- NOT validate payloads against this column (20260624000002), so the re-stamp rides migration
-- application, not a deploy gate. Additive posture — `schema_version` stays 1.
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
    "dispositions": {
      "description": "Where each folded incumbent's content went — the per-folded-id disposition map (the\ndefined-dangling-state design, D-D2). `folded` records WHICH blocks the partition\nsupersedes; this map records what a dangling read may name as successors, captured HERE\nbecause the manifest alone cannot resolve them (a folded block absorbed into a kept\nblock can leave no trace in the payload). Both arms populate it; a folded id absent\nfrom it (pre-map events, `charter_set`, historical `block_mutated` replaces-body folds)\nis the defined `unrecorded` disposition on every reading surface — never a guess.",
      "type": "object",
      "additionalProperties": {
        "$ref": "#/$defs/FoldDisposition"
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
    "FoldDisposition": {
      "description": "One folded incumbent's content disposition (D-D2), recorded at computation time while the\nfold logic still knows the mapping. Resolved read-path only: a dangling read walks the\nfolded row's `last_event_id` to the fold event and reads this map — no successor pointer\nis born on block rows, and the ledger stays the authority.",
      "oneOf": [
        {
          "description": "Content locatable in surviving blocks: every section holding the incumbent's FULL\nchunk-hash multiset names its block under `absorbers` (kept AND created — on the\nwhole-body path a rewritten-away incumbent is absorbed into a freshly minted section,\nso created absorbers are the common case); every further section holding a strict\nsubset names its block under `carried`. Absorbers can legitimately be empty: content\nspread across section boundaries may live everywhere in part and nowhere in whole.",
          "type": "object",
          "properties": {
            "absorbers": {
              "description": "Surviving blocks whose section contains the incumbent's full chunk-hash multiset.",
              "type": "array",
              "items": {
                "$ref": "#/$defs/BlockId"
              }
            },
            "carried": {
              "description": "Surviving blocks holding only part of the incumbent's content.",
              "type": "array",
              "items": {
                "$ref": "#/$defs/BlockId"
              }
            },
            "state": {
              "type": "string",
              "const": "located"
            }
          },
          "required": [
            "state"
          ]
        },
        {
          "description": "Nothing locatable: the incumbent had no current chunks, or its chunk hashes appear in\nno section of the new partition. A named arm, never an inference — its provenance rows\nstay history on the folded row, and no successor is named.",
          "type": "object",
          "properties": {
            "state": {
              "type": "string",
              "const": "content_gone"
            }
          },
          "required": [
            "state"
          ]
        }
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
}
$JS$::jsonb
WHERE name = 'resource_reblocked';
