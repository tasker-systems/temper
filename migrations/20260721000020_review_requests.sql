-- Review requests (spec 2026-07-20 D15).
--
-- A Revoked principal must be able to ask for reconsideration WITHOUT that request being able to
-- erase the revocation. The rejected alternative -- allow Revoked -> Request -> Requested and have
-- Withdraw return to the PRIOR state -- works, but preserves the audit signal by careful
-- bookkeeping. D15 makes it structural instead: there is no path out of Revoked except an admin
-- act, so there is nothing to launder.
--
-- It is also the more honest model. "Please let me in" and "please reconsider your decision" are
-- different speech acts with different admin context -- a reviewer needs the revocation reason,
-- which a plain Request has no slot for.
--
-- THIS TABLE IS AN INBOX SIGNAL, NEVER AN ADMISSION INPUT (D15 obligation 1). Admission reads
-- standing and nothing else; a Revoked principal is refused whether or not a review is pending.
-- ANDing this into the decision would restore precisely the conjunction-across-provisional-facts
-- shape D2 forbids -- and it is THE tempting change, which is why it is stated here and tested in
-- `a_pending_review_does_not_change_admission`.
--
-- ITS OPEN/DECIDED LIFECYCLE IS NOT A REGRESSION ON D5. We remove kb_join_requests.status in Phase
-- 2 because it DUPLICATED standing. A review's open/decided state duplicates nothing -- standing
-- stays `revoked` throughout, whatever the outcome. Different question, different answer.

CREATE TABLE kb_principal_review_requests (
    id            uuid PRIMARY KEY DEFAULT uuid_generate_v7(),
    profile_id    uuid NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    message       text,
    created       timestamptz NOT NULL DEFAULT now(),
    decided_at    timestamptz,
    decided_by    uuid REFERENCES kb_profiles(id),
    decision_note text
);

-- ITS OWN DUPLICATE GUARD (D15 obligation 2). For join requests, `requested` standing IS the
-- duplicate guard (D12) -- but a review does not move standing, so it inherits none. This is what
-- idx_join_requests_one_pending used to do, reappearing for a different reason. Note it is
-- per-PRINCIPAL, with no team dimension, which is more correct under D9 than the index it echoes.
CREATE UNIQUE INDEX idx_principal_review_one_open
    ON kb_principal_review_requests (profile_id)
    WHERE decided_at IS NULL;

COMMENT ON TABLE kb_principal_review_requests IS
  'A revoked principal asking for reconsideration (spec D15). AN INBOX SIGNAL ONLY -- never read '
  'by the admission decision. If you are here to AND this into has_system_access, re-read D2.';
