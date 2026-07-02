# agent-workflows

Deployed agent runtimes over the temper-mcp surface. **Eve** is the first runtime
binding; **Claude Managed Agents (CMA)** is a planned second (the two runtimes are
near-isomorphic — see
`docs/research/2026-06-18-vercel-eve-and-claude-managed-agents-investigation.md`).

## Agents

- `steward/` — the team self-cognition steward (Eve). Design:
  `docs/superpowers/specs/2026-07-01-t5-eve-steward-agent-directory-design.md`.

## Why this package is workspace-isolated

Each Eve agent is a **self-contained Eve project** with its own toolchain
(TypeScript 7, `ai` v7, `@vercel/connect`, an npm lockfile). It is deliberately NOT
a member of the root bun `workspaces` array, so it never collides with
`temper-cloud`'s TypeScript 5.8 and the repo pre-commit never touches it. Run all
tooling from inside the agent's directory — and install from inside it too (running
`npm install` from the repo root picks up the root's bun-oriented `overrides` and
fails):

```bash
cd steward && npm install                    # from inside, uses steward/package.json as root
cd steward && npm run typecheck              # tsc
cd steward && npm exec -- eve dev --no-ui    # boot locally (no REPL)
```

## Config

- `TEMPER_MCP_URL` — the temper-mcp target (`https://temperkb.io/mcp` or a
  self-hosted instance URL). Never hardcoded.
- Auth is platform/OAuth-carried. Production uses Vercel Connect
  (`connect()` in `agent/connections/temper.ts`); set `TEMPER_CONNECT_CONNECTOR` to
  the connector UID once `vercel connect create` has registered temper-mcp (T6).
  Until then, a `getToken` dev fallback reads an already-OAuth-obtained token from
  `TEMPER_TOKEN` for local boot and verification.

Deployment (Vercel cron live, envelope audit live) is T6.
