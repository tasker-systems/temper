# Cogmap genesis owns its bootstrap trust in exactly one place

**Date:** 2026-08-26
**Status:** Decided — accepted residue, recorded
**Scope:** `BornSubject`, `GrantWarrant::Birth`, and the creator seed in `create_cognitive_map`
**Task:** `01a035f2-d37a-7a83-9f6c-b93d58eb5847`

## Decision

Cognitive-map genesis mints its creator's `read + write + grant` under a warrant that performs
**no authority check**, because at genesis there is no prior authority over the subject to check.

The genesis exception is not eliminated. It is **confined to one named `pub(crate)` type with
exactly one construction site**, rather than granted as a generic hatch.

## Why

Bootstrap trust is irreducible: the first grant on any object cannot be authorized against an
existing grant on that object. Something has to own that.

The obvious accommodation — `Authorized::at_genesis(..)` — was refused on purpose, and the reasoning
is worth preserving. `Authorized<A>` is generic, so a hatch on it hands **every** domain a bypass in
order to solve **one** domain's problem. `BornSubject` is that bypass confined to its own name.

Confinement buys three things a generic hatch would not: the exception is countable, it is greppable
under a single name, and the constructor's name — `minted_in_this_transaction` — states the
unverified claim out loud at every call site. `GrantWarrant::Birth` re-states it as *"the only arm
backed by no authority check at all."*

## What the type does not do, and what the call site does anyway

`BornSubject` **cannot prove freshness**, and says so in its own doc comment rather than implying it:

> **Honest limit, stated rather than implied: this cannot *prove* freshness.** Nothing stops a
> caller minting one for an id that has existed for a year. What it buys is confinement and
> visibility […] Bootstrapping is hard to model without exceptions; the risk is owned somewhere, and
> this is the smallest blast radius available.

That is true of the type. It is **not** the whole picture at the one live site, and the distinction
matters — a reader who takes the type's limit as the system's limit will read this row as a larger
open risk than the code carries, and may "fix" something already defended.

At `create_cognitive_map` in `crates/temper-services/src/backend/db_backend.rs`, freshness is in
fact established by control flow, three ways:

1. **A non-admin cannot name an id at all.** A caller-supplied `cogmap_id` is honored only for a
   system admin; otherwise the id is server-minted (`uuid::Uuid::now_v7()`), so a non-admin can never
   place a map at a chosen — e.g. reserved — id.
2. **An admin-supplied id that already exists short-circuits before the grant.** The existence
   pre-check returns `created: false` without reaching the `Birth` site.
3. **A genesis race duplicate-keys on the `kb_cogmaps` primary key**, inside a single
   `SERIALIZABLE` transaction, surfacing as `Conflict` at commit.

So: the type cannot prove freshness; the call site currently establishes it by other means, and
**those means are part of what a re-review must check.**

## One absolute phrasing that is an inference, not a constraint

The code comment at the seed claims the freshly-minted id *"cannot already carry a grant"* and that
*"the conflict arm is unreachable."*

Unconditionally true on the server-mint branch. On the **admin-supplied-id** branch it is an
inference: `kb_access_grants.subject_id` is polymorphic with **no foreign key**
(`migrations/20260630000001_access_grants_seam.sql`), and `GrantAuthority::resolve` returns
`SystemAdmin` from `is_system_admin` alone, consulting nothing about subject existence. A system
admin could pre-plant a grant on an unused uuid and then genesis at that id, making the
`ON CONFLICT DO UPDATE` arm reachable and the event carry a `previous`.

**No privilege is gained** — the actor is already a system admin. This is recorded as a precision
issue in the comment, not as a vulnerability, so that the next reader does not inherit an absolute
claim the schema does not back.

## How it is enforced

Three mechanisms, three different scopes:

1. **`pub(crate)` + private field + private `mod authz`** — `BornSubject` is unconstructible outside
   `temper-services`, and unconstructible-by-literal outside `authz/grant.rs` itself. Does **not**
   prevent a second construction site inside the crate.
2. **`born_subject_has_exactly_one_construction_site`** (`crates/temper-services/src/authz/grant.rs`,
   plain unit tier — no feature gate). Current expected count: **1**. Its doc comment carries the
   anti-rebaseline rule explicitly: *"If this fails, the fix is never 'bump the number.'"* It counts
   assertions; it does **not** check them.
3. **`.github/scripts/audit-grant-sinks.sh`** — CI-wired and green. For this decision it pins exactly
   one thing: **`db_backend.rs` has exactly 1 grant write-site.**

**`audit-grant-sinks.sh` does not constrain `BornSubject`.** It freezes the set of
`kb_access_grants` write-sites, and its own header disclaims more: *"It does NOT prove attenuation."*
Citing it as a `BornSubject` guard would repeat the visibility-coupling scar recorded in that
script's own header, where a guard was credited with coverage it did not have. Cite it for the sink
count; cite the unit test for the construction count.

Two named blind spots in the count test, recorded because the value of this row is its honest limit:

- The walker excludes files by **basename** `grant.rs`. A second file with that basename anywhere
  under `src/` would become an invisible region. (Today `find crates/temper-services/src -name
  'grant.rs'` returns exactly one file, so the exclusion is unambiguous.)
- It is a textual `str::matches` scan, so it is blind to a struct literal written inside `grant.rs`.

Neither is exploitable today. Both are the kind of thing the count test's own doc comment says
should be seen rather than inferred.

**Nothing anywhere verifies freshness itself.** The type says so, the test's doc comment says so, and
the audit script's header says so. That is the owned boundary.

## Revisiting

- `born_subject_has_exactly_one_construction_site` fails. The fix is never bumping the number; it is
  reading the new site and satisfying yourself its subject is genuinely minted in the same
  transaction.
- Any of the three freshness defenses in `create_cognitive_map` changes: the non-admin
  id-suppression branch, the existence pre-check, or the `SERIALIZABLE` isolation level.
- A second domain wants a `Birth`-shaped warrant. That is a request to **widen the genesis
  exception**, and it belongs in a diff under review — not in a generic hatch.
- A `BornSubject` is constructed on any subject kind other than `AnchorTable::Cogmaps`.
- `db_backend.rs`'s line in `audit-grant-sinks.sh`'s baseline moves off `1`.

## Relation to the database trust ring

This boundary and the one recorded in
[The database is the outermost trust ring](./2026-08-26-the-database-is-the-outermost-trust-ring.md)
are the same *kind* of thing — irreducible rather than unfixed, owned deliberately — but they are
**not equally bounded**, and flattening them would overstate one and understate the other.

The database ring is unbounded: nothing narrows it further. This one is bounded at **exactly one
call site with a CI-enforced count**. "We own this and cannot shrink it" and "we own this and have
shrunk it to one countable place" are different positions.
