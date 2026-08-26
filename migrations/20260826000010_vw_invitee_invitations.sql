-- Migration: vw_invitee_invitations — "the pending invitations addressed to a profile",
-- stated once so that counting them and listing them cannot disagree.
--
-- WHY A VIEW AND NOT A SECOND QUERY
-- ---------------------------------
-- `invitation_service::list_for_profile` carried the whole predicate inline: pending,
-- unexpired, un-revoked, on an active team, addressed to a verified email that exactly
-- one profile owns. `temper warmup` needs the COUNT of that same set, and the count is
-- read on every session start.
--
-- Copying five clauses into a second query is the failure this repo already named:
-- "when multiple queries need the same WHERE filters (e.g. list rows + count), extract
-- a filter-builder — never duplicate filter logic across queries, the two copies will
-- drift." A drifted copy here does not error; it reports a different number of waiting
-- invitations than the command it tells you to run, and the reader has no way to tell
-- which one lied. So the predicate moves into the view and both sites SELECT from it.
--
-- THE JOIN IS DEDUPLICATED, AND THAT IS NOT COSMETIC
-- --------------------------------------------------
-- The inline original tested `lower(i.invited_email) IN (SELECT ...)`, which is set
-- semantics: a profile holding the SAME verified email on two auth links (two identity
-- providers, one address — ordinary) matched once. A plain JOIN on that subquery would
-- match twice and report one waiting invitation as two. `SELECT DISTINCT profile_id,
-- lower(email)` restores the set semantics the `IN` had for free.
--
-- The uniqueness conjunct is carried verbatim rather than restated: an address claimed
-- by more than one verified profile addresses nobody, because the invitation cannot be
-- attributed. That is the incumbent rule and this migration does not move it.
CREATE VIEW vw_invitee_invitations AS
SELECT i.id,
       i.team_id,
       t.slug AS team_slug,
       t.name AS team_name,
       i.invited_email,
       i.invited_by_profile_id,
       i.role,
       i.token,
       i.status,
       i.expires_at,
       i.created,
       inv.profile_id AS invitee_profile_id
  FROM kb_team_invitations i
  JOIN kb_teams t ON t.id = i.team_id
  JOIN (
        SELECT DISTINCT al.profile_id, lower(al.email) AS email
          FROM kb_profile_auth_links al
         WHERE al.email IS NOT NULL
           AND al.email_verified
           AND (SELECT COUNT(DISTINCT al2.profile_id)
                  FROM kb_profile_auth_links al2
                 WHERE lower(al2.email) = lower(al.email)
                   AND al2.email_verified) = 1
       ) inv ON inv.email = lower(i.invited_email)
 WHERE i.status = 'pending'
   AND i.expires_at > now()
   AND i.revoked_at IS NULL
   AND t.is_active;

COMMENT ON VIEW vw_invitee_invitations IS
  'Pending invitations addressed to a profile, keyed by invitee_profile_id. The single '
  'statement of "what is waiting on me" — listed by GET /api/invitations/mine and counted '
  'by GET /api/invitations/mine/count, so the two can never report different sets.';

SELECT declare_migration(
    20260826000010,
    'additive',
    'One new view, vw_invitee_invitations, and nothing else: no column, constraint, function signature or existing view is altered, and no shipped binary can reach it because the name is new (task 01a03bb7-de22-7aa0-a7f0-9b8698a57743). It exists so that counting the caller''s pending invitations and listing them read ONE predicate instead of two copies. temper warmup runs from the SessionStart hook and previously obtained its invitation count by fetching every full row -- including each invitation''s redemption token, a bearer capability transferred purely so that .len() could be taken -- and the fix is a count-shaped read, which needs the list''s five-clause predicate (pending, unexpired, un-revoked, active team, verified uniquely-owned email) at a second call site. THE DEDUPLICATED JOIN IS A CORRECTNESS DETAIL, NOT A TIDY-UP: the inline predicate this view replaces used `lower(invited_email) IN (subquery)`, whose set semantics matched once for a profile holding one verified address on two auth links; a naive JOIN would match that profile twice and count one waiting invitation as two, so the subquery is SELECT DISTINCT. The uniqueness conjunct -- an address claimed by more than one verified profile addresses nobody -- is carried verbatim from crates/temper-services/src/services/invitation_service.rs:363-372 rather than restated, and this migration does not move that rule. A lagging binary meeting this schema is unaffected: it selects from kb_team_invitations directly and never names the view. Deliberately NOT indexed -- the view is a projection over an existing join and the counted set is per-principal and small; if it ever needs one the index belongs on the base table.'
);
