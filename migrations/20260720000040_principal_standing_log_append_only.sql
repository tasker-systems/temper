-- The principal-standing log is append-only in ENFORCEMENT, not only in its COMMENT.
--
-- THE DEFECT THIS CLOSES. 20260720000010 gives kb_principal_standing_events the comment
-- "Append-only. NEVER UPDATE OR DELETE A ROW HERE." and enforces nothing. A comment asserting a
-- property the object does not hold is the same failure mode as `SystemAuthorized`, whose doc
-- claims a type-state guarantee its `pub` field does not enforce (temper-services/src/auth/mod.rs:258)
-- -- and it is exactly as misleading, because it will keep telling a reader the property exists.
-- Found by an adversarial probe, not by review: a plain UPDATE and a plain DELETE both succeeded.
--
-- WHY IT MATTERS MORE HERE THAN "AN AUDIT ROW WAS EDITED". `principal_prior_standing`
-- (20260720000030) reads THIS LOG to decide what `Reactivate` restores to -- the log is not merely
-- the audit trail, it is the authoritative input to a state transition. A rewritable log therefore
-- means the target of a reactivation can be changed after the fact: set `resulting_state` on the
-- pre-deactivation entry and a `Deactivated` principal comes back `Approved` instead of `Denied`.
-- That is privilege escalation through the table the design treats as ground truth for restoration,
-- and no amount of correctness in the Rust machine can see it -- `transition()` is handed `prior`
-- and faithfully returns it.
--
-- THIS IS THE HOUSE PATTERN, NOT A NEW ONE. kb_events -- the repo's other append-only log -- has
-- carried exactly this guard since 20260624000001_canonical_schema.sql:
--     CREATE TRIGGER kb_events_append_only BEFORE DELETE OR UPDATE ON kb_events
--       FOR EACH ROW EXECUTE FUNCTION kb_events_append_only()
-- The distinction the repo actually draws is log-vs-projection: append-only LOGS get a trigger,
-- mutable PROJECTIONS (kb_access_grants, and kb_principal_standing / kb_principal_governance) rely
-- on the function chokepoint by convention and carry no trigger. This migration puts the standing
-- log on the correct side of that line; the two projection tables are deliberately left alone.
--
-- BEFORE, not AFTER: the exception must pre-empt the write rather than roll it back.
-- FOR EACH ROW mirrors kb_events exactly -- a statement-level trigger would not fire on a
-- zero-row UPDATE, and matching the established shape is worth more than that micro-optimisation.
--
-- ADDITIVE. CREATE FUNCTION + CREATE TRIGGER only; no column, index, or trigger is dropped, so
-- Phase 1 stays additive-on-schema and rides auto-deploy on `main`.

CREATE FUNCTION kb_principal_standing_events_append_only()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'principal standing log is append-only';
END;
$$;

CREATE TRIGGER kb_principal_standing_events_append_only
    BEFORE DELETE OR UPDATE ON kb_principal_standing_events
    FOR EACH ROW
    EXECUTE FUNCTION kb_principal_standing_events_append_only();

COMMENT ON FUNCTION kb_principal_standing_events_append_only IS
  'Refuses any UPDATE or DELETE on kb_principal_standing_events. The log is the authoritative '
  'input to Reactivate (principal_prior_standing reads it), not merely an audit trail -- a '
  'rewritable entry would let a reactivation restore a state that was never held. Mirrors '
  'kb_events_append_only; repairs quarantine, they do not delete.';
