-- Connection observed reach (S1 chunk B3 remainder): persist what the attach-time
-- mint actually saw, so the grant path can compare it against the declared reach.
--
-- Additive and all-nullable: existing rows are untouched (NULL = the credential has
-- never been minted, or the mint did not return reach metadata). Additive-only ⇒
-- `main` stays auto-deployable; this lands on the live, prod-migrated `kb_connections`
-- table (born in 20260714000010_connections.sql).
--
-- This column is the WITNESS the grant path consults. B4 surfaces the observed reach
-- on the attach response; without persistence it is ephemeral and gone by grant time.
-- B3's grant gate previously fired on `declares_reach()` alone — a static declaration.
-- With this column the gate fires when the mint's observed reach disagrees with the
-- declared reach (the remote-domain gap the mint can actually detect), or when the
-- credential was never minted at all. Remote-vs-temper scope stays incommensurable and
-- uncomputed — no `exceeds_temper_reach` bool — but the remote-domain drift the mint
-- witnesses is commensurable and now load-bearing.
ALTER TABLE kb_connections
  ADD COLUMN observed_reach JSONB NULL;

COMMENT ON COLUMN kb_connections.observed_reach IS
  'What the attach-time mint observed the credential can actually see (the provider''s mint `metadata`), as a provider-shaped JSON blob. NULL = the credential was never minted, or the mint returned no reach metadata. Persisted at `attach_credential` so the grant path can compare it against the declared reach (`reach_granularity`/`reach_covers`): a disagreement within the remote domain is the commensurable gap the mint can detect, and it sharpens the grant gate from "declares reach ⇒ must affirm" to "declares reach AND (gap OR unobserved) ⇒ must affirm". This is NOT a computed `exceeds_temper_reach` bool — remote and temper scope remain incommensurable; only the remote-domain observed-vs-declared drift is compared, and only to decide whether affirmation is required, never to auto-deny.';

SELECT declare_migration(
    20260818000010,
    'additive',
    'Adds nullable column kb_connections.observed_reach (JSONB). Additive and all-nullable: existing rows are untouched (NULL = never minted). The pre-deploy binary neither writes nor reads the column — it is written by `attach_credential` and read by `grant_reach`''s gate, both landing in the same deploy that applies this migration. No enum change, no wire-contract move, no index, no constraint. Additive-only ⇒ `main` stays auto-deployable.'
);