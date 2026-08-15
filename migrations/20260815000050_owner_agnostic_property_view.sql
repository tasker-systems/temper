-- The whole-value property relation, owner-agnostic — one definition behind `kb_edge_properties`
-- (`20260815000010`) and `kb_resource_properties` (`20260815000040`), which were written a day
-- apart, the second by reading the first.
--
-- Task 01a00675-b111-79b2-8aac-e872f30acdd5. Design:
-- docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md §6.3.
--
-- ── What this does NOT do, and why it is the point ──────────────────────────────────────────────
--
-- The SQL PREDICATE BODY that reads these views exists twice — inside `__temper_ungated_follow_from`
-- and inside `__temper_ungated_find_resources_with` — and is **deliberately left as two copies**.
-- Extracting it into a shared `LANGUAGE sql STABLE` function is the obvious move and is the one move
-- that must not be made: `[measured — 20260808000020:308]` such a body containing a sublink does not
-- inline, so the `EXISTS` loses its Index Only Scan on `uq_kb_properties_active` and becomes a
-- per-row call. `[measured — 2026-08-15]` that index is the access path the live plan actually uses,
-- so the extraction would cost exactly what it was meant to tidy.
--
-- A VIEW is not a function and does not carry that cost — it is a rewrite rule and flattens.
-- `[measured — 2026-08-15, pg18]` the base-plus-wrapper form below EXPLAINs byte-identically to the
-- incumbent direct view, both `Index Only Scan using uq_kb_properties_active`. That is why the view
-- layer can be unified and the predicate layer cannot, and it is measured here rather than inferred
-- from the function result being about functions.
--
-- The two predicate bodies are instead held to a **differential witness**
-- (`tests/property_predicate_parity.rs`), which drives both over one predicate corpus and asserts
-- identical admit/deny. **Two bodies with a proof of agreement is a weaker thing than one body**,
-- and is recorded as such rather than described as unified.
--
-- ── Traps a later edit breaks silently ──────────────────────────────────────────────────────────
--   1. `CREATE OR REPLACE VIEW` can neither DROP nor RENAME a column `[measured — 2026-08-15, pg18]`
--      (`cannot drop columns from view` / `cannot change name of view column`). Both wrappers below
--      therefore keep their exact column list, in order, with the same names and types. This is also
--      what decides `weight` — see below.
--   2. The base is NOT the element relation. `kb_property_elements` (`20260815000030`) explodes
--      arrays; this one passes `property_value` WHOLE. An array-shaped probe matches under this
--      grain and matches NOTHING under that one — 1,228 array-shaped rows on prod. The grain is
--      RULED `[decided — 2026-08-15, Pete]`, not a preference.
--   3. `NOT is_folded` lives in the BASE. Moving it into a wrapper would leave the other one reading
--      folded rows, which is the exact class of divergence this migration exists to close.
--
-- ── `weight` stays, and the reason is a constraint rather than a preference ──────────────────────
-- Both views expose `weight` and no predicate reads it; `kb_resource_properties` inherited it by
-- being written from `kb_edge_properties`. Removing it would need DROP + CREATE — **shape-breaking**,
-- which `temper-migrate --additive-only` halts on for `main` (see DEPLOYING.md). So it is kept
-- deliberately: a view column is not materialized and costs nothing to carry, and the alternative is
-- an operator-run cutover to delete an unread name. Whoever wants it gone should take it with the
-- next shape-breaking migration that is happening anyway, not schedule one for this.
--
-- Additive: one CREATE VIEW, two CREATE OR REPLACE VIEW at identical column lists. No DROP.

-- ── The one definition ──────────────────────────────────────────────────────────────────────────
CREATE VIEW kb_owner_properties AS
    SELECT p.owner_table, p.owner_id, p.property_key, p.property_value, p.weight
      FROM kb_properties p
     WHERE NOT p.is_folded;

COMMENT ON VIEW kb_owner_properties IS
    'Live kb_properties at WHOLE-VALUE grain, for every owner table: one row per property, property_value exposed unchanged. The single definition behind kb_edge_properties and kb_resource_properties, which are owner-scoped wrappers over it and carry no predicate of their own beyond owner_table. Deliberately not kb_property_elements, which is the same rows at ELEMENT grain and serves the tags and facet predicates: an array-shaped containment probe matches here and matches nothing there, and an empty array is a row here and no rows there. NOT is_folded lives here rather than in a wrapper, so a scoped view cannot lose it.';

-- ── The two owner-scoped wrappers, now derived ──────────────────────────────────────────────────
-- Column lists are unchanged in name, type and order: CREATE OR REPLACE VIEW admits nothing else.

CREATE OR REPLACE VIEW kb_edge_properties AS
    SELECT op.owner_id AS edge_id, op.property_key, op.property_value, op.weight
      FROM kb_owner_properties op
     WHERE op.owner_table = 'kb_edges';

COMMENT ON VIEW kb_edge_properties IS
    'Live properties owned by an edge: kb_owner_properties scoped to owner_table = ''kb_edges''. A predicate reads this rather than the table, so it can neither lose a convention nor wrongly inherit one. Since 20260815000050 it derives from the owner-agnostic base rather than restating the shape, so it and kb_resource_properties cannot drift; the liveness rule (NOT is_folded) lives in the base. property_value is exposed unchanged; the element-exploding relation is a separate object. weight is exposed and read by nothing — kept because dropping a view column is shape-breaking, not because anything needs it.';

CREATE OR REPLACE VIEW kb_resource_properties AS
    SELECT op.owner_id AS resource_id, op.property_key, op.property_value, op.weight
      FROM kb_owner_properties op
     WHERE op.owner_table = 'kb_resources';

COMMENT ON VIEW kb_resource_properties IS
    'Live properties owned by a resource: kb_owner_properties scoped to owner_table = ''kb_resources'', with property_value exposed WHOLE. The relation BOTH members of the closed operator set read — contains against the whole value, has_key as a row-existence test. Since 20260815000050 it derives from the owner-agnostic base rather than restating the shape, so it and kb_edge_properties cannot drift; the liveness rule (NOT is_folded) lives in the base. Deliberately not kb_property_elements: an array-shaped containment probe matches here and matches nothing there, and an empty array is a row here and no rows there. kb_property_elements serves the tags and facet predicates, whose semantics are AND-containment over elements. weight is exposed and read by nothing — kept because dropping a view column is shape-breaking, not because anything needs it.';

SELECT declare_migration(
    20260815000050,
    'additive',
    'Gives kb_edge_properties (20260815000010) and kb_resource_properties (20260815000040) one definition: a new owner-agnostic kb_owner_properties -- live kb_properties at WHOLE-VALUE grain for every owner table -- with both incumbents rewritten as thin owner-scoped wrappers over it at byte-identical column lists. The two were written a day apart, the second by reading the first, and agreed only for that reason; the liveness rule NOT is_folded now has one home rather than two. Extraction is by VIEW and not by function for the reason 20260808000020 measured -- a LANGUAGE sql STABLE body containing a sublink does not inline and loses its Index Only Scan on uq_kb_properties_active -- and the view form is measured plan-identical here too: base-plus-wrapper EXPLAINs byte-identically to the incumbent direct view on pg18, both Index Only Scan using uq_kb_properties_active. The SQL PREDICATE BODY that reads these views is deliberately NOT extracted and remains two copies, held instead to a differential witness that drives both over one predicate corpus and asserts identical admit/deny; two bodies with a proof of agreement is a weaker thing than one body and is recorded as such. weight stays on both wrappers, unread: CREATE OR REPLACE VIEW cannot drop or rename a column, so removing it would be shape-breaking and halt the additive-only deploy, and a view column is not materialized. Additive: one CREATE VIEW, two CREATE OR REPLACE VIEW; no DROP. Task 01a00675-b111-79b2-8aac-e872f30acdd5.'
);
