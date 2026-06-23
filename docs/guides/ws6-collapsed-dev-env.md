# WS6 Collapsed-Schema Dev Environment

How to run local development against the **collapsed** single-schema shape that the
WS6 endgame produces — *before* the live schema rename has happened. This lets the
code plan (`docs/superpowers/plans/2026-06-22-ws6-endgame-collapse-code.md`) be
developed and tested TDD-style against the post-collapse world.

## The idea

Post-collapse there is **one** schema. The live cutover renames the already-live
`temper_next` substrate to the canonical `public` (operator runbook:
`docs/guides/ws6-endgame-collapse-runbook.md`). The collapsed *code* carries no
`temper_next.`-qualified SQL and no per-connection search_path hooks — it issues
plain unqualified SQL against the connection default.

To develop that code locally before the rename, we point the **connection default**
at the substrate (`temper_next`) instead of `public`. De-qualified SQL then resolves
to the substrate exactly as it will resolve to `public` after the rename. The legacy
`public.*` schema stays present in the dev DB but is unreferenced.

## The dev loop

1. Load the substrate artifact into the local Docker DB:

   ```bash
   cargo make db-collapsed
   ```

   This runs `00_namespace_reset` + `01_schema` + `02_functions` (the substrate
   install), the same load preamble `prepare-next` uses.

2. Export the collapsed `DATABASE_URL` — the connection default search_path is the
   substrate:

   ```bash
   export DATABASE_URL="postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dtemper_next,public"
   ```

3. Build/check/test as usual (`cargo make check`, focused `cargo nextest`). Every
   `sqlx::query!` macro and runtime query resolves against the substrate, because it
   is the connection default — no `temper_next.` qualification, no `SET LOCAL
   search_path` needed.

## Why this mirrors the runbook

This is the local stand-in for the post-rename production state:

| Local (this guide)                              | Production (after the runbook)                 |
|-------------------------------------------------|------------------------------------------------|
| substrate lives in `temper_next`                | substrate renamed to `public`                  |
| connection default = `temper_next,public`       | connection default = `public` (bare pool)      |
| de-qualified SQL → resolves to substrate         | de-qualified SQL → resolves to substrate        |
| legacy `public.*` present, unreferenced          | legacy renamed to `public_legacy`, then dropped |

The substrate is the substrate in both cases; only the *name* the connection defaults
to differs, and de-qualified SQL is indifferent to that name. So code that passes
locally against this collapsed env is the code the runbook's redeploy step ships.
