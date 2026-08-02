# Memories

> **Memories are resources.** Durable working knowledge is stored as resources of type `memory`,
> carrying `open_meta.status` (`active` / `superseded`) and `open_meta.verified` — the date the
> claim was last checked. A `verified` date far in the past means **nobody has re-checked it**, not
> that it is false.

This machine's `MEMORY.md` is a **rendered projection** of those resources, written by
`temper memory emit`. Do not hand-edit it: the record is the store, the file is a cache.

## Start with `status` — it works before you have adopted anything

```bash
temper memory status
```

Reports what this machine carries whether or not a `[memory]` section exists in `config.toml`:
whether it has opted in, which contexts it reads, how many memories are in Temper, how many local
memory files sit beside the index, and which of those have **no counterpart** in the store.
Declining is a supported end state — a machine can measure its own divergence and never adopt.

## Populating the store — `harvest`, then `migrate`

A machine still holding local memory files has not populated anything yet. Two commands do that,
and **the order is load-bearing**.

**`temper memory harvest` runs first.** A memory's human-readable title exists in exactly one
place: the link text in the hand-written `MEMORY.md`. `harvest` copies each curated title into the
file it names. Skip it, and letting `emit` take the index over destroys those titles — and with
them `migrate`'s ability to move the remaining files at all, since a file no link names is skipped
(*"no MEMORY.md link names it, so it has no curated title"*) rather than given a title invented
from its filename. It is idempotent: a file already carrying a `title:` is skipped. Each stamp also
pins `metadata.modified` to the file's pre-write mtime, so the write's own mtime bump cannot
re-date a claim nobody re-checked.

**`temper memory migrate` moves the files in, and it reconciles rather than bulk-imports.** The
reconciliation is exactly one thing: a file already in Temper is matched on `open_meta.source_file`
and skipped. **It detects nothing about near-duplicates** — it searches nothing and compares
nothing, so two accounts of one incident written on two machines both land. Three consequences a
reader who expects a bulk import will trip over:

- It is **interactive by default** — one confirmation for the whole batch (count, target context,
  cohort), defaulting to *no*, and it states plainly there that near-duplicates are not detected.
  A batch with nothing in it never asks. With no terminal attached it **refuses to write** unless
  `--unattended` authorizes writing without that confirmation. `--unattended` skips no check;
  there is no check to skip.
- `--dry-run` is always permitted and writes nothing. Run it first.
- Re-running is safe. The `source_file` match is what makes a run interrupted halfway resume rather
  than duplicate.

### Before migrating, read for near-duplicates yourself

A pre-write full-text search used to do this, and it was deleted because it did not work: against a
real 184-memory store on 2026-08-02 it surfaced **54 collisions, 3 of them genuinely overlapping**
— 51 false positives, ~94% noise. The false positives matched on *shared project vocabulary*, not
on claim: `temper invocation` was surfaced against `NEVER abbreviate a UUIDv7 to its prefix`,
`cargo make cannot` against `the ts-rs drift gate`. Both mention temper, or cargo, or a CI job.
Full-text search cannot see that they make unrelated claims, and this gets **worse in a shared
context, not better** — shared vocabulary is exactly what a team's context accumulates.

**The rule did not go with the mechanism.** Two accounts of nearly the same thing are surfaced for
judgment, never merged automatically. What changed is who forms the candidate set and on what
basis: read the local memories against what the store already holds, and
**compare the claim, not the wording**. Two memories about one subsystem are not an overlap; two
memories asserting the same thing are, however differently they are phrased. Bring a short list a
human can actually adjudicate.

**Three outcome kinds, each observed on 2026-08-02.** A reader who expects only the first will
mis-handle the other two:

- **Duplicate** — the same incident documented twice, in that case on two different machines.
  Resolved by keeping the **older, richer** account: a judgement no confidence score encodes.
- **Supersession** — a three-weeks-newer account, strictly richer than the one it covers.
- **Both stale** — both accounts out of date, and the newer one still wrong. The real instance was
  two memories about what triggers an `sqlx::migrate!` rebuild; the more recent one was the more
  wrong. **Recency arbitrated nothing.**

**Say what this costs: a compiled-in gate is unskippable, and this is guidance, which is skippable
by construction.** An agent can simply not do it. The trade is an *enforcement* mechanism for a
*judgement* mechanism, and it is defensible only because the enforcement traded away was 94% noise
and was already being switched off under pressure — **not** because guidance is as strong as a
gate. It is not.

Two repairs that look obvious are **rejected**, not deferred: tuning the score, or swapping
full-text for embedding cosine — several of those 51 false positives are genuinely *about* the
same subsystem and would score **higher** under embeddings; and merging automatically above a
confidence threshold, which is the one thing the rule forbids.

One cohort per run, keyed on the files' own frontmatter `type`:

```bash
temper memory migrate --dry-run                     # cohort `feedback` — the default
temper memory migrate --cohort project --dry-run
temper memory migrate --cohort reference --dry-run
```

`feedback` is the cross-project cohort and lands in the first `shared_contexts` entry; every other
cohort lands in the first `project_contexts` entry, beside the resources it discusses. Override
with `--context <ref>`. A cohort whose list is empty is a refusal, never a silent fallback to the
other list — **reach is a property of which config list names the context**, and a memory never
carries one of its own.

Every skipped file is reported with its reason. A file that vanished silently from a migration
would be indistinguishable from one that migrated.

## Recording that a memory was load-bearing

```bash
temper resource update <ref> --open-meta-add '{"reinforced":["2026-08-02"]}'
```

`open_meta.reinforced` is a list of bare ISO dates: the days this memory did work. **Two things
count as a day's work, and the second is the one that matters.** The situation the memory describes
*recurred* — the trap it names actually fired. Or the memory *caught you*, and a mistake did not get
made because of it.

Counting only recurrence would be broken in a way worth understanding: **a memory that works
prevents its own situation from recurring.** It would go unreinforced, decay out of the index, and
then the situation would recur — a loop that oscillates, each swing costing whatever the memory was
preventing. Counting the catch dissolves that.

**Use `--open-meta-add`, never `--open-meta`.** They differ in exactly the way that matters here:
`--open-meta '{"reinforced":["2026-08-02"]}'` **replaces** the list, discarding every date already
stored, silently, with a success response. `--open-meta-add` unions server-side over what is there.

The date is bare, and the contract is bare deliberately. **No `by` field** — `resource update`
already emits a `property_set` event carrying the acting principal, so the trail answers *who*, and
a self-asserted copy in a tier nothing validates could only ever disagree with the record that is
already right. **No note field** — a free-text note makes every record structurally unique, which
stops the union from collapsing same-day duplicates and loses the one-a-day grain that comes free
from the stored shape.

**The harder half: if the catch revealed a shape the memory's body does not describe, amend the
body — that is the act.** A metadata note *about* the gap is the memory-system version of shipping a
"for now" comment: a breadcrumb standing in for the fix. Reinforce the date *and* rewrite what the
memory says.

Amending a memory's body is **not** re-checking its claim, so it does not advance `verified`. Those
are separate judgments and only one of them is about whether the claim still holds.

### What the index does with it — and what it does today, which is nothing

`reinforced_min` in `[memory]` is the number of distinct dates a memory needs to keep its own line
in the index. **It has no default and is absent everywhere**, so today every memory renders
individually exactly as it always has. A threshold can only be chosen from months of real
reinforcement data; until then, reinforcing a memory changes what `temper memory status` reports and
nothing about the file.

Once one is set, below-threshold memories collapse into a per-section tail line:

```
- … 47 more in @me/temper, unreinforced — demoted, not dropped: `temper resource list --type memory --context @me/temper --all`
```

**Demotion, never deletion.** A collapsed memory is still in the record, still addressable, still
searchable — it has stopped occupying a line in a file that is loaded every session. If you are
looking for a memory you remember and cannot see, the tail line is where it went, and the command
on it lists that section's memories, collapsed ones included.

A malformed `reinforced` is a **soft** report: `temper memory status` names it under
`reinforcement.malformed` and the index renders straight through, treating that memory as
unreinforced. This is deliberately unlike `status`/`verified`, which fail the whole index — those
hold up claims the render makes, `reinforced` only orders the list. The accepted cost is that one
mistyped date demotes a memory into the tail without announcing itself in the index; `status` is
where it announces itself, and demotion is recoverable because nothing was deleted.

## Rendering and gating the index

```bash
temper memory emit          # render the index from Temper and write it
temper memory check         # exits non-zero when the on-disk index has drifted
```

`emit` **refuses to overwrite a file it did not write** — an existing hand-written `MEMORY.md`
survives until someone deliberately moves it aside — and it refuses the whole index rather than
render a memory whose `status` or `verified` is malformed. That refusal is the takeover gate, so
take it only once `harvest` has run. `reinforced` is deliberately **not** in that set — see above.

The index lives outside any repo, so nothing in git can diff it; `check` is what a person or a hook
runs instead, gating on the exit code.
