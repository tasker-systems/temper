# Operational Memory

How a person, and then a team, moves the working knowledge an agent session accumulates out of one
machine's directory and into Temper — and how to evaluate the whole thing without adopting any of
it.

**A memory is a resource.** It has doc type `memory`, lives in a context, and is read, written,
corrected and searched with the ordinary resource commands and tools. What it needs beyond a body
lives in its open tier: `status` (`active` / `superseded`) and `verified` (the ISO date the claim
was last checked against the system) — the two the index render reads, and the two it refuses on —
plus `descriptor` (the one-line recall hook) and `source_file` (the local file it was migrated from,
when it was migrated from one). Everything else — reach, history, attribution, search — is what a
resource already does.

**`MEMORY.md` is one client's convenience.** Claude Code loads a rendered index each session, and
`temper memory emit` writes it. The file is a cache; the resources are the record. A client with no
index file and no command line reads the same memories over the cloud and needs none of this guide
past the sharing section.

> **Status, 2026-08-02.** Read this before acting on anything below.
>
> | Part | State |
> |---|---|
> | Contract, commands, the local drift gate | **shipped** (PR #612) |
> | Migration of an existing local corpus | **exercised** — 182 local files accounted for, 183 memories in Temper on one machine |
> | Title harvest before the takeover | **exercised** — 110 titles stamped |
> | The takeover (`emit` over the hand-written index) | **taken, 2026-08-02** — 181 entries, 19,339 bytes, `check` clean |
> | A second machine | **not exercised** |
> | Two people writing into one shared context | **not exercised** — the sharing section below describes an intended mechanism, not an observed one |
> | Reading memories from Desktop / mobile / web | **not exercised** — believed to work by construction |
>
> Every command output in this guide is pasted from a real run on 2026-08-02, not described from
> memory. Those runs were captured non-interactively, so the output is JSON; on a terminal you get
> TOON by default and `--format json` forces what is shown here. Long `skipped` arrays are
> summarized in the prose beneath them and absolute home paths are shortened; nothing else is edited.
> The corpus grew from 183 to 184 partway through, so earlier outputs report the smaller count —
> they are left as they ran rather than reconciled after the fact.

## Nothing happens until you edit a config file

Start here, on any machine, adopted or not:

```console
$ temper memory status
{
  "opted_in": false,
  "contexts": [],
  "in_temper": 0,
  "defects": [],
  "local_files": 0,
  "local_without_counterpart": []
}
```

`opted_in: false` is not an error. It is the primary case this command exists to report, and it is
a supported end state — a machine may measure itself, decide against the whole idea, and never
return.

**What that report cannot tell you yet, and this is a real limit rather than a caveat.** The
`local_files` count is read from the directory containing the configured `index_path`. With no
`[memory]` section there is no `index_path`, so there is no directory to scan, so an unadopted
machine reports `local_files: 0` **even when it is carrying 182 memory files**. The zero above came
from exactly such a machine. Before adoption, `status` tells you that you have not adopted; it does
not measure your divergence.

Measuring divergence therefore costs one config edit. That edit writes nothing — not to disk, not
to Temper — which is what makes it an evaluation step rather than a commitment.

## Adopting: one config section

Add `[memory]` to `~/.config/temper/config.toml`:

```toml
[memory]
# Contexts whose memories reach EVERY project on this machine.
shared_contexts = ["@me/working-agreements"]
# Contexts for this project. A list — a project may legitimately span contexts.
project_contexts = ["@me/temper"]
# The rendered index. `emit` refuses to overwrite a file it did not write, so an
# existing hand-written MEMORY.md is safe until it is deliberately moved aside.
index_path = "~/.claude/projects/<project-dir>/memory/MEMORY.md"
```

| Field | Meaning |
|---|---|
| `shared_contexts` | rendered for every project on this machine |
| `project_contexts` | rendered for this project only. Both lists are read shared-first and deduped, so a context named in both is fetched and rendered once |
| `index_path` | where the rendered index is written, and the directory scanned for local files |
| `stale_after_days` | how old a `verified` date must be before the index marks it `UNEXAMINED`. Defaults to **90**; omit it unless you want a different number |

The section is `Option`, not defaulted, precisely so *"not configured"* and *"configured empty"* stay
distinguishable — and at least one context is required, so an empty `[memory]` block is rejected
rather than silently doing nothing.

With `index_path` set, the same command now measures the machine. On a directory holding one local
memory that has never been migrated:

```console
$ temper memory status
{
  "opted_in": true,
  "contexts": [
    "@me/working-agreements",
    "@me/temper"
  ],
  "in_temper": 183,
  "defects": [],
  "local_files": 1,
  "local_without_counterpart": [
    "feedback_never_run_migrations_on_friday.md"
  ]
}
```

`local_without_counterpart` is the list that matters: local files that no memory in Temper claims
via `open_meta.source_file`. Matching is on that exact filename and nothing else — never on a title,
never on a sluggified form — so a memory authored natively in Temper (from a session, from Desktop)
simply contributes no match and is never itself reported as orphaned.

`defects` is a report, not a failure: a memory with a missing or malformed `status`/`verified` is
named here and the report still succeeds. Only `emit` refuses on one.

## Populating the store: `harvest`, then `migrate`

Two commands, and **the order is load-bearing.**

### `temper memory harvest` runs first

A memory's human-readable title exists in exactly one place: the link text in the hand-written
`MEMORY.md`. Nothing in the memory file carries it. `harvest` copies each curated title into the file
it names, so the title survives the index being taken over later.

Take the takeover first and those titles are gone — and with them `migrate`'s ability to move the
remaining files at all, because a file no link names is skipped rather than given a title invented
from its filename. That is deliberate: measured on a real corpus, filename-derivation lost the hook
on 51 of 110 files.

`harvest` is idempotent, and a dry-run shows you why:

```console
$ temper memory harvest --dry-run
{
  "dry_run": true,
  "scanned": 182,
  "titles_harvested": 178,
  "stamped": [],
  "skipped": [ ... ]
}
```

Nothing to stamp on this machine, because all 182 files are already accounted for: 110 *already
carry a `title:`* (a prior run stamped them) and 72 are *already in Temper (matched by
`source_file`)* — a migrated memory's title lives in the store, which is authoritative, so stamping
the file would be writing to the cache.

Each stamp also pins `metadata.modified` to the file's **pre-write mtime**, so the write's own mtime
bump cannot re-date a claim nobody re-checked. Run it with `--dry-run` first; it takes `--unattended`
to write with no terminal attached. It reaches the network — the already-migrated set comes from
Temper, not from the files.

### `temper memory migrate` reconciles — it does not bulk-import

```console
$ temper memory migrate --dry-run
{
  "cohort": "feedback",
  "context": "@me/working-agreements",
  "dry_run": true,
  "scanned": 182,
  "titles_harvested": 178,
  "proposals": [],
  "skipped": [ ... ]
}
```

On the run above the 182 skips break down as: 69 *already in Temper (matched by `source_file`)*,
107 *not in cohort (type is `project`)*, 6 *not in cohort (type is `reference`)*.

- **Cohorts.** `--cohort` selects by the files' frontmatter `type`, defaulting to `feedback`. The
  target context defaults to the first `shared_contexts` entry for the cross-project cohort and the
  first `project_contexts` entry otherwise; `--context` overrides it. Run it once per cohort.
- **Re-runs are safe.** A file already in Temper is matched on `source_file` and skipped, so an
  interrupted batch is resumed by running it again rather than by reasoning about what got through.
- **It searches before each write** and surfaces near-matches for you to judge. It never resolves an
  overlap itself.
- **Interactive by default**, and it *refuses to write with no terminal attached* unless
  `--unattended` explicitly authorizes a run — which then **skips** every collision rather than
  resolving one. `--dry-run` is always permitted and writes nothing.

## The takeover: `emit` and `check`

`emit` renders the index from Temper and writes it. Against a hand-written file it refuses:

```console
$ temper memory emit
✗ temper: Bad request: refusing to overwrite /Users/.../memory/MEMORY.md — it was not generated
  by `temper memory emit` (it does not start with the generated-file header). This looks like a
  hand-written or otherwise pre-existing file. Move it aside or point `index_path` (or --path)
  somewhere else, then run `temper memory emit` again.
$ echo $?
1
```

The check is one fact `emit` can verify without a second source of truth: a previously-emitted index
starts with the line `<!-- GENERATED by ... — do not edit -->` shown in the render below. A file that
does not is either hand-written or something else entirely, and clobbering it silently is the bug.

**So the takeover is a deliberate act, and it is the step to take last.** Move the hand-written index
aside (keep it — it is the only copy of anything `harvest` did not stamp), then run `emit`. To see
what would be written without touching anything, point it elsewhere first:

```console
$ temper memory emit --path /tmp/MEMORY-preview.md
✓ Memory index written: /tmp/MEMORY-preview.md
```

That render, on a 184-memory corpus, was 193 lines and 19,339 bytes — near enough the 19,759 the
hand-written file it replaced occupied:

```markdown
<!-- GENERATED by `temper memory emit` — do not edit -->

# Memory index

> **Hooks only — a line is a title, not the memory.** Read the resource before acting on one: `temper resource show <id>`, or the `get_resource` MCP tool.
> A `verified` date far in the past means nobody has re-checked the claim — never that it is wrong.

## @j-cole-taylor/temper

- [feature unification pulls in ort](019fc011-e40f-72b2-be4b-598ec2f68c71)  [verified 2026-06-19]
- [sqlx::migrate!() stale cache](019fc010-c86e-7ed0-94e7-b36bf255d36c)  [verified 2026-04-21 — UNEXAMINED 103d]
```

Two things that file says about itself, both deliberate:

**The link target is a bare id**, which is what `temper resource show` accepts — a target copied out
of the index resolves as-is. It is not a `temper://` URI: that form resolves in neither surface a
reader has, since the CLI's ref parser rejects it and MCP's resource URI is `temper://resources/{id}`.

**The preamble is the one place the index explains itself**, and it costs its three lines once rather
than per entry. What a line is (a hook, not the memory), how to resolve one, and what an old date
does *not* mean are exactly the things a session loading this file cold would otherwise have to
infer.

The marker says **UNEXAMINED**, never *stale* or *wrong*. An old date means nobody has re-checked the
claim; it carries no opinion about whether the claim holds.

**A memory whose `status` or `verified` is missing or unparseable stops the whole render** rather
than being rendered with a guessed value. Those are the fields that describe a memory's reliability,
and nothing validates them at write time — so the render is where a bad one has to surface.

`check` is the drift gate. The index lives outside the repo, so nothing in git can diff it:

```console
$ temper memory check --path /tmp/MEMORY-preview.md
✓ Memory index is up to date.
$ echo $?
0
```

Against an index that has diverged it prints a unified diff and exits non-zero — which is exactly
what a machine that has migrated but not yet taken the index over will see, since the hand-written
file and a fresh render are not the same document. Gate a hook or a CI step on the exit code. If you
emitted with `--path`, check with the same `--path`, or you are checking a different file than the
one that was written.

## Sharing memories with a team

> **Intended mechanism, not an observed one.** Nothing has yet exercised two people writing into one
> shared context. What follows is grounded in how the commands are wired, not in a run.

A memory reaches whoever can read the **context it lives in**. There is no per-memory reach field,
and a memory makes no claim about its own audience — `emit` and `status` list `memory`-typed
resources in each configured context through the ordinary resource read path, so a member sees a
memory exactly when they could see any other resource there.

That makes team adoption a context question, answered with the commands that already exist:

1. Put the shared memories in a context the team can read — either a team-owned context
   (`+team-slug/<slug>`) or one you own, shared into the team's read-reach:

   ```bash
   temper context share @handle/working-agreements +my-team
   ```

   Sharing requires that you administer the context **and** manage the target team (owner or
   maintainer), or that you are an instance administrator. Note that `@me` shorthand is not accepted
   here — use your handle or the context UUID.

2. Each member names that context in their own `[memory]` section, under `shared_contexts` if it
   should reach every project on their machine or `project_contexts` if it is scoped to one.

3. Members who work from Desktop, mobile or the web name nothing and configure nothing. They read
   and author the same memories as resources.

Two consequences worth being explicit about. **Reach follows membership and grants, so it changes
when the team changes** — nothing has to be re-tagged when someone joins or leaves. And **a member
who never adopts the CLI is not a second-class participant**: the index is one client's rendering,
not the mechanism.

## Declining

Delete the `[memory]` section. Nothing else changes: no local file is touched, no resource is
removed, and `temper memory status` keeps answering. A machine that evaluated and declined is in a
supported state, not an unfinished one.

## The limits, stated plainly

- **Before adoption, `status` cannot measure divergence** — see the top of this guide. Evaluating
  costs a config edit, which writes nothing but is still an edit.
- **`--collision-limit 0` disables the near-duplicate gate.** The cap is applied *before* the
  empty-check, so a limit of zero turns every verdict into "clear" and every proposal writes unasked.
  The flag reads as a display knob and is documented as one. It has been reached for in practice,
  under operational pressure from an unrelated failure. Treat it as `--yes`, not as `--quiet`.
- **Detection does not eliminate the residue.** Two accounts of the same thing phrased differently
  enough share no lexemes, pass the search, and land twice. That is deliberate — a reconciler that
  merged on similarity would destroy one of two accounts someone may have wanted to compare — but it
  means the store can hold near-duplicates.
- **Nothing validates the open tier at write time.** `status`, `verified`, `descriptor` and
  `source_file` are ordinary open-meta keys. A malformed one is accepted by the write, reported by
  `status`, and refused by `emit`. Nothing catches it at the moment it is written.
- **Some harvested titles will not be hooks, and you only see it after the first `emit`.** A title
  comes from the link text in the hand-written index, and link text that read fine inside its
  sentence — *"Slack — `[topic file]`"*, *"Principal admission (D11) — `[Phase 1 shipped]`"* — is
  useless standing alone in a flat list, because the sentence supplied the subject. Read the emitted
  index once and retitle the offenders with `temper resource update <ref> --title`. Doing so does
  **not** advance `verified`, which is correct: renaming a memory is not re-checking its claim.
- **The lazy tail is never forced.** Beyond the cohort you deliberately migrate, files move when
  someone touches them. Nothing requires the rest to finish, and nothing reports the remainder except
  `status`.
- **The evidence base is one person, one machine, one client.** The corpus is real (184 memories)
  and the takeover has been taken; the multi-writer and multi-environment claims remain unexercised.
- **The index is flat.** The hand-written file it replaces grouped entries under principle headings;
  nothing keys a memory to a grouping, so the render lists by context and nothing else. That
  synthesis, where it exists, has to live in a memory of its own.

## Where the rationale lives

This guide is deliberately not the argument for any of the above. For *what was decided and why* —
the memory contract, why reach is the home context rather than a field, why the index is generated,
and why the migration is a bounded batch plus a lazy tail — read
[the design](../superpowers/specs/2026-08-01-memories-in-temper-design.md). The session-facing
version of the workflow, which is what an agent reads rather than a person, ships in the temper
skill's `memories.md`.
