-- A machine's credential records who last minted its secret by rotation.
--
-- Task 01a19826-4e34-735c-a8c5-13ec6fb27644, S3 residual route 3, ruled by Pete 2026-11-01 (decision
-- 01a19843-b86b-766f-b06e-65560b0f6f6f). A delegate's grant to a machine they control is held to the
-- in-person grant right (`access_service::callers_machine_grant_bar`). "Control" counted the
-- registrant and the current owners of the machine's owning team. A co-owner could rotate the secret,
-- taking the credential, then step down or leave the owning team, and be counted as controlling
-- nothing while holding the key. Nothing recorded who had rotated: `secret_rotated_at` carries only
-- the time. This column records the rotator, and the grant door counts it as it counts
-- `registered_by_profile_id`.
--
-- NULL for a secret never rotated, including every row that predates this column: a rotation made
-- before it is unrecorded, and is counted only through the owning-team arm. Set by
-- `machine_client_service::rotate_secret` only. A rebind writes a new client row whose
-- `registered_by_profile_id` already names the rebinder.
--
-- ON DELETE SET NULL: a profile row is not deleted by any door today (erasure scrubs it in place),
-- and a rotator's deletion must not take the credential's row with it.

ALTER TABLE kb_machine_clients
    ADD COLUMN secret_rotated_by_profile_id uuid REFERENCES kb_profiles(id) ON DELETE SET NULL;

COMMENT ON COLUMN kb_machine_clients.secret_rotated_by_profile_id IS
  'Who minted the current secret by rotation (NULL if never rotated since this column was added). '
  'Counted, beside registered_by_profile_id, as controlling the machine for the in-person grant bar.';

-- The owning team's comment said it was never consulted for authorization. Since B2 D5 it decides
-- who may operate the credential (rotate, revoke, show), and since this task it also decides who
-- controls the machine. It still never confers READ REACH, which is what the old sentence protected.
COMMENT ON COLUMN kb_machine_clients.team_id IS
  'The machine OWNER -- which team a registration was performed on behalf of. Decides who may operate '
  'the credential (an owner of this team may rotate, revoke and show it) and so who controls the '
  'machine for the in-person grant bar and for the membership cascade when a controller leaves a team. '
  'It confers no read reach: reach is kb_access_grants plus team membership, both plural.';

SELECT declare_migration(
    20261115000010,
    'additive',
    'One new nullable column on kb_machine_clients and two column comments. A binary that predates it '
    'never names the column, and its inserts and updates leave it NULL.'
);
