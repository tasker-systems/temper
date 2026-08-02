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
> | The collapsed tail (`reinforced_min`) | **shipped dormant** — the mechanism works and no machine has set a threshold, because none has the months of data one would have to be chosen from |
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
| `reinforced_min` | how many distinct `open_meta.reinforced` dates a memory needs to keep its own line in the index. **No default, and leaving it out is the right answer today** — see [The collapsed tail](#the-collapsed-tail--demotion-never-deletion) |

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
- **It detects nothing about near-duplicates.** The `source_file` skip above is the whole of its
  reconciliation; nothing is compared against what the target context already holds, so two accounts
  of one incident written on two machines both land. Adjudicating overlap is a step to take *before*
  running it — see [Reading for near-duplicates](#reading-for-near-duplicates-is-yours-now).
- **Interactive by default**, where it confirms the whole batch once — count, target context, cohort
  — defaulting to *no*, and says plainly at the prompt that near-duplicates are not detected. A batch
  with nothing in it never asks. It *refuses to write with no terminal attached* unless
  `--unattended` authorizes writing without that confirmation. `--dry-run` is always permitted and
  writes nothing.

### Reading for near-duplicates is yours now

A pre-write full-text search used to surface overlapping memories for you to judge. **It was deleted
on 2026-08-02, and the measurement is why:** against a real 184-memory store it surfaced 54
collisions, **3** of which genuinely overlapped. The other 51 matched on *shared project vocabulary*
rather than on claim — `temper invocation` against `NEVER abbreviate a UUIDv7 to its prefix`,
`cargo make cannot` against `the ts-rs drift gate`. Full-text search cannot see that two texts make
unrelated claims, and this gets **worse in a shared context, not better**, because shared vocabulary
is exactly what a team's context accumulates.

**The rule did not go with the mechanism.** Two accounts of nearly the same thing are still surfaced
for judgment and never merged automatically. What changed is who forms the candidate set: read the
local memories against what the store already holds and **compare the claim, not the wording**. Three
outcome kinds were each observed on 2026-08-02, and a reader who expects only the first will
mis-handle the other two — a **duplicate** (one incident recorded twice, resolved by keeping the
older, richer account), a **supersession** (a newer account strictly richer than the one it covers),
and **both stale** (both out of date, the newer one still the more wrong — recency arbitrated
nothing). The `memories` skill carries the working form of this on both surfaces.

**What this costs, plainly.** A compiled-in gate is unskippable; guidance is skippable by
construction. The trade is an *enforcement* mechanism for a *judgement* mechanism, and it is
defensible only because the enforcement traded away was ~94% noise and was already being switched
off under operational pressure — **not** because guidance is as strong as a gate. It is not.

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

**The preamble is the one place the index explains itself**, and it costs its two lines once rather
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

## The collapsed tail — demotion, never deletion

An index bounded by hand stops being bounded the moment nobody trims it. `open_meta.reinforced` —
the dates a memory did work, written with `--open-meta-add` (see
[the rationale](#where-the-rationale-lives)) — is what lets the index bound itself by evidence
instead. `reinforced_min` is the number of distinct dates a memory needs to keep its own line.

**It has no default, and on every machine in existence it is absent.** That is not a soft default
dressed up as an absence: the key is `Option` with no `#[serde(default)]`, and a test at the
deserializer fails if anyone ever gives it one. A threshold can only honestly be chosen from months
of real reinforcement data, and guessing one now would put a constant with no evidence behind it
into every reader's index.

So the mechanism ships **dormant**, and the first thing to check is that dormant means *nothing*.
Building the pre-change binary and the current one, then running both against this machine's corpus
seconds apart, produced the same file byte for byte `[verified 2026-08-02]`:

```console
$ shasum -a 256 /tmp/AB-old.md /tmp/AB-new.md
b035be83f216989c6b1fe489038517b53ffa316daf806d3ddb85a7dd3128b6e9  /tmp/AB-old.md
b035be83f216989c6b1fe489038517b53ffa316daf806d3ddb85a7dd3128b6e9  /tmp/AB-new.md
```

Both binaries were built *before* either ran, because a second machine was actively culling this
corpus at the time — an A/B with a build in the middle of it measures the cull, not the change.
Worth knowing generally: this file is a projection of a shared store, so two renders minutes apart
are not expected to match, and a diff between them is not evidence about the renderer.

`status` reports the distribution a threshold would eventually be chosen from, which today reads as
"the convention is in use, barely" — exactly the state a number cannot yet be set from:

```console
$ temper memory status --format json | jq '{in_temper, reinforcement}'
{
  "in_temper": 294,
  "reinforcement": {
    "reinforced": 5,
    "never_reinforced": 289,
    "last_reinforced": "2026-08-02",
    "malformed": []
  }
}
```

Set one, and below-threshold memories stop rendering individually. With `reinforced_min = 1` on that
same corpus, 290 rendered entries became 5 plus two tail lines:

```markdown
- … 163 more in @j-cole-taylor/temper, unreinforced — demoted, not dropped: `temper resource list --type memory --context @j-cole-taylor/temper --all`
- … 122 more in @j-cole-taylor/working-agreements, unreinforced — demoted, not dropped: `temper resource list --type memory --context @j-cole-taylor/working-agreements --all`
```

**Those numbers reconcile, and the arithmetic is the point.** `in_temper: 294` is every `memory` row
fetched, superseded ones included. The render drops superseded before the threshold ever applies, so
it works over **290** — leaving 4. And `5 + 163 + 122 = 290`: the five reinforced memories `status`
counted are exactly the five that keep their line.

But `never_reinforced: 289` against `163 + 122 = 285` collapsed is a gap of **4** — those same four
superseded memories. Neither number is wrong; the gap is the documented consequence of the
distribution counting the corpus `in_temper` counts rather than the corpus the index renders. **The
distribution answers "is this convention being used at all", not "what will the tail hide"** — read
as the second, it over-counts by the number of superseded memories, every time.

**Demotion, never deletion.** Nothing is dropped, nothing becomes unfindable, and the file stays
bounded however large the corpus grows. The tail is per section, states its own count, and carries
a route back to the section it hides them in — `--all` included, because a bare `resource list` returns
a capped page and sending a reader there would replace one truncation with another. This is the same
rule the rest of the design already holds: supersession replaces deletion, and falling out of a
summary is never falling out of the record.

Above a threshold of 1 the line states the threshold instead (`reinforced fewer than 2 times`),
because a memory with one date *is* reinforced and still demoted — calling it "unreinforced" would
be a false statement about the memories it describes.

**A malformed `reinforced` is a soft report, not a defect.** `status` names it under
`reinforcement.malformed` and the render carries straight on, treating that memory as unreinforced —
unlike `status`/`verified`, which stop the whole render. The line is what each key holds up:
`status` and `verified` are the fields the render's own claims depend on, `reinforced` only orders
the list. The accepted cost is stated rather than mitigated: one mistyped date silently demotes a
memory into the tail. It is recoverable precisely because demotion is never deletion, and `status`
is where it stops being silent.

> The two `status` captures earlier in this guide predate the `reinforcement` field and are left as
> they ran, on the same policy as the 183/184 count drift noted at the top.

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
- **Nothing detects a near-duplicate any more.** The residue used to be "what the search missed"; it
  is now everything, because there is no search. `migrate` will write a second account of something
  the context already holds and say nothing about it. The store can hold near-duplicates, and only a
  reader looking for them will find them — see
  [Reading for near-duplicates](#reading-for-near-duplicates-is-yours-now) for why that trade was
  taken and what it costs.
- **The replacement is guidance, and guidance is skippable.** An agent told to adjudicate overlap
  before migrating can simply not do it, and nothing fails. That is a real downgrade from the gate it
  replaced, and it is not defended on the grounds that the two are equivalent.
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
