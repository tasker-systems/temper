-- `property_unset`'s projection becomes one SQL body, `_project_property_unset`.
--
-- Until now the projection ran in Rust (`events::project_property_unset`), and the event's
-- registration header (20260921000010) says so: "no `_project_*` function, fire and replay share
-- one body". This migration supersedes that claim. The field-grain scrub (spec 2026-10-09, S4
-- step 3) clears a property family by appending a `property_unset` and projecting it inside its
-- own SQL function, and it must reach the same body the fire path and replay reach, not a second
-- copy of it. The Rust function is now one call of this function, so fire and replay still share
-- one body; the body is here.
--
-- The three statements are the Rust function's, in its order, ported unchanged:
--   1. the write guard, with the owner table spelled 'kb_resources' whatever the payload's
--      `owner.table` says (spec 2026-09-28 D13: the owning resource is locked FOR KEY SHARE and an
--      erased one refuses the unset);
--   2. the fold of every live `(kb_resources, owner, property_key)` row, stamping the event;
--   3. the search-vector rebuild, only when the payload's `owner.table` is 'kb_resources' and the
--      key is `keywords`, `descriptor` or `tags`, after the fold so it reads the post-fold live
--      set. A payload with no owner id there raises, as the Rust function's
--      "property_unset payload carries no owner id" did.
-- It returns the fold's row count, which the Rust function returned as its `rows_affected()`.
-- Idempotent under replay: a second application folds zero rows and the rebuild is a refresh.
CREATE FUNCTION _project_property_unset(p_event uuid, p_payload jsonb)
RETURNS bigint
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_folded bigint;
    v_owner  uuid;
BEGIN
    PERFORM _resource_write_guard_owner('kb_resources', (p_payload->'owner'->>'id')::uuid);

    UPDATE kb_properties
       SET is_folded = true, last_event_id = p_event
     WHERE owner_table = 'kb_resources'
       AND owner_id = (p_payload->'owner'->>'id')::uuid
       AND property_key = (p_payload->>'property_key')
       AND NOT is_folded;
    GET DIAGNOSTICS v_folded = ROW_COUNT;

    IF p_payload->'owner'->>'table' = 'kb_resources'
       AND p_payload->>'property_key' IN ('keywords', 'descriptor', 'tags') THEN
        v_owner := (p_payload->'owner'->>'id')::uuid;
        IF v_owner IS NULL THEN
            RAISE EXCEPTION 'property_unset payload carries no owner id';
        END IF;
        PERFORM _rebuild_resource_search_vector(v_owner);
    END IF;

    RETURN v_folded;
END;
$$;

SELECT declare_migration(
    20261018100000,
    'additive',
    'New: the function _project_property_unset(uuid, jsonb) RETURNS bigint, the property_unset projection the Rust projector ran as three statements (guard, fold, search-vector rebuild), now one body the Rust projector calls. No table, column, constraint, grant or event type changes. A binary without this migration never calls the function and keeps running the same three statements itself.'
);
