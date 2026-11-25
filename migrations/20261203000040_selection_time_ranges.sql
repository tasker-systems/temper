-- `ResourceFilter.created` and `.updated`: the selection narrows by when a resource was created or
-- last updated. Four new functions, each the existing function of its name called unchanged and
-- narrowed by two `tstzrange` arguments: the selection core (20261104000000), the list core
-- (20261128000010), and the gated wrapper of each.
--
-- What a later edit must not break:
--   1. THE CORES ARE CALLED, NEVER COPIED. Each new function selects from the function it widens
--      and adds only the two range conjuncts, so the selection's predicates keep one site
--      (20260815000040, 20261128000010 point 1).
--   2. `p_created` READS `kb_resources.created` AND `p_updated` READS `kb_resources.updated`, each
--      as `column <@ range`. The compiler builds every range `[)`: `from` inclusive, `until`
--      exclusive, an absent end unbounded. NULL narrows nothing.
--   3. THE RANGES SIT BENEATH THE PAGE. They narrow the list core's unpaged match set, which the
--      compiler orders, pages and counts for the stage's `total` (20261128000010 points 2 and 3).
--   4. The two ranges follow `p_properties` and precede the list core's `p_bound_ids`. Both are
--      `tstzrange`, so a transposition of the two would typecheck; the compiler's test holds the
--      order.
--   5. NO DEFAULT on any parameter (20260815000040, point 1).
--   6. The ungated forms apply no gate of their own: p_visible_ids is the verdict, and NULL admits
--      nothing. The gated wrappers are the only forms that compute it.
--
-- Additive: four CREATE FUNCTION. No REPLACE, no DROP.

CREATE FUNCTION __temper_ungated_find_resources_with(
    p_visible_ids    uuid[],
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_artifact_asks  jsonb,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_anchor_reader  uuid,
    p_properties     jsonb,
    p_created        tstzrange,
    p_updated        tstzrange)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT s.resource_id
      FROM __temper_ungated_find_resources_with(
             p_visible_ids, p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains, p_artifact_asks,
             p_anchor_table, p_anchor_id, p_anchor_reader, p_properties) s
      JOIN kb_resources r ON r.id = s.resource_id
     WHERE (p_created IS NULL OR r.created <@ p_created)
       AND (p_updated IS NULL OR r.updated <@ p_updated);
$$;

CREATE FUNCTION query_find_resources_with(
    p_principal      uuid,
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_artifact_asks  jsonb,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_properties     jsonb,
    p_created        tstzrange,
    p_updated        tstzrange)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id
      FROM __temper_ungated_find_resources_with(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains, p_artifact_asks,
             p_anchor_table, p_anchor_id, p_principal, p_properties, p_created, p_updated) c;
$$;

CREATE FUNCTION __temper_ungated_list_resources_with(
    p_visible_ids    uuid[],
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_artifact_asks  jsonb,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_anchor_reader  uuid,
    p_properties     jsonb,
    p_created        tstzrange,
    p_updated        tstzrange,
    p_bound_ids      uuid[])
RETURNS TABLE (resource_id uuid, created_epoch double precision)
LANGUAGE sql STABLE AS $$
    SELECT l.resource_id, l.created_epoch
      FROM __temper_ungated_list_resources_with(
             p_visible_ids, p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains, p_artifact_asks,
             p_anchor_table, p_anchor_id, p_anchor_reader, p_properties, p_bound_ids) l
      JOIN kb_resources r ON r.id = l.resource_id
     WHERE (p_created IS NULL OR r.created <@ p_created)
       AND (p_updated IS NULL OR r.updated <@ p_updated);
$$;

CREATE FUNCTION query_list_resources_with(
    p_principal      uuid,
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_artifact_asks  jsonb,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_properties     jsonb,
    p_created        tstzrange,
    p_updated        tstzrange,
    p_bound_ids      uuid[])
RETURNS TABLE (resource_id uuid, created_epoch double precision)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.created_epoch
      FROM __temper_ungated_list_resources_with(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains, p_artifact_asks,
             p_anchor_table, p_anchor_id, p_principal, p_properties, p_created, p_updated,
             p_bound_ids) c;
$$;

COMMENT ON FUNCTION __temper_ungated_find_resources_with(
        uuid[], text[], text[], jsonb, text, text, uuid, text, text, jsonb, varchar, uuid, uuid,
        jsonb, tstzrange, tstzrange) IS
    'UNGATED. The 14-argument selection core, called unchanged, narrowed by p_created (kb_resources.created <@ p_created) and p_updated (kb_resources.updated <@ p_updated). NULL narrows nothing; the query compiler builds each range [) from ResourceFilter.created / .updated, an absent end unbounded. p_visible_ids is the verdict (NULL admits none); no DEFAULT on any parameter (20261203000040).';

COMMENT ON FUNCTION query_find_resources_with(
        uuid, text[], text[], jsonb, text, text, uuid, text, text, jsonb, varchar, uuid, jsonb,
        tstzrange, tstzrange) IS
    'Gated wrapper over the 16-argument __temper_ungated_find_resources_with: computes resources_visible_to(p_principal) once and passes the principal down as the anchor reader. No defaults (20261203000040).';

COMMENT ON FUNCTION __temper_ungated_list_resources_with(
        uuid[], text[], text[], jsonb, text, text, uuid, text, text, jsonb, varchar, uuid, uuid,
        jsonb, tstzrange, tstzrange, uuid[]) IS
    'UNGATED. The 15-argument list core, called unchanged, narrowed by p_created (kb_resources.created <@ p_created) and p_updated (kb_resources.updated <@ p_updated); NULL narrows nothing. Unpaged and unordered, so the query compiler''s page and the stage''s total are over the narrowed set. p_visible_ids is the verdict (NULL admits none); no DEFAULT on any parameter (20261203000040).';

COMMENT ON FUNCTION query_list_resources_with(
        uuid, text[], text[], jsonb, text, text, uuid, text, text, jsonb, varchar, uuid, jsonb,
        tstzrange, tstzrange, uuid[]) IS
    'Gated wrapper over the 17-argument __temper_ungated_list_resources_with: computes resources_visible_to(p_principal) once and passes the principal down as the anchor reader. Unpaged and unordered. No defaults (20261203000040).';

SELECT declare_migration(
    20261203000040,
    'additive',
    'New overloads __temper_ungated_find_resources_with (16 arguments), query_find_resources_with (15), __temper_ungated_list_resources_with (17) and query_list_resources_with (16): each the existing function of its name called unchanged and narrowed by two tstzrange arguments, p_created and p_updated, with no DEFAULT on any parameter. No existing function, table or default changes; no REPLACE and no DROP.'
);
