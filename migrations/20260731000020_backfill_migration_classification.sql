-- The 148 migrations that shipped before anything asked them to declare.
--
-- Task 019fb4e5-2365-7880-8cf1-7fea6755d60e · goal 019fb35b-c64e-7cd2-a7c0-aa117d1ab1a7.
-- Declaring function and vocabulary: `20260731000010`.

-- ── Why a backfill and not 148 file edits ───────────────────────────────────────────────────────
-- A shipped migration cannot be edited: sqlx compares `migration.checksum != applied_migration.
-- checksum` at `sqlx-core-0.8.6/src/migrate/migrator.rs:175` and refuses. So the 148 get their
-- declarations from here, and every migration from `20260731000010` onward declares in its own file.

-- ── The definition actually applied, and the imprecision it resolves ────────────────────────────
-- Spec §3 says `additive` is "safe with any binary in either direction". Read strictly that would
-- make almost every feature migration shape-breaking, since a new binary reading a column it just
-- added fails against the schema that predates it — and §3's own auto-apply rule would then cover
-- almost nothing. Three things in the design say the intended test is one direction:
--
--   1. The clause is named `a-migrations-compatibility-with-a-LAGGING-binary-is-stated-not-remembered`.
--   2. All three consequences §3 draws — no mismatch by construction, `vercel rollback` stays safe,
--      build ordering stops mattering — are about an OLD binary meeting a NEW schema.
--   3. The spec's own census counts 109 of 148 as purely additive, which is only reachable under
--      the one-direction reading.
--
-- So the test applied here is: **does a binary that predates this migration keep working against
-- the schema after it is applied?** No is `shape-breaking`. `[decided — 2026-07-31]` The spec's
-- §3 wording is corrected in the same change, so the two never disagree again.

-- ── How the 148 were classified ─────────────────────────────────────────────────────────────────
-- `[observed — 2026-07-31]` Three sweeps, then per-file review of everything any of them flagged.
--
--   Sweep 1 — breaking verbs (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN … TYPE, RENAME): 33.
--     For each, the previous definition of every dropped function was located in its own earlier
--     migration and the old and new signatures compared side by side. That is what separates a
--     parameter appended WITH a default (still resolves — additive) from one appended WITHOUT
--     (does not resolve — shape-breaking), and an appended output column (additive) from a replaced
--     or retyped one.
--
--   Sweep 2 — constraint tightening, which sweep 1 is structurally blind to: 14, of which 13 were
--     new. This is a real false-negative class, and it is why the remainder was NOT accepted on a
--     grep miss. Reviewed by asking whether the constrained table is written by the binary
--     unmediated; three of them are, and are classified shape-breaking on that evidence.
--
--   Sweep 3 — ADD COLUMN … NOT NULL without a default, apply-time DELETE, ALTER TYPE, DROP INDEX/
--     TRIGGER/CONSTRAINT over the 103 files the first two did not reach: 13 flagged, all cleared on
--     review (every NOT NULL carries a DEFAULT, every DELETE is inside a function body rather than
--     an apply-time data change, and the one ALTER TYPE is an enum ADD VALUE).
--
-- The ~90 files no sweep flagged are additive by a structural argument rather than by a sample:
-- with no breaking verb, no tightened constraint, and no NOT NULL/DELETE/ALTER TYPE, the only way
-- left to touch an existing object is `CREATE OR REPLACE FUNCTION` — and Postgres refuses to let
-- that change a signature or a return type. There is no remaining mechanism by which they could
-- break a lagging binary.
--
-- **The probe that produced sweep 3 was broken on its first run and returned zero for every
-- pattern, including a control that had to match.** `xargs` could not find `rg` on its PATH, so
-- nine "no hits" were nine failures to execute. Recorded because a sweep that cannot fail is not a
-- sweep, and this one silently looked like a clean corpus.

-- ── Two findings this review produced ───────────────────────────────────────────────────────────
-- **`20260727000040` declares itself ADDITIVE in its own header and is not.** It added a third,
-- required parameter to `steward_ingest_delta`, so the deployed two-argument call stops resolving.
-- Probed against the live schema rather than argued: `function steward_ingest_delta(uuid, uuid)
-- does not exist`. Its header is accurate about everything it ADDS and silent about the parameter
-- that breaks — the same species of error as the outage, found in a marker written to prevent it.
--
-- **The count disagrees with the spec's census and the disagreement is not resolved here.** The
-- spec estimated 39 non-additive and "24 callable-shape-breaking"; this review finds 10. Neither
-- number was validated when written, and the plan says so. This one is per-file and cites its
-- evidence in each `reason`, which is why it is the one recorded — but it is a different question
-- answered a different way, not a correction of the earlier figure, and the gap is worth a look.

-- ── What this migration is ──────────────────────────────────────────────────────────────────────
SELECT declare_migration(20260731000020, 'additive',
    'Inserts 148 rows into a table born one migration ago. Touches no pre-existing object.');

SELECT declare_migration(20260624000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260624000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260624000003, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260625000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260626000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260626000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260626000003, 'shape-breaking',
    'graph_subgraph_nodes: parameter p_context_name varchar became p_context_id uuid. A lagging binary''s call does not resolve against the new signature.');
SELECT declare_migration(20260627000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260627000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260627000003, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260628000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260629000001, 'shape-breaking',
    'Adds UNIQUE kb_content_blocks (resource_id, seq) WHERE NOT is_folded, on a table the binary INSERTs directly (embed_service.rs:646). A lagging binary writing a second live block at the same seq is rejected where it previously succeeded.');
SELECT declare_migration(20260629000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260629000003, 'additive',
    'Function parameters appended WITH defaults, so a lagging binary''s shorter positional call still resolves. No return type or output column removed or retyped.');
SELECT declare_migration(20260629000004, 'shape-breaking',
    'unified_search gained a 12th parameter p_scope_ids uuid[] with NO default, so the deployed 11-argument call stops resolving.');
SELECT declare_migration(20260629000005, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260629000006, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260629000007, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260630000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260630000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260701000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260701000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260701000003, 'additive',
    'Drops an object no binary ever named: git log -S over crates/ finds no Rust reference at any point, and a live pg_proc scan on 2026-07-31 finds no surviving SQL function referencing it. The functions that did read it were replaced in the same transaction.');
SELECT declare_migration(20260701000004, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260701000005, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260701000006, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260702000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260702000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260703000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260703000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260703120000, 'shape-breaking',
    'Adds a partial UNIQUE on pending kb_team_invitations (team_id, invited_email), on a table the binary INSERTs directly (invitation_service.rs:59). A lagging binary issuing a second pending invitation is rejected where it previously succeeded.');
SELECT declare_migration(20260703130000, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260703140000, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260704000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260704000002, 'additive',
    'Drops an object no binary ever named: git log -S over crates/ finds no Rust reference at any point, and a live pg_proc scan on 2026-07-31 finds no surviving SQL function referencing it. The functions that did read it were replaced in the same transaction.');
SELECT declare_migration(20260704000003, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260704000004, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260704000005, 'additive',
    'Output columns added to a RETURNS TABLE function; none removed or retyped. Verified 2026-07-31 that production Rust names its columns explicitly - the only three SELECT * against a set-returning function are one jsonb_populate_recordset and two tests - so a lagging binary''s named-column select is unaffected.');
SELECT declare_migration(20260704000006, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260704000007, 'additive',
    'Output columns added to a RETURNS TABLE function; none removed or retyped. Verified 2026-07-31 that production Rust names its columns explicitly - the only three SELECT * against a set-returning function are one jsonb_populate_recordset and two tests - so a lagging binary''s named-column select is unaffected.');
SELECT declare_migration(20260704000008, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260704000009, 'additive',
    'Output columns added to a RETURNS TABLE function; none removed or retyped. Verified 2026-07-31 that production Rust names its columns explicitly - the only three SELECT * against a set-returning function are one jsonb_populate_recordset and two tests - so a lagging binary''s named-column select is unaffected.');
SELECT declare_migration(20260705000001, 'additive',
    'Constraints are declared on objects this same migration creates, so there is no pre-existing write for them to reject.');
SELECT declare_migration(20260705000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260706000001, 'additive',
    'Output columns added to a RETURNS TABLE function; none removed or retyped. Verified 2026-07-31 that production Rust names its columns explicitly - the only three SELECT * against a set-returning function are one jsonb_populate_recordset and two tests - so a lagging binary''s named-column select is unaffected.');
SELECT declare_migration(20260706000002, 'additive',
    'Output columns added to a RETURNS TABLE function; none removed or retyped. Verified 2026-07-31 that production Rust names its columns explicitly - the only three SELECT * against a set-returning function are one jsonb_populate_recordset and two tests - so a lagging binary''s named-column select is unaffected.');
SELECT declare_migration(20260706000003, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260706120000, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260706120100, 'additive',
    'Drops an object no binary ever named: git log -S over crates/ finds no Rust reference at any point, and a live pg_proc scan on 2026-07-31 finds no surviving SQL function referencing it. The functions that did read it were replaced in the same transaction.');
SELECT declare_migration(20260706120200, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260706120300, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260706130000, 'additive',
    'Output columns added to a RETURNS TABLE function; none removed or retyped. Verified 2026-07-31 that production Rust names its columns explicitly - the only three SELECT * against a set-returning function are one jsonb_populate_recordset and two tests - so a lagging binary''s named-column select is unaffected.');
SELECT declare_migration(20260707000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260707140000, 'additive',
    'Output columns added to a RETURNS TABLE function; none removed or retyped. Verified 2026-07-31 that production Rust names its columns explicitly - the only three SELECT * against a set-returning function are one jsonb_populate_recordset and two tests - so a lagging binary''s named-column select is unaffected.');
SELECT declare_migration(20260708000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260708000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260708000003, 'additive',
    'Output columns added to a RETURNS TABLE function; none removed or retyped. Verified 2026-07-31 that production Rust names its columns explicitly - the only three SELECT * against a set-returning function are one jsonb_populate_recordset and two tests - so a lagging binary''s named-column select is unaffected.');
SELECT declare_migration(20260708000004, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260708000005, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260708000006, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260708000007, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260708000008, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260708000009, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260708000012, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260709000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260709000002, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260709000003, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260709000004, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260709000005, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260709000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260709000011, 'additive',
    'Output columns added to a RETURNS TABLE function; none removed or retyped. Verified 2026-07-31 that production Rust names its columns explicitly - the only three SELECT * against a set-returning function are one jsonb_populate_recordset and two tests - so a lagging binary''s named-column select is unaffected.');
SELECT declare_migration(20260709000020, 'additive',
    'Drops an object no binary ever named: git log -S over crates/ finds no Rust reference at any point, and a live pg_proc scan on 2026-07-31 finds no surviving SQL function referencing it. The functions that did read it were replaced in the same transaction.');
SELECT declare_migration(20260709000030, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260709000040, 'shape-breaking',
    'Adds UNIQUE kb_entities (profile_id, name), on a table the binary INSERTs directly (materialize_service.rs:134, scenario loaders). Its own header claims additive because no row is dropped and no column altered - true, and blind to the write path, which is where a new UNIQUE bites.');
SELECT declare_migration(20260709000050, 'additive',
    'Function parameters appended WITH defaults, so a lagging binary''s shorter positional call still resolves. No return type or output column removed or retyped.');
SELECT declare_migration(20260710000001, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260710000010, 'additive',
    'Function parameters appended WITH defaults, so a lagging binary''s shorter positional call still resolves. No return type or output column removed or retyped.');
SELECT declare_migration(20260711000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260711000011, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260711000020, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260711000030, 'additive',
    'Function parameters appended WITH defaults, so a lagging binary''s shorter positional call still resolves. No return type or output column removed or retyped.');
SELECT declare_migration(20260711000040, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260711000050, 'additive',
    'Function parameters appended WITH defaults, so a lagging binary''s shorter positional call still resolves. No return type or output column removed or retyped.');
SELECT declare_migration(20260711000060, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260711000070, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260712000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260712000020, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260712000030, 'additive',
    'WIDENS an existing CHECK (DROP CONSTRAINT then ADD a more permissive one). A lagging binary''s writes were already a subset of what the new constraint admits.');
SELECT declare_migration(20260712000040, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260712000050, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260712000060, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260712000070, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260712000080, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260712000090, 'additive',
    'Function parameters appended WITH defaults, so a lagging binary''s shorter positional call still resolves. No return type or output column removed or retyped.');
SELECT declare_migration(20260712000110, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260713000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260713000020, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260713000040, 'additive',
    'Function parameters appended WITH defaults, so a lagging binary''s shorter positional call still resolves. No return type or output column removed or retyped.');
SELECT declare_migration(20260713000050, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260714000001, 'additive',
    'Constraints are declared on objects this same migration creates, so there is no pre-existing write for them to reject.');
SELECT declare_migration(20260714000002, 'additive',
    'Constraints are declared on objects this same migration creates, so there is no pre-existing write for them to reject.');
SELECT declare_migration(20260714000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260714000020, 'additive',
    'WIDENS an existing CHECK (DROP CONSTRAINT then ADD a more permissive one). A lagging binary''s writes were already a subset of what the new constraint admits.');
SELECT declare_migration(20260715000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260715000020, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260715000030, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260715000040, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260716000010, 'additive',
    'Output columns added to a RETURNS TABLE function; none removed or retyped. Verified 2026-07-31 that production Rust names its columns explicitly - the only three SELECT * against a set-returning function are one jsonb_populate_recordset and two tests - so a lagging binary''s named-column select is unaffected.');
SELECT declare_migration(20260716000020, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260716000040, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260717000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260717000020, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260717000030, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260718000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260718000020, 'additive',
    'Narrows kb_event_types.category and adds an FK from kb_events, but neither table is written unmediated by production Rust - events are appended through _event_append, which this migration keeps consistent, and kb_event_types is seeded by migrations and bootseed only.');
SELECT declare_migration(20260718000030, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260719000010, 'additive',
    'Narrows kb_event_types.category and adds an FK from kb_events, but neither table is written unmediated by production Rust - events are appended through _event_append, which this migration keeps consistent, and kb_event_types is seeded by migrations and bootseed only.');
SELECT declare_migration(20260719000020, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260720000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260720000020, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260720000030, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260720000040, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260720000100, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260720000110, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260720000120, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260721000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260721000020, 'additive',
    'Constraints are declared on objects this same migration creates, so there is no pre-existing write for them to reject.');
SELECT declare_migration(20260722000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260722000100, 'shape-breaking',
    'Drops kb_profiles.system_access, kb_profiles.is_active and kb_system_settings.access_mode. A contract migration is only safe once its expand phase has DEPLOYED, and nothing records that a merge deployed - this goal''s founding lesson. Gated deliberately.');
SELECT declare_migration(20260724000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260724000020, 'additive',
    'Function parameters appended WITH defaults, so a lagging binary''s shorter positional call still resolves. No return type or output column removed or retyped.');
SELECT declare_migration(20260724000030, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260724000040, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260724000110, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260724000120, 'shape-breaking',
    'resource_standing_shape''s output columns were replaced and retyped (indep_breadth/adversarial_survival/challenge_count -> citation_magnitude/audit_coverage/citation_quality), standing_band''s parameter list changed, kb_independence_pairs dropped, three kb_resource_standing columns with it. Already an operator-gated cutover in DEPLOYING.md.');
SELECT declare_migration(20260724000130, 'shape-breaking',
    'Rides the 20260724000120 cutover; DEPLOYING.md instructs taking it in the same window. The narrow decode test would call it additive - workflow_job_claim''s new p_principal is appended WITH a default - but its DROP+CREATE is a moment a concurrently running steward or embed tick can fail. Classified by the operational meaning of the class, not the narrow test.');
SELECT declare_migration(20260724000200, 'additive',
    'Adds kb_citation_audits.audited_by_profile_id and SETs it NOT NULL, but every write reaches that table through _project_citation_audited, which this same migration updates to fill the column. The only direct INSERT in Rust first appeared the same day, in the same branch, so no lagging binary writes it unmediated.');
SELECT declare_migration(20260724000210, 'additive',
    'DROP+CREATE of a function whose parameter list and return type are identical before and after; only the body changed. Nothing a lagging binary calls or decodes moved.');
SELECT declare_migration(20260724000220, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260726000010, 'additive',
    'DROP+CREATE of a function whose parameter list and return type are identical before and after; only the body changed. Nothing a lagging binary calls or decodes moved.');
SELECT declare_migration(20260726000030, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260727000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260727000020, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260727000030, 'additive',
    'DROP+CREATE of a function whose parameter list and return type are identical before and after; only the body changed. Nothing a lagging binary calls or decodes moved.');
SELECT declare_migration(20260727000040, 'shape-breaking',
    'steward_ingest_delta gained a THIRD parameter p_fingerprint text with no default, so the deployed 2-argument call stops resolving. Probed 2026-07-31 against the live schema: function steward_ingest_delta(uuid, uuid) does not exist. This migration''s own header claims ADDITIVE and enumerates only what it adds; the required parameter it also added is the thing that breaks.');
SELECT declare_migration(20260727000050, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260728000010, 'additive',
    'Adds only. Cleared all three sweeps of 2026-07-31: no breaking verb (DROP FUNCTION/TABLE/VIEW/COLUMN, ALTER COLUMN TYPE, RENAME), no constraint tightened on a pre-existing table, and no ADD COLUMN NOT NULL without a default, no apply-time DELETE, no ALTER TYPE. Where it redefines a function it uses CREATE OR REPLACE, which Postgres will not let change a signature or return type.');
SELECT declare_migration(20260730000010, 'shape-breaking',
    'facet_set and property_set went from RETURNS uuid to RETURNS uuid[]. The running binary decodes by type, so it is a caller nobody updated. The 2026-07-30 production outage that motivated this whole mechanism.');
