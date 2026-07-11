-- T4 — the w_cos kernel: teach the lens projection about the kernel columns, then seed the context
-- regime's lens through the ledger.
--
-- Spec §3.1, §3.2. Companion to the Rust kernel (crates/temper-substrate/src/knn.rs).

-- ─────────────────────────────────────────────────────────────────────────────
-- 1. `_project_lens_created` never learned about the T2 columns.
--
-- T2 (20260712000030) ADDED w_cos / knn_k / cos_floor to kb_cogmap_lenses with defaults
-- (0.0 / 12 / 0.55), but the projection function still inserts only the original 13 columns. Every
-- new lens therefore silently takes w_cos = 0.0 — the declared-only regime.
--
-- That is fine for every lens that exists today, and FATAL for the one below: seeding
-- `workflow-default` through `lens_create` without this change would land a lens whose regime switch
-- is OFF. The kernel would be a no-op, contexts would cluster into all-singletons, and nothing would
-- error — the failure would present as "w_cos doesn't work" rather than "the lens didn't load".
--
-- COALESCE, not a bare read: the payload keys are optional (kb_events is append-only, so the two
-- pre-kernel `lens_created` events are immortal and carry no `weights.cos`). Defaults here mirror
-- both the column defaults and the serde defaults on `payloads::LensCreated` — one value, declared in
-- three places that must agree.
CREATE OR REPLACE FUNCTION _project_lens_created(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_lens uuid := (p_payload->>'lens_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_cogmap_lenses
        (id, cogmap_id, name, selection_kind,
         w_express, w_contains, w_leads_to, w_near, w_prop,
         w_cos, knn_k, cos_floor,
         s_telos, s_ref, s_central, resolution, asserted_by_event_id, created)
    VALUES (v_lens,
            (p_payload->>'cogmap_id')::uuid,             -- NULL for a global lens
            p_payload->>'name', p_payload->>'selection_kind',
            (p_payload#>>'{weights,express}')::double precision,
            (p_payload#>>'{weights,contains}')::double precision,
            (p_payload#>>'{weights,leads_to}')::double precision,
            (p_payload#>>'{weights,near}')::double precision,
            (p_payload#>>'{weights,prop}')::double precision,
            COALESCE((p_payload#>>'{weights,cos}')::double precision, 0.0),
            COALESCE((p_payload->>'knn_k')::int, 12),
            COALESCE((p_payload->>'cos_floor')::double precision, 0.55),
            (p_payload#>>'{salience,telos}')::double precision,
            (p_payload#>>'{salience,ref}')::double precision,
            (p_payload#>>'{salience,central}')::double precision,
            (p_payload->>'resolution')::double precision,
            p_event, v_occurred);
    RETURN v_lens;
END;
$$;

-- ─────────────────────────────────────────────────────────────────────────────
-- 2. Seed the `workflow-default` lens — the context regime (spec §3.2).
--
-- Global (cogmap_id NULL ⇒ home_anchor_table NULL), so every context picks it up with no per-context
-- authoring. Event-sourced via lens_create, exactly as the two system lenses are seeded
-- (20260624000003_canonical_seed.sql:68) — a lens is a declared artifact, and short-circuiting the
-- ledger with a raw INSERT would be precisely the quiet inconsistency the event model exists to
-- prevent. The payload is NESTED (weights{} / salience{}), not positional.
--
-- Everything DELIBERATE sits at cogmap parity — w_express, w_contains and w_prop are NOT zeroed,
-- despite contexts carrying zero facets today. A weight is a rate of exchange (what a signal is worth
-- WHEN PRESENT), not a prior on how often it appears; zeroing them would make the discipline provably
-- unrewarded, and an information system that returns no signal for signal provided gets routed around.
-- w_leads_to is LIFTED to 0.9: `advances` is cheap to create but it is the hub topology (§3.3).
--
-- Idempotent: a re-run must not append a second lens_created event or a duplicate row.
DO $$
DECLARE v_system uuid;
BEGIN
    IF EXISTS (SELECT 1 FROM kb_cogmap_lenses WHERE name = 'workflow-default') THEN
        RETURN;
    END IF;

    -- The same system-entity lookup the two system lenses use (canonical_seed.sql:79).
    SELECT e.id INTO v_system
      FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id
     WHERE p.handle = 'system' AND e.name = 'system';
    IF v_system IS NULL THEN
        RAISE EXCEPTION 'workflow-default lens: no system entity to attribute the lens_created event to';
    END IF;

    PERFORM lens_create(
        jsonb_build_object(
            'lens_id',        uuid_generate_v7(),
            'cogmap_id',      NULL,
            'name',           'workflow-default',
            'selection_kind', 'homed',
            'weights', jsonb_build_object(
                'express',  1.0,    -- parity — deliberate, rare, high-information
                'contains', 1.0,    -- parity
                'leads_to', 0.9,    -- `advances` — the hub topology (§3.3)
                'near',     0.35,   -- `relates_to` — cheapest, most abundant. Real but weak.
                'prop',     0.4,    -- parity
                'cos',      1.0     -- THE REGIME SWITCH: inferred similarity is PRIMARY here
            ),
            'knn_k',     12,
            'cos_floor', 0.55,
            'salience', jsonb_build_object(
                'telos',   0.6,
                'ref',     0.15,    -- contexts have shallower provenance depth than distilled nodes
                'central', 0.25
            ),
            'resolution', 0.5
        ),
        v_system
    );
END;
$$;

-- ─────────────────────────────────────────────────────────────────────────────
-- 3. Republish the `lens_created` contract (payload spec §6: repo == registry == Rust types).
--
-- The schema is GENERATED from `payloads::LensCreated` and committed at
-- tests/fixtures/payloads/lens_created.v1.schema.json; `bootseed` stamps that file into
-- kb_event_types for fresh databases. This UPDATE is what keeps an ALREADY-SEEDED database (prod)
-- honest — the same move 20260712000040 made for `region_materialized`.
--
-- Additive: `weights.cos`, `knn_k` and `cos_floor` are OPTIONAL with defaults, so every pre-kernel
-- `lens_created` event stays valid and readable. (_event_append does not validate against this schema
-- — it is a published contract, not a gate — so this is a documentation fix, not an enforcement one.)
UPDATE kb_event_types SET payload_schema = $schema${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "LensCreated",
  "type": "object",
  "properties": {
    "lens_id": {
      "$ref": "#/$defs/LensId"
    },
    "cogmap_id": {
      "anyOf": [
        {
          "$ref": "#/$defs/CogmapId"
        },
        {
          "type": "null"
        }
      ]
    },
    "name": {
      "type": "string"
    },
    "selection_kind": {
      "type": "string"
    },
    "weights": {
      "$ref": "#/$defs/LensWeights"
    },
    "salience": {
      "$ref": "#/$defs/SalienceWeights"
    },
    "resolution": {
      "type": "number",
      "format": "double"
    },
    "knn_k": {
      "type": "integer",
      "format": "uint32",
      "minimum": 0,
      "default": 12
    },
    "cos_floor": {
      "type": "number",
      "format": "double",
      "default": 0.55
    }
  },
  "required": [
    "lens_id",
    "name",
    "selection_kind",
    "weights",
    "salience",
    "resolution"
  ],
  "$defs": {
    "LensId": {
      "description": "A `kb_cogmap_lenses.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "CogmapId": {
      "description": "A `kb_cogmaps.id` value — a cognitive map.",
      "type": "string",
      "format": "uuid"
    },
    "LensWeights": {
      "type": "object",
      "properties": {
        "express": {
          "type": "number",
          "format": "double"
        },
        "contains": {
          "type": "number",
          "format": "double"
        },
        "leads_to": {
          "type": "number",
          "format": "double"
        },
        "near": {
          "type": "number",
          "format": "double"
        },
        "prop": {
          "type": "number",
          "format": "double"
        },
        "cos": {
          "description": "The sparse exact-kNN cosine weight — the regime switch (spec §3.1). **Defaulted, not required:**\n`kb_events` is append-only, so the `lens_created` events for the pre-kernel lenses are immortal\nand carry no `cos` key. A required field would break `replay`'s round-trip through this struct\non every one of them. The default (0.0) is exactly what those lenses ARE — declared-only.",
          "type": "number",
          "format": "double",
          "default": 0.0
        }
      },
      "required": [
        "express",
        "contains",
        "leads_to",
        "near",
        "prop"
      ]
    },
    "SalienceWeights": {
      "type": "object",
      "properties": {
        "telos": {
          "type": "number",
          "format": "double"
        },
        "ref": {
          "type": "number",
          "format": "double"
        },
        "central": {
          "type": "number",
          "format": "double"
        }
      },
      "required": [
        "telos",
        "ref",
        "central"
      ]
    }
  }
}$schema$::jsonb
 WHERE name = 'lens_created';
