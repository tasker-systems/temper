-- One definition behind `kb_edge_properties` (`20260815000010`) and `kb_resource_properties`
-- (`20260815000040`): the whole-value property relation, owner-agnostic.
--
-- Reasoning, measurements and the differential witness that replaces the extraction this does NOT
-- do: task 01a00675-b111-79b2-8aac-e872f30acdd5, PR #684, and
-- docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md §6.3.
--
-- Traps a later edit breaks silently:
--   1. `CREATE OR REPLACE VIEW` can neither DROP nor RENAME a column, so both wrappers' column
--      lists are frozen in name, type and order. This is also why the unread `weight` stays:
--      removing it would be shape-breaking, not additive.
--   2. `NOT is_folded` lives in the BASE only. Restating it in a wrapper invites the sibling
--      wrapper to lose it.
--   3. Each wrapper's `owner_table` filter is the ONLY thing separating the two owners'
--      properties, and `kb_properties.owner_id` is not unique across owner tables.
--   4. Whole value, NOT `kb_property_elements`: an array-shaped containment probe matches here and
--      nothing there, and an empty array is a row here and no rows there.
--   5. A VIEW and not a predicate function, measured `[20260808000020:308]` — a `LANGUAGE sql
--      STABLE` body containing a sublink does not inline and loses its Index Only Scan.
--
-- Additive: one CREATE VIEW, two CREATE OR REPLACE VIEW at identical column lists. No DROP.

CREATE VIEW kb_owner_properties AS
    SELECT p.owner_table, p.owner_id, p.property_key, p.property_value, p.weight
      FROM kb_properties p
     WHERE NOT p.is_folded;

COMMENT ON VIEW kb_owner_properties IS
    'Live kb_properties at WHOLE-VALUE grain, for every owner table: one row per property, property_value exposed unchanged. The single definition behind kb_edge_properties and kb_resource_properties, which scope it by owner_table and rename owner_id. Not kb_property_elements, which is the same rows at ELEMENT grain and serves the tags and facet predicates. NOT is_folded lives here, so a scoped view cannot lose it.';

CREATE OR REPLACE VIEW kb_edge_properties AS
    SELECT op.owner_id AS edge_id, op.property_key, op.property_value, op.weight
      FROM kb_owner_properties op
     WHERE op.owner_table = 'kb_edges';

COMMENT ON VIEW kb_edge_properties IS
    'Live properties owned by an edge: kb_owner_properties scoped to owner_table = ''kb_edges''. A predicate reads this rather than the table, so it can neither lose a convention nor wrongly inherit one. weight is exposed and read by nothing.';

CREATE OR REPLACE VIEW kb_resource_properties AS
    SELECT op.owner_id AS resource_id, op.property_key, op.property_value, op.weight
      FROM kb_owner_properties op
     WHERE op.owner_table = 'kb_resources';

COMMENT ON VIEW kb_resource_properties IS
    'Live properties owned by a resource: kb_owner_properties scoped to owner_table = ''kb_resources''. The relation both members of the closed operator set read -- contains against the whole value, has_key as a row-existence test, which is why it is not the element view. weight is exposed and read by nothing.';

SELECT declare_migration(
    20260815000050,
    'additive',
    'Gives kb_edge_properties (20260815000010) and kb_resource_properties (20260815000040) one definition: a new owner-agnostic kb_owner_properties, with both incumbents rewritten as owner-scoped wrappers over it at byte-identical column lists. The two were written a day apart, the second by reading the first; NOT is_folded now has one home rather than two. Extraction is by VIEW and not by function for the reason 20260808000020 measured, and the view form is measured plan-identical here: base-plus-wrapper EXPLAINs byte-identically to the incumbent, both Index Only Scan using uq_kb_properties_active. The SQL predicate body that reads these views is deliberately NOT extracted and remains two copies, held instead to a differential witness. weight stays on both wrappers, unread: CREATE OR REPLACE VIEW cannot drop a column, so removing it would be shape-breaking. Additive: one CREATE VIEW, two CREATE OR REPLACE VIEW; no DROP. Task 01a00675-b111-79b2-8aac-e872f30acdd5.'
);
