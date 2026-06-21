# WS6 Flip Runbook

This is the operator checklist for the WS6 hard cutover: dumping `public.*` from
production, synthesizing `temper_next` locally, loading it back into prod, and
flipping the backend-selection flag. The procedure is also used for the step-D
rehearsal (see note below).

Design spec: `docs/superpowers/specs/2026-06-21-ws6-flip-dump-synthesize-load-runbook-design.md`

---

## Rehearsal (step D) — run this before the real flip

The same scripts drive both rehearsal and the real flip. For rehearsal, point
both connection strings at a fresh **Neon branch** instead of main, and skip the
freeze, snapshot, cutover, and redeploy steps.

```bash
# Create a throwaway rehearsal branch
neonctl branches create --name ws6-rehearsal-<date>
# Note the connection string for the branch

export FLIP_SOURCE_URL="<rehearsal-branch-conn-string>"
export FLIP_TARGET_URL="<rehearsal-branch-conn-string>"
# FLIP_LOCAL_URL overrides the flip container connection (default:
# postgresql://temper:temper@localhost:5438/temper_development); normally left unset.

# Then run: Pre-flight, Dump public → local, Synthesize locally,
# Load temper_next → prod, and Verify sections.
# Skip: Freeze, Snapshot, Cutover, Unfreeze, and Cleanup.
#
# Before the Verify step, flip the branch's flag once — it's a throwaway branch,
# so this is safe:
#   psql "$FLIP_SOURCE_URL" -c "UPDATE kb_backend_selection SET backend='next' WHERE id=true;"
# This proves the temper_next read path; without it, Verify would still read from public.

# Delete the branch when done
neonctl branches delete <branch-id>
```

Run the rehearsal until it is boring. Then run the identical steps against main
for the real flip.

---

## Pre-flight

1. [ ] Confirm a **green branch rehearsal** of this runbook is on record: synthesis
   §8 body-parity gate reported zero mismatches, §9 read-floor verify returned
   expected rows from `temper_next`.

2. [ ] Confirm `neonctl` is authenticated and version ≥ 2.26:

   ```bash
   neonctl --version
   neonctl projects list
   ```

3. [ ] Confirm PostgreSQL 17 client tools are available. The scripts resolve
   `pg_dump` via `FLIP_PG17_BIN` (default `/opt/homebrew/opt/postgresql@17/bin`,
   the Homebrew `postgresql@17` keg). Either install Homebrew `postgresql@17` or
   set `FLIP_PG17_BIN` to the directory containing `pg_dump` v17. Verify:

   ```bash
   # Using the default Homebrew keg
   /opt/homebrew/opt/postgresql@17/bin/pg_dump --version   # must show 17.x

   # Or with a custom path
   ${FLIP_PG17_BIN:-/opt/homebrew/opt/postgresql@17/bin}/pg_dump --version
   ```

   You do **not** need to put these on `PATH`; the scripts find them via
   `FLIP_PG17_BIN`.

4. [ ] Start the PG17 flip container (port 5438, distinct from the PG18 dev DB
   on :5437):

   ```bash
   cargo make flip-db-up
   ```

   Expected: container starts cleanly, no port conflicts.

5. [ ] Confirm disk headroom for the `public` dump file and local restore (check
   available space on the partition where the project lives — at minimum, twice
   the size of the production `public` schema).

---

## Freeze

6. [ ] **Stop all writes to production.** The operator is the only writer
   (single-user posture). Confirm:
   - No ingest pipeline job is running or scheduled.
   - No cron or autonomous agent is writing to prod.
   - No other browser/CLI session is open against prod.

   There is no code-level freeze mechanism — this is operator discipline.

---

## Snapshot

7. [ ] Create a Neon snapshot branch from main as the rollback target:

   ```bash
   neonctl branches create --name flip-rollback-<date> --parent main
   ```

   **Record the branch name and id.** This is the rollback point. If anything
   goes wrong after the load step, you restore from here.

   Example output to record:
   ```
   Branch: flip-rollback-2026-06-21
   Id:     br-<id>
   ```

---

## Dump public → local

8. [ ] Set the source connection string (production / the main Neon branch):

   ```bash
   export FLIP_SOURCE_URL="<prod-conn-string>"
   ```

9. [ ] Dump `public` from production and restore it into the local PG17 flip
   container:

   ```bash
   cargo make flip-dump-public
   ```

   The task: dumps the `public` schema (schema + data) from `FLIP_SOURCE_URL`,
   then restores into the container on :5438. `flip-db-up` already installed
   the `vector` extension and the portable uuid shim; `flip-dump-public`
   re-asserts the portable `uuid_generate_v7()` shim after the restore because
   `--clean` may drop same-named objects. `pg_uuidv7` is intentionally NOT
   installed locally — the portable shim replaces it.

   > **Note:** the prod `public` dump contains `CREATE EXTENSION pg_uuidv7`,
   > which the local PG17 container cannot satisfy. `flip-dump-public` suppresses
   > that error (`|| true`) and still exits 0. A `pg_uuidv7` extension error line
   > in the output is **expected** — judge success by the final
   > `public restored. rows: N` count, not by the absence of error output.

---

## Synthesize locally

10. [ ] Run synthesis and the §8 body-parity gate against the local PG17
    container:

    ```bash
    cargo make flip-synthesize-local
    ```

    The task: loads `temper_next` schema + functions (the clean artifact — NOT
    the `03_seed` demo fixture) into the local DB, runs `temper-next synthesize`
    with `search_path = temper_next, public` against localhost, then asserts the
    §8 parity gate.

    **The task must exit 0.** A non-zero exit means the parity gate failed.
    Do not proceed to the load step with a failed synthesis. Diagnose, fix,
    and re-run.

---

## Load temper_next → prod

11. [ ] Set the target connection string (production — same as source for the
    real flip):

    ```bash
    export FLIP_TARGET_URL="<prod-conn-string>"
    ```

12. [ ] Load `temper_next` from the local container back into production:

    ```bash
    cargo make flip-load-next
    ```

    The task: dumps `--schema=temper_next` (full: schema + data) from the local
    container, then on `FLIP_TARGET_URL` runs `DROP SCHEMA temper_next CASCADE`
    followed by restore — in one transaction. This replaces prod's dormant
    `temper_next` wholesale with the locally-built one.

    **The flag is still `legacy` at this point** — prod is still serving from
    `public.*`. The load step is safe to re-run if it fails; `public.*` is never
    touched.

---

## Cutover

13. [ ] Flip the backend-selection flag on production:

    ```bash
    psql "$FLIP_TARGET_URL" -c "UPDATE kb_backend_selection SET backend='next' WHERE id=true;"
    ```

    Confirm the `UPDATE 1` acknowledgement.

14. [ ] **Redeploy the Vercel app** (both `api` and `mcp` functions). The flag
    is read **once at API startup** (`crates/temper-api/src/main.rs:34`); a
    running process will not pick up the change without a restart.

    Deploy via the Vercel dashboard or CLI:

    ```bash
    vercel --prod
    ```

    Wait for the deployment to complete before proceeding to verify.

---

## Verify

15. [ ] Confirm that production surfaces are now serving from `temper_next`.
    Run each of these against the production API and confirm the expected rows
    appear:

    ```bash
    temper resource list
    temper resource show <ref>
    temper resource search <query>
    ```

    Also run a graph read (edge list or relationship query) to confirm the
    `temper_next` schema is serving correctly.

    Expected: all commands return data consistent with the pre-flip production
    state (ids preserved — synthesis preserves external ids). No 5xx errors.

---

## Unfreeze

16. [ ] Resume writes. New writes now land in `temper_next` via the NextBackend
    write path.

---

## Cleanup

17. [ ] Stop and remove the local PG17 flip container (including its volume — the
    container holds only transient prod data):

    ```bash
    cargo make flip-db-down
    ```

18. [ ] Retain the rollback snapshot branch from step 7 for a retention window
    (suggested: 30 days) before deleting it.

19. [ ] `public.*` is left **dormant in place**. Renaming it aside and dropping
    it is explicitly out of scope for this runbook — it belongs to the
    migration-endgame spec.

---

## Rollback

If something goes wrong after cutover, rollback is cheap because `public.*` was
never mutated (synthesis only read it, under freeze).

**Flag-flip rollback** (use first — usually sufficient):

```bash
psql "$FLIP_TARGET_URL" -c "UPDATE kb_backend_selection SET backend='legacy' WHERE id=true;"
vercel --prod   # redeploy to pick up the flag change
```

After the redeploy, production is back on the untouched legacy `public.*` schema.

**Snapshot restore** (use if a deeper issue is found):

If the flag-flip rollback is not enough, restore from the Neon branch created in
step 7:

```bash
# Promote or use the snapshot branch as the new main via Neon dashboard or neonctl
neonctl branches get <branch-id>   # confirm branch state
# Follow Neon's branch-restore procedure
```

`temper_next`'s loaded data can be left in place (it is dormant under `legacy`)
or truncated — it does not affect the legacy read path either way.
