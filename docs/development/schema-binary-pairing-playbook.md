# Schema/binary pairing — what to do when something goes wrong

**Audience:** whoever is looking at a red check, or at a production write path that has stopped
working. Start at the section that matches what you are seeing.

The mechanisms this describes are the migration classification (`declare_migration`), the
declaration check (`audit-migration-declarations.sh`), the wire diff (`sqlx-wire-diff.sh`) and the
cross-check (`sqlx-schema-crosscheck.sh`). Design:
[`docs/superpowers/specs/2026-07-30-schema-binary-pairing-design.md`](../superpowers/specs/2026-07-30-schema-binary-pairing-design.md).
The rule they enforce is in [`DEPLOYING.md`](../../DEPLOYING.md).

> **The one symptom worth memorising.** A pairing break looks like: **writes fail totally, reads stay
> perfectly healthy, nothing is slow, and every check is green.** Nothing degrades. If someone
> reports "creating things is broken but browsing is fine", go to
> [§4](#4-a-live-pairing-break-in-production) first and read the rest afterwards.

---

## 1. CI says a migration declares nothing

```
audit-migration-declarations: FAILED
  20260801000001_add_thing.sql: declares nothing. Silence is not a classification —
  add a declare_migration call naming this version.
```

**What it means.** Exactly what it says. Silence is not a classification, and an absent statement is
as loud as a wrong one by design.

**What to do.** Decide which of the two classes the migration is, then add the call. The decision
procedure is in [`DEPLOYING.md` § *Every migration declares which of the two it
is*](../../DEPLOYING.md#every-migration-declares-which-of-the-two-it-is), and it is one question:

> **Does a binary that predates this migration keep working against the schema after it is applied?**

Yes → `additive`. No, or you are not sure → `shape-breaking`. Being unsure is itself the answer:
`additive` is a *definition*, not a default.

```sql
SELECT declare_migration(
    20260801000001,
    'additive',
    'One new table. Nothing pre-existing is altered or dropped, and no binary reads it yet.'
);
```

**Other messages from the same check, and what each means:**

| message | cause |
|---|---|
| `declares class 'destructive', which is not one of: additive shape-breaking` | There is no third class. Dropping something **is** shape-breaking. |
| `declares a class with an empty reason` | The reason is required. A class token alone records a verdict without its argument. |
| `declared, but no migration file has that version` | A copy-pasted or mistyped version argument. |

The last one usually arrives **paired** with a "declares nothing" for your own migration — that is
the copy-paste being caught from both ends, and fixing the version argument clears both.

---

## 2. CI says the wire contract moved and nothing declares shape-breaking

```
sqlx-schema-crosscheck: FAILED — the wire contract moved and no migration in this change
declares itself shape-breaking.
```

**What it means.** The `.sqlx` caches say a query whose **text did not change** now gets something
different back from the schema. The only thing that can cause that is the schema moving underneath
it. Something in this change altered what the database returns to the deployed binary.

**Read the entries it printed first.** They name the query and the exact transition:

```
  .sqlx/query-177087d2….json
      query: SELECT facet_set($1,$2,$3,$4,$5)
      column 0 'facet_set': Uuid -> UuidArray
```

**Then one of two things is true.**

**(a) The classification is wrong — the common case, and the one that caused the outage.** A
function's return type or argument list changed. That reads like an ordinary edit because every
caller in the repo was updated in the same commit — but **the running binary decodes by type, and it
is a caller you did not update.** Change the declaration to `shape-breaking` and follow
[§5](#5-shipping-a-shape-breaking-migration) for how it deploys.

Note the tell, because it is the opposite of what intuition suggests: Postgres refuses to change a
function's return type via `CREATE OR REPLACE`, so a return-type change **must** be written as
`DROP FUNCTION` + `CREATE FUNCTION`. **The tell is the presence of a `DROP FUNCTION`, not its
absence.**

**(b) The cache was regenerated against a database the repo does not describe.** Your dev database
has migrations that `migrations/` does not, so `cargo sqlx prepare` recorded a schema nobody else
has. Check with:

```bash
psql "$DATABASE_URL" -c \
  "SELECT version FROM _sqlx_migrations
    WHERE version NOT IN (SELECT version FROM migration_current) ORDER BY version;"
```

Any row here is a migration your database has applied that no migration file declares — usually a
branch you switched away from. Reset the dev database, re-run `cargo make db-migrate`, regenerate
the caches, and commit the result.

---

## 3. CI says the wire moved but there is no migration at all

```
sqlx-schema-crosscheck: FAILED — the wire contract moved, and there is
no migration in this change to explain it.
```

Same two causes as §2(b) — a cache regenerated against a foreign database, or a migration applied
out of band — but with nothing in the change that could account for it. Start with the
`_sqlx_migrations` query above; it resolves this the large majority of the time.

This row is **not** in the design spec's table. It was added because a silent pass here would be a
hole in exactly the drift the whole mechanism exists to catch.

---

## 4. A live pairing break in production

**Recognising it.** Writes fail totally, reads are unaffected, and the error carries a decode
mismatch:

```
error occurred while decoding column 0: mismatched types;
Rust type `core::option::Option<uuid::Uuid>` (as SQL type `UUID`)
is not compatible with SQL type `UUID[]`
```

Reads staying healthy is what makes this look narrower than it is. On 2026-07-30 the affected
surface was `resource create` (every frontmatter property is a `PropertyAssert` through
`facet_set`), `resource update` with any metadata change, and every facet write — about 40 minutes,
first noticed by a user hitting a write.

**Confirm what is actually running, do not infer it.** A merge is not a deploy.

```bash
curl -s "https://temperkb.io/api/health?cb=$(date +%s)" | jq -r .commit
git rev-parse origin/main
```

If those differ, the running binary is not `main`, which is half of how the outage happened.

**Then pick a direction, before applying anything.** There is no schema state that satisfies both
binaries at once, so this is not a problem a cleverer migration solves. The two directions conflict,
so choosing after you start is how you get both.

| direction | what it means |
|---|---|
| **Forward** | Deploy the paired binary. Correct when the binary is built and the migration is the newer, intended state. |
| **Back** | Revert the signature **and** the code together. Correct when the binary cannot be deployed promptly. |

**Reverting the migration alone does not work** and reverting the code alone does not either — the
pairing is the unit.

**Afterwards**, if the classification that let it through was wrong, correct it by appending —
[§6](#6-the-classification-was-wrong-and-it-already-shipped).

---

## 4b. `migrate` refuses to run: "migration N is partially applied"

```
Error: migration 29999999999998 is partially applied; fix and remove row from
`_sqlx_migrations` table
```

**What it means.** A previous run wrote `pending` for that migration and never wrote a terminal
entry — the runner died between starting the apply and recording its outcome. This is the crash
case, and the refusal is deliberate: a second attempt on top of half-applied state is how a bad
situation becomes an unrecoverable one.

**The message is sqlx's, and its advice is wrong for us.** It says to remove a row from
`_sqlx_migrations` — but if the apply never completed there is no row there to remove. What is
actually blocking is an unresolved `pending` in `kb_migration_ledger`. Find it:

```sql
SELECT version, occurred_at, reason FROM kb_migration_ledger l
 WHERE l.state = 'pending'
   AND NOT EXISTS (SELECT 1 FROM kb_migration_ledger t
                    WHERE t.version = l.version
                      AND t.state IN ('success','failed','cancelled') AND t.id > l.id);
```

**Then establish what actually happened before clearing it.** Did the migration apply or not?

```sql
SELECT * FROM _sqlx_migrations WHERE version = <version>;
```

- **A row is there** → the body committed and the runner died before recording it. Append
  `success`, saying so.
- **No row** → the body rolled back, or it was a `-- no-transaction` migration that may be
  *genuinely* half-applied. For a transactional migration, append `failed`. For a `no-transaction`
  one, **inspect the schema by hand first** — that is the one case where the database can be in a
  state no ledger entry describes.

```sql
SELECT record_migration_state(<version>, 'failed',
    'Runner died mid-apply on <date>; _sqlx_migrations has no row, so the body rolled back.',
    'operator');
```

Use `cancelled` instead when the apply was abandoned deliberately, so a decision stays
distinguishable from a fault. Either way the next `migrate` proceeds — the ledger is append-only, so
resolving is a new entry and the `pending` stays as the record that it happened.

---

## 4c. The deploy failed: "a shape-breaking migration is pending"

The build applies additive migrations and refuses the rest. This message means the pending set
reached one it will not take, so **nothing after it was applied and the binary did not deploy** —
which is the safe state, not a broken one. Schema and binary are both at N-1, together.

Find which one, and why it is being refused:

```sql
SELECT version, class, class_reason FROM migration_current WHERE class = 'shape-breaking'
  AND version > (SELECT max(version) FROM _sqlx_migrations) ORDER BY version LIMIT 1;
```

Then take it as a cutover (§5 below) and redeploy. The next run finds it already applied in
`_sqlx_migrations` and continues past it.

**Three things worth knowing before you reach for a workaround:**

- **Unrelated deploys are blocked too**, including hotfixes. That is the accepted cost of the
  decision `[decided — 2026-07-31, Pete]`, not an oversight: the alternative is deploying a binary
  that expects schema it does not have, which is the 2026-07-30 outage inverted. If the migration
  should not have merged, **revert it** — that is the fast path, not disabling the gate.
- **There is no override flag, on purpose.** The operator gate *is* applying the migration. A switch
  that let the build take a shape-breaking migration would delete the only thing this mechanism does.
- **Exit 3 is a refusal; any other non-zero is a failure.** If the log says *"the migration runner
  exited 1"*, a migration genuinely broke — read `migration_current` for a `failed` state and go to
  §4b, not to §5.

A halt is also reachable from a migration that declares **nothing**, or a class token outside the
vocabulary. Both halt for the same reason: silence is not safety, and the router will not guess.
CI's declaration check should have caught either before merge — a halt for one of those means
something reached `main` without it, which is worth understanding before simply adding the
declaration.

---

## 5. Shipping a shape-breaking migration

A `shape-breaking` migration is **never** a silent `main` auto-deploy. It is an operator-gated
cutover, per [`DEPLOYING.md` § *Applying schema changes per target*](../../DEPLOYING.md#applying-schema-changes-per-target),
run against each target independently. **The build enforces this rather than trusting it** — a
deploy carrying an unapplied shape-breaking migration fails (§4c) until an operator has taken it:

```
back up (durable Neon snapshot)  →  migrate  →  deploy the binary  →  verify
```

The migration and its binary **land together**. Say so in the PR body so whoever merges knows a
coupled deploy is required, and confirm the running commit afterwards rather than inferring it from
the merge.

**One target is not another.** temperkb.io and any enterprise self-hosted install have independent
databases and independent cadences. Cutting one over says nothing about the others.

---

## 6. The classification was wrong and it already shipped

A shipped migration cannot be edited — sqlx checksum-verifies at
`sqlx-core/src/migrate/migrator.rs:175` and refuses to run. So the wrong claim cannot be fixed where
it was written. Correct it by **appending**, from a new migration:

```sql
SELECT reclassify_migration(
    20260730000010,
    'shape-breaking',
    'Revised: facet_set went from RETURNS uuid to RETURNS uuid[]. Originally declared additive on the reasoning that CREATE OR REPLACE drops nothing and every in-repo caller was updated in the same commit — which is true, and misses that the running binary decodes by type.'
);
```

`migration_current` immediately reads the new class, and **the original claim stays in
`kb_migration_ledger` beside it**, with `recorded_by = 'reclassification'` marking which is which.
That is the point of the ledger being append-only: what the authors believed at the time is
evidence, and an `UPDATE` would destroy it.

**Write the reason as a revision, not a replacement.** Say what was believed before and what
changed — the entry beside it carries the old claim but not the story of why it was wrong.

**Prefer, in order:**

1. **Correct the class in the same PR, before merge.** The wire diff and cross-check exist to make
   this the normal case, and it costs nothing.
2. **If it has shipped:** a new migration with `reclassify_migration`. It declares itself as usual
   *and* revises the older one — CI requires the first and validates the second.

To see the whole history for a version rather than just where it landed:

```sql
SELECT state, class, recorded_by, occurred_at, reason
  FROM kb_migration_ledger WHERE version = 20260730000010 ORDER BY id;
```

---

## 7. What these checks cannot see

Stated so that a green cross-check is not read as more than it is.

- **A break invisible to the caches.** The wire diff only sees what a **compile-checked** query
  touches. A function only the MCP surface calls, a column read through a runtime `sqlx::query`, an
  operator-run cutover that no Rust query names — none of them move the cache. This is precisely why
  a `shape-breaking` declaration with an empty wire diff **passes, noted**: failing an honest
  over-declaration would train people to declare `additive` and hope, which is the behaviour that
  produced the outage. **A `PASS, NOTED` needs no action.**
- **Nullability.** A column becoming nullable is a real decode risk (`T` versus `Option<T>`), and the
  wire diff reports it — but it does not trigger the verdict, because sqlx infers nullability
  heuristically and a noisy signal folded into a verdict teaches people to ignore the verdict.
  **If you see a `NOTED … nullable[]` line, read it yourself.**
- **A migration applied by something other than the runner.** `cargo make db-migrate`, CI, and the
  deploy's own build (`scripts/vercel-build.sh`) all use the `temper-migrate` binary, which brackets
  each apply. A migration applied by `sqlx migrate run`, by `psql`, or by hand gets no state entry at
  all — `migration_current.state` is NULL. That is "not observed", never "did not happen".

  Every migration applied to production before 2026-07-31 reads that way and always will: the
  retroactive backfill in `20260731000020` has already fired and its `NOT EXISTS` guard is now
  permanently satisfied, so nothing will ever fill those in. **A shape-breaking cutover, which is
  hand-run by design, leaves a NULL state for the same reason** — the runner refuses it rather than
  applying it, so it has nothing to observe. Read a NULL beside a `shape-breaking` class as "an
  operator took this", not as a gap.
- **`tests/e2e/.sqlx`.** Deliberately out of scope; the wire contract that breaks a deploy is the
  running binary's. Announced on every run so the omission is never silent.
- **Whether the claim is *true*.** The cross-check tests a claim against the compiler's record. It
  cannot test it against reality — a migration can be honestly declared, pass every check, and still
  be misjudged. The classification is a human judgment with a tripwire under it, not a proof.
- **Rate.** Nothing here closes over merge and deploy cadence, and two merges 47 seconds apart is
  what turned a routine migrate→deploy window into an outage.

---

## Quick reference

```bash
# Is every migration declared?
.github/scripts/audit-migration-declarations.sh

# What does each one claim?
.github/scripts/audit-migration-declarations.sh --list

# Did this change move the wire contract?           (0 = no, 10 = yes, 2 = could not tell)
.github/scripts/sqlx-wire-diff.sh

# Does the claim survive the compiler's record?
.github/scripts/sqlx-schema-crosscheck.sh

# Pin the base explicitly (any of the three above)
WIRE_DIFF_BASE=<rev> .github/scripts/sqlx-wire-diff.sh

# What is actually running in production?
curl -s "https://temperkb.io/api/health?cb=$(date +%s)" | jq -r .commit

# What the DEPLOY will do to the schema — the same call vercel-build.sh makes.
# Exit 0 = applied (or nothing pending); exit 3 = halted, an operator must take it.
cargo run -p temper-migrate --bin temper-migrate -- --additive-only

# Apply EVERYTHING, shape-breaking included. What a developer and an operator run.
cargo make db-migrate

# Where does each migration stand?
psql "$DATABASE_URL" -c "SELECT version, state, class FROM migration_current ORDER BY version DESC LIMIT 10"
```

An exit of **2** from the wire diff means *the base could not be read*, never *nothing moved*. The
cross-check turns that into a failure whenever the change carries a migration — which is exactly
when it matters.
