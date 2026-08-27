# The database is the outermost trust ring

**Date:** 2026-08-26
**Status:** Decided — ratifies an already-published position
**Scope:** `kb_events` and what the ledger does and does not cover
**Task:** `01a035f2-d37a-7a83-9f6c-b93d58eb5847`

## Decision

The event ledger (`kb_events`) records **acts that pass through the application's SQL mutation
functions**. A connection holding `DATABASE_URL` — an operator, a DBA, **or the application's own
credential** — can write state with no ledger row, and can insert forged ledger rows.

Audit at the Postgres boundary is a **named non-goal**, not an unfinished edge.

## This is a ratification, not a new statement

The position is already published, deliberately, in prose written for exactly this purpose.

`/operating/governance-and-administration`:

> **The ledger stops at the persistence layer.** A command issued straight to Postgres can bypass
> the event stream entirely. That's not a hole in the audit — it's a **system-responsibility
> boundary**: below the application, you're in the domain of database controls and infrastructure
> policy, not Temper's ledger.

`/operating/observability-and-audit`, under "One non-goal we'd rather name":

> Audit at the **Postgres boundary**, protecting against someone with direct database access, is out
> of scope on purpose. […] Better to be clear about where the system's guarantees stop than to imply
> they reach further than they do.

`AuditHomesDiagram.svelte` draws it — a dashed line beneath everything, marking the boundary below
which direct database commands fall outside the ledger.

**The actionable delta is not that the claim is unwritten. It is that `docs/` is silent on it** —
`docs/concepts/trust-boundary.md` covers only the 401/402 API gates and never mentions the database,
`psql`, or direct SQL. That page is where the install playbooks send an operator immediately before
handing them a `psql` prompt: `docs/playbooks/bootstrap-an-org.md` runs `system-bootstrap.sh
--run-root` against `DATABASE_URL`, and `docs/playbooks/enterprise-install.md` requires *"`psql` and
`DATABASE_URL_UNPOOLED` for the DB-only steps."* The bypass is not hypothetical; it is a documented
install step, described on a page that does not say what it is outside of.

## Name the table

"The ledger" is ambiguous across three append-only logs with **three different enforcement levels**,
and a record that blurs them is worse than none:

| Log | Append-only enforcement |
|---|---|
| `kb_events` | trigger `kb_events_append_only` (`migrations/20260624000001_canonical_schema.sql`) |
| `kb_principal_standing_events` | trigger (`migrations/20260720000040_principal_standing_log_append_only.sql`) |
| `kb_migration_ledger` | **comment only** (`migrations/20260731000010_migration_ledger.sql`) |

This decision is about `kb_events`.

## Why

Coverage stops where the abstraction stops. Events are written through one SQL chokepoint —
`_event_append`, *"THE ONE EVENT WRITER"* — and mutation functions are two-part: append, then
project. Rust calls the function rather than writing the row.

Chokepoint-in-SQL was chosen over triggers **deliberately**, and both halves of the reasoning are on
record. `migrations/20260718000010_admin_grant_fns.sql` on why not a Rust sink: *"A Rust
service-layer sink would also MISS `connection_service::grant_reach`."*
`migrations/20260719000010_admin_cognition_firewall_declarative.sql` on why not trigger-emission:
*"A trigger doing a category lookup per row would be new cost on every `_event_append` AND on every
row of replay's bulk restore."*

Below the application you are in the domain of database controls and infrastructure policy — a
domain whose shape belongs to the operating organization, not to Temper.

## The ring is wider than "a DBA"

The application connects as the **owning role**. `migrations/20260808000030_composable_find_family.sql`
states it as an accepted residue:

> anyone holding psql, a Neon console, **or the app credentials** can call a core with an arbitrary
> `uuid[]` and receive ungated rows. **`REVOKE` buys nothing, because the application connects as
> the owning role.**

There is not one `GRANT`, `REVOKE`, `CREATE POLICY`, or `ENABLE ROW LEVEL SECURITY` statement in any
of the 203 migrations. One connection string, one role, no runtime `SET ROLE`. `DEPLOYING.md`
directs the migration cutover to *"Connect as `neondb_owner`, unpooled."*

So the honest phrasing is **"anyone holding the database credentials, the application's included"** —
not "a DBA."

## How it is enforced, and the three facts that must not be collapsed

`kb_events_append_only` raises on any attempt to modify a row. But its scope needs stating precisely,
because the three facts below are separate and each matters:

1. **It covers `UPDATE OR DELETE`. It does not cover `INSERT`.** A connection holding the credentials
   can **forge** ledger rows freely; it merely cannot edit or erase them.
2. **It is owner-disableable, and this repo demonstrates it.**
   `migrations/20260719000010_admin_cognition_firewall_declarative.sql` runs
   `ALTER TABLE kb_events DISABLE TRIGGER kb_events_append_only;`, backfills, and re-enables — its
   own comment conceding *"Disabling it for the duration is the only available path."*
3. **Projection tables carry no trigger by explicit design.** `migrations/20260720000040` draws the
   line the repo actually uses: *"append-only LOGS get a trigger, mutable PROJECTIONS
   (`kb_access_grants`, and `kb_principal_standing` / `kb_principal_governance`) rely on the function
   chokepoint by convention and carry no trigger."* `COMMENT ON TABLE kb_principal_standing` warns
   that *"a direct `UPDATE` bypasses the log and the ledger event"* — a comment, not a constraint.

So the trigger proves **no accidental rewrite in an application path**. It proves nothing against a
deliberate one from the connection string.

`.github/scripts/audit-grant-sinks.sh` freezes the set of grant write-sites. It is a CI tripwire over
source: it proves the set has not grown unreviewed, **not** that any given write is ledgered, and it
sees nothing that happens at runtime.

## "Outside the ledger" is a default, not a principle

The database is **not** uniformly outside the ledger's invariants, and the counter-example is the
house pattern for closing such a gap.

`migrations/20260719000010_admin_cognition_firewall_declarative.sql` moved the admin/cognition
firewall from convention to a `CHECK` + composite foreign key precisely so that direct SQL cannot
evade it. Its own note records why:

> Nothing in the DATABASE forbade an anchored admin event — a direct `INSERT INTO kb_events` could
> still mint one […] **The published docs deliberately said "firewalled by intent" rather than "by
> construction" because of exactly this gap (PR #489).**

Proven unevadable by `the_database_refuses_an_anchored_admin_event`
(`crates/temper-services/tests/admin_ledger_test.rs`), with non-vacuous controls.

That is the only ledger-content invariant the database itself enforces — and it establishes that
each invariant is a **separate question**, not a consequence of where the ring is drawn.

## Operator responsibility

The instance operator must separately confirm, for their deployment:

1. **Who holds the credentials** — `DATABASE_URL`, `DATABASE_URL_UNPOOLED`, and the provider console.
   For the hosted topology that is the Neon project and the Vercel environment variables.
2. **That the provider's own audit log** of console and direct-SQL sessions is enabled and retained.
   This is the control that substitutes for the ledger below the boundary.
3. **Backup and PITR retention** — a ledger the application cannot rewrite is still *droppable* by
   its owner.
4. **Whether the documented direct-SQL steps are performed under change control.**
   `system-bootstrap.sh --run-root`, the `kb_saml_idp` row apply, and `scripts/migrate-cutover.sh`
   are each authority-bearing acts that produce **no ledger row**.

Note that Temper's runtime and its migration path share a credential family. Separating them is an
operator choice; the repo does not do it.

## Revisiting

- The application stops connecting as the owning role, or a restricted runtime role is introduced —
  `REVOKE` and RLS become meaningful and the ring narrows.
- A ledger row becomes an authoritative **input** to a state transition rather than a record of one.
  This has already happened once: `principal_prior_standing` reading `kb_principal_standing_events`
  is what forced `migrations/20260720000040`.
- A compliance regime requires tamper-evidence **of the ledger itself** — at which point the answer
  is hash-chaining or an external WORM sink, not a trigger.
- Multi-tenant hosting where tenants are not co-trusted at the database layer.
