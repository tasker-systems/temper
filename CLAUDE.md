# CLAUDE.md

Router for agents working in this repository. [AGENTS.md](AGENTS.md) carries the same table.

| Topic | Read |
|---|---|
| Layout and what is not obvious about it | [internal/agents/architecture.md](internal/agents/architecture.md) |
| Build, test, and the failures that are not yours | [internal/agents/build-and-test.md](internal/agents/build-and-test.md) |
| Invariants that are easy to break | [internal/agents/key-patterns.md](internal/agents/key-patterns.md) |
| Code quality and SQL | [internal/agents/code-quality.md](internal/agents/code-quality.md) |
| Branches, PRs, migration comments, feature-gated tests | [internal/agents/conventions.md](internal/agents/conventions.md) |
| Local and cloud environment | [internal/agents/environment.md](internal/agents/environment.md) |
| The tasker-systems engineering baseline, and what a gate enforces here | [`.claude/skills/tasker-systems-conventions`](.claude/skills/tasker-systems-conventions/SKILL.md) |

- **The baseline** is pinned in `.conventions/consumer.toml`; never edit `.conventions/baseline/`.
  Move the pin with `python3 -I .conventions/baseline/tools/conventions.py bump`.
  *(gated: `quality-gate.yml#conventions`)*
- **Specs, plans, reviews, spikes and handoffs go in
  [`tasker-systems/temper-artifacts`](https://github.com/tasker-systems/temper-artifacts)**, never
  here — not `docs/` (public), not `internal/`. This overrides the superpowers skills'
  `docs/superpowers/` default, and any machine-local temper `guidance/fundamentals.md` that names
  `internal/superpowers/`: that file is hand-kept per machine, and this one wins.
  *(gated: `check-no-process-artifacts.sh`)*
- **Temper itself:** `temper warmup` primes a session; the workflow skill installs with
  `temper skill install --target claude`.
