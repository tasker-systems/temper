-- `list-resources-with`: the rows a `find-resources-with` selection selects, as a returning act.
-- Two new functions over the 14-argument selection core (20261104000000), which is called unchanged.
--
-- What a later edit must not break:
--   1. THE CORE IS CALLED, NEVER COPIED. The list's match set is the selection's set, narrowed by
--      p_bound_ids. A second copy of the predicates would be a second predicate site, which
--      20260815000040 exists to prevent.
--   2. UNPAGED AND UNORDERED. The compiler orders and pages this set, and counts it for the stage's
--      `total`, in the same statement. A LIMIT or ORDER BY here would leave nothing to count.
--   3. THE BOUND SITS BENEATH THE PAGE. p_bound_ids narrows the match set before the compiler's
--      LIMIT, as find-exact's bound does (20260808000030). NULL is unbounded; '{}' admits nothing.
--   4. created_epoch IS A COLUMN OF THE ROW ALONE: `kb_resources.created` as epoch seconds. No
--      window, no aggregate, so a different visible set cannot move any row's value — the act's
--      `PrincipalAgnostic` profile. `created` is written once by the projector and never updated.
--   5. NO DEFAULT on any parameter, as on the core (20260815000040, point 1).
--   6. The ungated core applies no gate of its own: p_visible_ids is the verdict, and NULL admits
--      nothing. The gated wrapper is the only form that computes it.
--
-- Additive: two CREATE FUNCTION. No REPLACE, no DROP.

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
    p_bound_ids      uuid[])
RETURNS TABLE (resource_id uuid, created_epoch double precision)
LANGUAGE sql STABLE AS $$
    SELECT r.id, extract(epoch FROM r.created)::double precision
      FROM __temper_ungated_find_resources_with(
             p_visible_ids, p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains, p_artifact_asks,
             p_anchor_table, p_anchor_id, p_anchor_reader, p_properties) s
      JOIN kb_resources r ON r.id = s.resource_id
     WHERE p_bound_ids IS NULL OR r.id = ANY(p_bound_ids);
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
    p_bound_ids      uuid[])
RETURNS TABLE (resource_id uuid, created_epoch double precision)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.created_epoch
      FROM __temper_ungated_list_resources_with(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains, p_artifact_asks,
             p_anchor_table, p_anchor_id, p_principal, p_properties, p_bound_ids) c;
$$;

COMMENT ON FUNCTION __temper_ungated_list_resources_with(
        uuid[], text[], text[], jsonb, text, text, uuid, text, text, jsonb, varchar, uuid, uuid,
        jsonb, uuid[]) IS
    'UNGATED. The match set of a list-resources-with stage: the 14-argument __temper_ungated_find_resources_with selection, unchanged, narrowed by p_bound_ids (NULL is unbounded, ''{}'' admits nothing), with each row''s created as epoch seconds (created_epoch, the act''s ordering column). Unpaged and unordered: the query compiler orders it by created_epoch and id, pages it, and counts it for the stage''s total. p_visible_ids is the verdict (NULL admits none); no DEFAULT on any parameter (20261128000010).';

COMMENT ON FUNCTION query_list_resources_with(
        uuid, text[], text[], jsonb, text, text, uuid, text, text, jsonb, varchar, uuid, jsonb,
        uuid[]) IS
    'Gated wrapper over __temper_ungated_list_resources_with: computes resources_visible_to(p_principal) once and passes the principal down as the anchor reader, as query_find_resources_with does. Unpaged and unordered. No defaults (20261128000010).';

SELECT declare_migration(
    20261128000010,
    'additive',
    'New functions __temper_ungated_list_resources_with (15 arguments: the 14-argument selection core''s, plus p_bound_ids uuid[]) and query_list_resources_with (14 arguments), each returning (resource_id uuid, created_epoch double precision), with no DEFAULT on any parameter. Both call the existing selection core unchanged. No existing function, table or default changes; no REPLACE and no DROP.'
);
