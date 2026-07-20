-- THE CUTOVER (spec 2026-07-20 §7, §9, D2, D10).
--
-- Everything before this migration is inert. After it, standing is the gate and governance is
-- admin-ness. Deliberately its own migration so it can be reasoned about -- and if necessary
-- reverted -- alone.
--
-- NUMBERING: …000110, NOT the plan's …000040/…000050. Two independent reasons, both load-bearing:
--   1. …000040 is already taken by 20260720000040_principal_standing_log_append_only.sql.
--   2. More importantly, this migration and 20260720000100_is_system_admin_nonempty_gating_slug.sql
--      BOTH `CREATE OR REPLACE FUNCTION is_system_admin`. Migrations apply in filename order, so
--      whichever sorts LAST wins the final body on any fresh database (CI, a new enterprise
--      instance, a local reset). Numbered below …000100, this repoint would apply first and then
--      …000100 would silently REVERT is_system_admin back to gating-team ownership -- while prod,
--      where …000100 is already applied, would keep the governance body. Prod and fresh DBs would
--      diverge invisibly. Numbered above …000100, this repoint has the final say everywhere.
--      …000100 documents itself as SUPERSEDED by exactly this cutover.
--
-- SIGNATURES ARE UNCHANGED, so all call sites follow with no code change. That is the whole
-- economics of D10, and the chokepoint is the SQL BODY, not the Rust wrapper: 21 production Rust
-- call sites reach is_system_admin through access_service::is_system_admin, which is a pure
-- passthrough (`SELECT is_system_admin($1)`), and one caller is IN-DATABASE
-- (20260715000010_context_reassign_fns.sql:76) and never touches Rust at all. Repointing the body
-- moves all 22 at once.
--
-- EXISTS, NOT A SCALAR COMPARISON. Measured on local dev 2026-07-20 with kb_system_settings
-- emptied, in BEGIN/ROLLBACK:
--
--     empty-settings has_system_access = <NULL> (is null: t)
--     IF NOT has_system_access(...) => GUARD DID NOT FIRE      <- fail-OPEN
--     WHERE-shape rows returned = 0                            <- fail-CLOSED
--
-- A NULL in a WHERE clause is fail-closed; a NULL in plpgsql `IF NOT` is fail-OPEN. There are
-- exactly two `IF NOT <predicate>` sites in this repo (a naive grep returns 14; twelve are
-- `IF NOT FOUND`, a row-count diagnostic):
--
--   20260629000002_auto_join_team_generalization.sql:44  IF NOT has_system_access(...)
--   20260715000010_context_reassign_fns.sql:76           IF NOT is_system_admin(...)   <- falls
--                                                        open into SYSTEM ADMIN
--
-- Both are fixed by making the predicates total here. Neither file is edited. The old bodies were
-- shaped `SELECT ... FROM settings`, so an empty kb_system_settings made both return NULL today --
-- a LATENT trap, not a live exploit (the row is seeded by 20260624000003_canonical_seed.sql:23,
-- pinned by CHECK (id = 1), and every production writer is an UPDATE; there are no DELETEs). The
-- new table must not inherit it.
--
-- ORDER IS DELIBERATE: is_system_admin first. Both are read during the same deploy and it guards
-- the higher-blast-radius site.

CREATE OR REPLACE FUNCTION is_system_admin(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    -- D10: admin-ness IS a governance row. Gating-team ownership no longer carries authorization
    -- meaning, which is what makes the ~20 uncoordinated writers to kb_team_members harmless --
    -- they became ordinary team-role churn the moment this body stopped reading them.
    --
    -- Note there is no AND against standing here. The invariant "admin implies Approved" is
    -- maintained by the transition (Revoke and Deactivate demote) and guarded at promotion, never
    -- checked at read time -- ANDing across two tables at read time is the exact shape D2 forbids.
    SELECT EXISTS (
        SELECT 1 FROM kb_principal_governance g WHERE g.profile_id = p_profile_id
    )
$$;

CREATE OR REPLACE FUNCTION has_system_access(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    -- D2: one authoritative state in one table. No access_mode, no gating-team membership, no
    -- tier. Absence denies structurally (spec §7 obligation 1), which is what makes D7's
    -- connection-profile safety hold without a check anyone can forget.
    SELECT EXISTS (
        SELECT 1 FROM kb_principal_standing s
         WHERE s.profile_id = p_profile_id
           AND s.state = 'approved'
    )
$$;

COMMENT ON FUNCTION has_system_access IS
  'May this principal use this instance? Reads kb_principal_standing and NOTHING else (spec D2). '
  'EXISTS, never a scalar comparison: a NULL here falls through plpgsql `IF NOT` guards, fail-OPEN.';
COMMENT ON FUNCTION is_system_admin IS
  'May this principal change the rules? Reads kb_principal_governance and NOTHING else (spec D10).';
