# Multi-target deployment model — decoupling release from deploy

**Date:** 2026-06-25
**Status:** design, approved (approach A)
**Supersedes:** the `deploy-production`-in-`release.yml` approach on `jct/cd-release-deploy-gating` (commit `1dea8637`)

## Problem

The `jct/cd-release-deploy-gating` branch bolted a `deploy-production` job onto
`.github/workflows/release.yml`, so cutting a `v*` release tag **deployed the
temperkb.io Vercel project**. That couples two things that live at different layers:

- **Releasing** (merge to `main`, bump `VERSION`, cut a `v*` tag) is an
  **OSS-commitment-level**, target-agnostic act. It produces the versioned source +
  CLI binaries + GitHub Release that *the world and every operator* consume.
- **Deploying temperkb.io** is just **one consumer** of a release.

The coupling assumes the repo's CI maps 1:1 to a single Vercel deployment. It does
not. temperkb.io is one target; an **enterprise self-hosted target is about to go
live**, and it runs its **own** Vercel project (own Vercel org/project, own Neon DB,
own Auth0 tenant/env, own deploy cadence). A release must not know any single target's
identity.

## Decision

**Decouple the two layers.** Releasing stays OSS-pure; deployment is per-target and
target-owned.

- **Layer 1 — Release (OSS, target-agnostic).** `release.yml` returns to:
  `determine-version` → `build-cli-binaries` → `release-summary` (GitHub Release).
  No deploy job, no Vercel secrets, no migration gate. The artifact model needs
  **nothing new** — both targets are Vercel projects consuming the same source.

- **Layer 2 — Deploy (per Vercel project).** Each target is an independent Vercel
  project pointed at this repo, configured entirely on the Vercel side: its own
  org/project, `DATABASE_URL` (its own Neon), Auth0 env, and its own deploy trigger
  and cadence. The repo's CI does **zero** deploying.
  - **temperkb.io** — Vercel git auto-deploy from `main` (the status quo, live and
    healthy now).
  - **enterprise self-hosted** — the operator connects their own Vercel project to
    the repo (or their fork/pin) with their own env + Neon, and deploys on their
    own schedule.

### Where the migration-safety lesson goes

The CD branch's migration gate **generalized a once-ever emergency into a permanent
tax on every release.** The WS6 big-bang collapse (search-path flip) is operator-run
via the runbook and happens essentially never again. The **steady state is additive
migrations**, which are backward-compatible by construction — new code runs against
the old schema and old code against the new — so auto-deploying an additive change is
**safe**.

So the safety relocates from a CI gate to a **stated invariant**, documented in
`DEPLOYING.md`:

> **Additive-only on `main`.** Changes that merge to `main` carry only
> backward-compatible (additive) migrations, so any target auto-deploying `main`
> stays safe. A non-additive / big-bang schema change is **not** an ordinary merge —
> it goes through the cutover runbook, operator-gated, against each target's DB,
> coordinated with that target's deploy. It is never a silent `main` auto-deploy.

Each target owns the order **back up → migrate → verify → deploy** against its own
DB. That ownership is the target's, not the release's.

## Changes

1. **`.github/workflows/release.yml`** — revert the `1dea8637` additions: remove the
   `deploy-production` job and the `migrations_applied` workflow_dispatch/workflow_call
   inputs. Back to OSS-pure.

2. **`RELEASING.md`** — rewrite as an OSS release doc only: how cutting a `v*` tag
   produces CLI binaries + a GitHub Release. Drop every Vercel-deploy, migration-gate,
   `migrations_applied`-override, and `VERCEL_*`-secret instruction. Add a one-line
   pointer to `DEPLOYING.md` for "how a release reaches a running site."

3. **`DEPLOYING.md`** (new) — the per-target deployment model: N independent Vercel
   projects, each owning org/project/Neon/env/cadence; temperkb.io = git auto-deploy
   from `main`; enterprise = their own Vercel project; the **additive-only-on-`main`**
   invariant; big-bang cutovers → `docs/guides/ws6-endgame-collapse-runbook.md`.

4. **`CLAUDE.md`** — one-line deployment pointer (release ≠ deploy; targets are
   independent Vercel projects; see `DEPLOYING.md`) so future sessions don't
   re-derive or re-couple it.

5. **Keep** `f4206d9f` (the runbook search-path-flip rework) unchanged.

## Non-goals

- No reusable `deploy-vercel.yml` / CI-owned deploy (approach B) — not needed when
  Vercel's own git integration handles per-project deploys.
- No `vercel.json` ignored-build-step migration gate (approach C) — the additive
  invariant makes auto-deploy safe without a clever script.
- No container/server-binary release artifact — both targets are Vercel projects, so
  source + CLI binaries suffice.

## Risk / rollback

Low. The only behavioral change to live infra is *removing* an unmerged CI job that
never ran in production. temperkb.io continues exactly as today (git auto-deploy from
`main`). The branch's net effect becomes: strip the deploy job, reshape two docs, add
one. The runbook doc commit is preserved.
