-- Authorship of a map in the admin-only regime belongs to a system admin, whatever grants exist.
--
-- A cognitive map is in the admin-only regime when it is the reserved L0 kernel, or when it is
-- bound to an instance team: temper-system, or the team kb_system_settings.gating_team_slug names.
-- Both instance teams reach every profile on the instance, so what a map bound to either says, it
-- says to everyone (access_service::is_root_or_gating_team is the Rust spelling of "instance team").
--
-- Until now the regime lived only in Rust (access_service::cogmap_write_requires_admin), and only
-- reconcile and the grant door asked it. Content creation into a map asks
-- cogmap_authorable_by_profile, which read the explicit write grant and nothing else. So the regime
-- governed who may mint a write grant, but not whether a grant already on the map still authored
-- it, and a map entered the regime with its grants intact: by a bind, or by an admin repointing
-- gating_team_slug at a team whose maps already carry grants. The regime also matched the gating
-- team only, so a map bound to temper-system was outside it whenever the gating slug named some
-- other team.
--
-- The rule, decided once and here: a map in the regime is authored by a system admin and by nobody
-- else, the same answer reconcile gives (DbBackend::authorize_reconcile); every other map is
-- authored by an explicit write grant, unchanged. Grants on a regime map stay in place and are
-- inert while the map is in the regime; unbinding the map (an admin act) makes them live again.
-- Placing this in cogmap_authorable_by_profile reaches every path that authors into a map, because
-- they all ask it: resource create and update (check_cogmap_authorable), the container-write
-- cascade in can_modify_resource, edges, blobs, shapes, invocation_open and the steward watermark.
--
-- cogmap_write_requires_admin(uuid) is new, and becomes the one definition of the regime: the Rust
-- function of the same name now calls it instead of restating it. Fail-closed on L0 as before:
-- with gating unconfigured is_system_admin is false for everyone, so the kernel is immutable.
--
-- Measured before writing, on a branch of main taken 2026-10-31: the only map bound to an instance
-- team is the L0 kernel, and no regime map carries a write or grant row, so no principal's access
-- changes on that deployment.

CREATE OR REPLACE FUNCTION public.cogmap_write_requires_admin(p_cogmap uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
    SELECT p_cogmap = '00000000-0000-0000-0005-000000000001'::uuid   -- the reserved L0 kernel
        OR EXISTS (
            SELECT 1
              FROM kb_team_cogmaps tc
              JOIN kb_teams t ON t.id = tc.team_id
             WHERE tc.cogmap_id = p_cogmap
               AND (t.slug = 'temper-system'
                    OR t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1))
        );
$function$;

COMMENT ON FUNCTION public.cogmap_write_requires_admin(uuid) IS
  'Is this map in the admin-only regime: the L0 kernel, or bound to temper-system or the gating '
  'team? Caller-independent. A soft-deleted instance team still counts (fail-closed). The one '
  'definition; access_service::cogmap_write_requires_admin calls it.';

CREATE OR REPLACE FUNCTION public.cogmap_authorable_by_profile(p_profile uuid, p_cogmap uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
    SELECT CASE
             WHEN cogmap_write_requires_admin(p_cogmap) THEN is_system_admin(p_profile)
             ELSE profile_explicit_grant(p_profile, 'write', 'kb_cogmaps', p_cogmap)
           END;
$function$;

COMMENT ON FUNCTION public.cogmap_authorable_by_profile(uuid, uuid) IS
  'May this profile author into this map? A map in the admin-only regime '
  '(cogmap_write_requires_admin): a system admin only, and grants on it are inert. Every other map: '
  'an explicit write grant (cogmaps have no owner). Membership alone never authors.';

SELECT declare_migration(
    20261113000010,
    'additive',
    'New function cogmap_write_requires_admin(uuid), and a body-only CREATE OR REPLACE of '
    'cogmap_authorable_by_profile with signature and return type unchanged: a map bound to '
    'temper-system or the gating team, or the L0 kernel, is authored by a system admin only. '
    'Behaviour change, deliberate. The running binary reads the regime from its own query until it '
    'is replaced, so applying ahead of it is safe. No row is rewritten.'
);
