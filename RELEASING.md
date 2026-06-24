# Releasing temper

Production for the `temper-cloud` Vercel project (the repo-root `vercel.json` that
ships the Rust serverless functions — `/api/axum` and `/api/mcp`) deploys on a
**release tag**, not on merge to `main`. Merges to `main` produce only **preview**
deploys; cutting a release is the intentional gesture that ships prod, and it runs
behind a **migration-aware gate** so schema-incompatible code can't auto-ship against
an un-migrated Neon DB.

## One-time prerequisite (operator, manual)

**Disable Vercel's git auto-deploy to production for `temper-cloud`.** Without this,
Vercel still ships prod on every `main` merge and the gate below is bypassed.

In the Vercel dashboard → Project `temper-cloud` → **Settings → Git**, do one of:

- Clear/redirect the **Production Branch** so `main` no longer maps to production, or
- Add an **Ignored Build Step** that skips production builds (e.g. exit 0 when the
  target environment is production), leaving preview builds on PR branches intact.

The goal: `main` merges only ever produce **preview** deploys; production is reached
exclusively through the `deploy-production` job in `.github/workflows/release.yml`.

Also add these repo secrets (Settings → Secrets and variables → Actions):

- `VERCEL_TOKEN` — a Vercel access token with deploy rights to the project.
- `VERCEL_ORG_ID` — the Vercel org/team id (`vercel link` writes it to `.vercel/project.json`).
- `VERCEL_PROJECT_ID` — the `temper-cloud` project id (same `.vercel/project.json`).

## Release checklist

1. **Make changes and merge to `main`.** Per-PR preview deploys validate the change
   before it can reach prod.

2. **If the release includes a schema change, apply the prod migration/cutover FIRST.**
   Prod migrations are operator-run on Neon (boot-time `migrate!` was removed). Order:
   **back up → migrate/cutover → verify.**
   - Additive migration → `sqlx migrate run` against Neon with the canonical
     `search_path`.
   - Big-bang / search-path flip → follow
     [docs/guides/ws6-endgame-collapse-runbook.md](docs/guides/ws6-endgame-collapse-runbook.md).

   Do this **before** the deploy so the new code lands on a ready schema.

3. **Bump `VERSION` on `main`.** `release-tag.yml` derives and pushes the `v<VERSION>`
   tag, which invokes `release.yml`: `determine-version` → `build-cli-binaries` →
   `release-summary` (GitHub Release) and `deploy-production` (ships prod once the
   migration gate clears).

4. **If a migration was applied, clear the gate with `migrations_applied=true`.**
   The gate diffs `migrations/` between this tag and the previous `v*` tag. When
   migrations changed it **halts the deploy** unless the release was triggered with
   `migrations_applied=true` — a pure tag-push can't set that input, so it fails
   closed by design.

   To proceed after applying the prod migration, re-run the release via
   **workflow_dispatch**: Actions → **Release** → *Run workflow* → set `tag` to the
   release tag (e.g. `v0.1.7`) and `migrations_applied` to **true**. (The first job,
   `build-cli-binaries`, re-runs idempotently; `deploy-production` then clears the
   gate and ships.)

   A **code-only** release (no `migrations/` diff vs the previous tag) needs no
   override — `deploy-production` runs straight through on the tag push.

   > First release (no previous `v*` tag) fails closed too: with no prior schema to
   > diff against, the inaugural deploy requires `migrations_applied=true` once Neon
   > is provisioned.

## Rollback

`deploy-production` ships an immutable Vercel deployment. To revert prod, re-point the
production alias to the prior good deployment:

```bash
vercel rollback                       # previous production deployment
vercel rollback <deployment-url-or-id>  # a specific one
```

`rollback` is instant (no rebuild). If the bad release also applied a schema change,
roll the schema back per the runbook before/with the alias rollback.
