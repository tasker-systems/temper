-- Migration: vw_join_requests — the join-request row with its team + requester joined.
--
-- Code-quality audit 2026-06-26 (docs/code-reviews/2026-06-26-code-quality-audit.md,
-- chunk 15, CQ-13 duplicated-join-projection). The 13-column kb_join_requests
-- projection + `JOIN kb_teams` was copy-pasted across the access_service read sites;
-- this view states the projection and joins once, and `get_own_request` /
-- `list_pending_requests` SELECT from it.
--
-- Deliberately NOT total DRY: `create_join_request` / `review_request` keep inline
-- projections — theirs are RETURNING clauses on INSERT/UPDATE, which a joined view
-- cannot serve, and the `query_as!` macros compile-check those copies against the
-- JoinRequest struct so they cannot silently drift.
--
-- (Chunk 15's other half — the duplicated context-visibility predicate — was already
-- resolved by migration 20260627000001's `context_visible_to(principal, context)`
-- function; the retired `contexts_visible_to` view name stays retired.)
CREATE VIEW vw_join_requests AS
SELECT jr.id,
       jr.team_id,
       jr.requesting_profile_id,
       jr.status,
       jr.message,
       jr.source,
       jr.accepted_terms_version,
       jr.accepted_terms_at,
       jr.reviewed_by_profile_id,
       jr.reviewed_at,
       jr.decision_note,
       jr.created,
       jr.updated,
       t.slug AS team_slug,
       p.display_name,
       p.email
  FROM kb_join_requests jr
  JOIN kb_teams t ON t.id = jr.team_id
  JOIN kb_profiles p ON p.id = jr.requesting_profile_id;
