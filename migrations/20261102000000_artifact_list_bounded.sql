-- The bounded per-resource artifact list: a new arity of `artifacts_for_resource` that takes a row
-- limit and a keyset position.
--
-- The 5-argument function (`20260822000020`) is untouched and still serves the route when a
-- caller passes no `limit`. This arity differs from it in three places only: the keyset
-- predicate, `LIMIT`, and the argument list. The visibility gate, the projection and
-- `ORDER BY a.created, a.id` are the incumbent's, byte for byte.
--
-- Three things a later edit can break silently:
--   1. NO `DEFAULT` on any parameter. A default on this arity makes a 5-argument call ambiguous
--      against the incumbent, and every existing caller an error (`20260815000040`, point 1).
--   2. The keyset is `(created, id)` as a ROW comparison, in the same order the `ORDER BY` uses.
--      Comparing `created` alone drops or repeats the rows that share a timestamp.
--   3. Failing closed. An absent position is BOTH arguments NULL. The continuation arm demands
--      BOTH non-NULL, spelled out: a row comparison against `(X, NULL)` is not NULL but true for
--      every `created > X`, so without it half a position would resume. Half a position returns
--      nothing. A NULL `p_limit` returns nothing rather than the whole set: this arity cannot be
--      asked for everything.
--
-- The caller reads `limit + 1` rows to learn whether a next page exists, without a count.

CREATE FUNCTION artifacts_for_resource(p_profile uuid, p_resource uuid,
                                       p_kind text,
                                       p_intent text,
                                       p_include_folded boolean,
                                       p_limit bigint,
                                       p_after_created timestamptz,
                                       p_after_id uuid)
RETURNS TABLE(
    artifact_id        uuid,
    resource_id        uuid,
    kind_owner_table   varchar,
    kind_owner_id      uuid,
    artifact_kind      text,
    intent             text,
    precedence         double precision,
    content_hash       text,
    content_bytes      bigint,
    shape_state        text,
    is_folded          boolean,
    created            timestamptz,
    content            jsonb
)
LANGUAGE sql STABLE AS $$
    SELECT a.id,
           a.resource_id,
           a.kind_owner_table,
           a.kind_owner_id,
           a.artifact_kind,
           a.intent,
           a.precedence,
           a.content_hash,
           a.content_bytes,
           _data_artifact_shape_state(a.id),
           a.is_folded,
           a.created,
           c.content
      FROM _visible_artifacts(p_profile, p_resource, p_kind, p_intent, p_include_folded) va
      JOIN kb_data_artifacts a ON a.id = va.id
      LEFT JOIN kb_data_artifact_content c ON c.artifact_id = a.id
     WHERE (p_after_created IS NULL AND p_after_id IS NULL)
        OR (p_after_created IS NOT NULL AND p_after_id IS NOT NULL
            AND (a.created, a.id) > (p_after_created, p_after_id))
     ORDER BY a.created, a.id
     LIMIT coalesce(p_limit, 0)
$$;

COMMENT ON FUNCTION artifacts_for_resource(uuid, uuid, text, text, boolean, bigint, timestamptz, uuid) IS
    'One bounded page of a resource''s visible artifacts in (created, id) order, hydrated as the 5-argument arity hydrates them. Returns at most p_limit rows strictly after the (p_after_created, p_after_id) position, or from the start when both are NULL. A NULL p_limit returns no rows.';

SELECT declare_migration(
    20261102000000,
    'additive',
    'Adds a new 8-argument arity of artifacts_for_resource (limit and keyset position) with no DEFAULT on any parameter, so the existing 5-argument function and every call to it resolve exactly as before. No existing object changes; a binary without this change never calls the new arity.'
);
