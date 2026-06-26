# WS6 Collapsed-Schema Dev Environment

How to run local development against the **collapsed** single-schema shape that the
WS6 endgame produces — *before* the live schema rename has happened. This lets the
code plan (`docs/superpowers/plans/2026-06-22-ws6-endgame-collapse-code.md`) be
developed and tested TDD-style against the post-collapse world.

## The idea

Post-collapse there is **one** schema (`public`). The live cutover renamed the
old substrate schema to `public` (executed; procedure in git history). The collapsed
*code* carries no schema-qualified SQL and no per-connection search_path hooks — it
issues plain unqualified SQL against the connection default.

This guide describes the pre-rename dev loop used during WS6 development. The rename
is executed; the current dev env uses `public` directly (the standard `DATABASE_URL`).
The legacy `public.*` schema was retired with the cutover.

## The dev loop

1. Load the substrate artifact into the local Docker DB:

   ```bash
   cargo make db-collapsed
   ```

   This runs `00_namespace_reset` + `01_schema` + `02_functions` (the substrate
   install), the same load preamble used before the rename.

2. Export the collapsed `DATABASE_URL` — the connection default search_path pointed
   at the substrate (pre-rename):

   ```bash
   export DATABASE_URL="postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dpublic"
   ```

3. Build/check/test as usual (`cargo make check`, focused `cargo nextest`). Every
   `sqlx::query!` macro and runtime query resolves against the substrate, because it
   is the connection default — no schema qualification, no `SET LOCAL search_path`
   needed.

## Why this mirrors the runbook

This described the local stand-in for the post-rename production state. The rename is
now executed — the substrate lives in `public` everywhere (local, CI, production).
The current standard `DATABASE_URL` (`postgresql://temper:temper@localhost:5437/temper_development`)
is the dev default; no search_path option is needed.

De-qualified SQL is indifferent to the schema name, so code that passed against the
pre-rename collapsed env ships identically post-rename.
