# Memories live in Temper; MEMORY.md becomes a generated projection

`[decided — 2026-08-01, Pete]` · Design for moving Claude Code's per-project memory files into
Temper resources, leaving `MEMORY.md` as an emitted index rather than an authored one.

## The problem, stated precisely

Claude Code memories today are **182 memory files totalling 599KB**, plus the index, in one machine's
`~/.claude/projects/<project>/memory/`, indexed by a hand-written `MEMORY.md`. Three things are
wrong with that, and only the first is the one usually noticed.

**They do not travel.** Work spans machines, and spans environments — Claude Code, Claude on the
web, desktop, mobile. Every one of those already relies on Temper. The memory directory reaches
exactly one of them. A memory written on one machine is invisible to the same person on another,
so the same lesson is re-learned and re-written, and the two copies then disagree.

**They rot silently, and the rot is invisible from the index.** On 2026-08-01 two entries were
found stale in the same session — `project_cli_list_status_filter_is_a_noop` (fixed; the filter
works, verified by exact partition) and `project_cli_tags_flag_replaces_not_adds` (fixed; the flags
now union via `open_meta_add`). Both had been carried as live traps for days, instructing every
session to distrust working behaviour. **A stale "known bug" costs what a missed one costs** — it
just spends the attention somewhere less visible. Nothing in the file format records when a claim
was last checked, so nothing can flag one that has gone unexamined.

**The index outgrows its own read limit.** `MEMORY.md` is loaded into context every session and is
periodically hand-compacted, which is attention spent on curation rather than on work. The largest
individual memories are 20–23KB — `project_context_regions_goal.md` at 23,221 bytes is a research
document, about a goal that already exists in Temper as a resource.

**Temper is the system built for exactly this work** — versioned bodies through the ledger,
provenance, supersession, semantic search, staleness signals, and cross-environment reach by
construction. The memory directory is a parallel, weaker implementation of it, maintained by hand.

## What was decided

| Decision | Choice | Why |
|---|---|---|
| Index upkeep | **Generated projection + drift gate** | `MEMORY.md` is emitted, never hand-edited. Same pattern as the agent-skills projection shipped in PR #609. A memory's truth lives in one place; the index is a render, so it cannot disagree with what it indexes |
| Home + scope | **Split by reach: shared vs project context lists** | Reach is declared by *where a memory lives* and *which configs name that context*, never by a per-memory field that can be set wrong and that nothing validates |
| Staleness | **`status` + `verified`, both rendered** | Supersession keeps the principle and kills the instance; the verified date makes an unexamined claim visible instead of silent |
| Migration | **Bounded batch + lazy tail** | The cross-project cohort migrates deliberately; the rest moves when touched. Rationale below |

## The memory contract

**Doc type: `memory`.** It already exists in `crates/temper-workflow/schemas/memory.schema.json`,
has **zero** resources, and declares nothing but a required `temper-slug`. So it is defined here
rather than overloading `concept` (66 in `@me/temper`) or `decision` (23).

| Today (file) | In Temper |
|---|---|
| `name:` slug | `temper-slug` — already required by the schema |
| `description:` (the recall-relevance line) | `open_meta.descriptor` — FTS-indexed at weight D; its own schema describes it as keeping "the discriminating words searchable" |
| `type: feedback \| project \| reference` | **the home context** — see *Homes and reach* |
| body: the fact, `**Why:**`, `**How to apply:**` | the body, unchanged |
| `[[wikilinks]]` | `open_meta.relates_to`; a real edge where the relationship is load-bearing |
| *(nothing today)* | `open_meta.status`: `active` \| `superseded` |
| *(nothing today)* | `open_meta.verified`: ISO date the claim was last checked against the system |
| the filename itself | `open_meta.source_file`: **optional** — the memory file this was migrated from |

### `source_file`, and why it is optional

`status` must report *which local files have no counterpart in Temper*. There is no field that can
answer that by inference: `ResourceRow` carries `context_slug` but **no resource-level slug**
(`crates/temper-workflow/src/types/resource.rs:18-78`), and `ManagedMeta` excludes identity keys by
design. Matching a filename stem against a human-readable title fails on every real memory —
`feedback_a_clause_cannot_retire_its_own_goal` is not "a clause cannot retire its own goal" — which
would make the report 100% false positives on exactly the adopted machine it exists to serve.

So a migrated memory **records the file it came from**, and `status` matches on that exactly, with no
sluggify guesswork. It doubles as provenance: "where did this come from" stays answerable.

**It is optional, and that is load-bearing.** A memory authored natively in Temper — from a session,
from Desktop, from anywhere that never had a file — has no source file, and its absence must never be
a defect. `emit` therefore requires `status` and `verified` and **must not require `source_file`**.
A machine's local files that match nothing are reported; a Temper memory matching no local file is
ordinary, not an error.

### The open-tier cost, stated rather than hidden

`status` and `verified` live in `open_meta`, not `managed_meta`. `managed_meta` is a **closed**
vocabulary (`ManagedMeta`, `#[serde(deny_unknown_fields)]`) and its `temper-status` is declared
goal-only, so adding memory keys there means changing the Rust type and its drift tests.

The consequence is real: **nothing validates these two keys at write time.** A typo lands silently —
precisely the "accepts anything, acts on nothing" shape catalogued in T1 column 4 (linked below). It
is mitigated at the other end rather than at the write: **`emit` fails loudly** on any `memory`
resource missing either key, or carrying a value outside its vocabulary. The gate is therefore
load-bearing, not cosmetic.

Promoting both keys into `managed_meta` is the obvious v2 and is **deferred, not rejected**.

## Homes and reach

```
shared_contexts    @me/working-agreements   ← feedback (69). Read by EVERY project's emit.
project_contexts   @me/temper               ← project (107) + reference (6), beside the goals
                   (+ any others this project spans)   they discuss.

emit for project X  =  ⋃ shared_contexts  ∪  ⋃ project_contexts(X)
```

Reach is a property of **which configs list a context**, never of a field on the memory itself. That
is the property worth protecting: a memory cannot assert a reach it does not have, because it does
not carry one. What it does *not* mean is that reach is one-to-one — a project may legitimately span
contexts (`project_contexts` is a list), and two projects may both list the same one. So a memory in
a shared research context is read by every project that names it, which is the intent, not a leak.

Onboarding another project (`learning-maths` and `storyteller` both exist) is a config entry — no
per-memory rework.

**Supersession replaces deletion.** A memory that turns out wrong takes `status: superseded`, keeps
its body — which is where the *principle* lives, distinct from the instance that expired — and drops
out of the index. The ledger keeps the history. This is the shape hand-written on 2026-08-01 for the
two stale CLI memories, which retained their generalizable traps ("a filter that does not filter
makes presence the lie"; "a verification that cannot fail is not a verification") while their
instances were marked fixed.

## Emit and the drift gate

A new CLI command, following `temper skill emit`:

```
temper memory emit          # contexts and index_path come from [memory] config
temper memory emit --path <p>   # override, for a machine mid-adoption
```

Renders one line per **active** memory across the configured contexts, grouped by context, with the
`verified` date carried through and anything past a staleness threshold marked:

```markdown
<!-- GENERATED by `temper memory emit` — do not edit -->
- [`--edge-type` filters KIND, not LABEL](temper://019f…) — silently kills graph expansion  [verified 2026-08-01]
- [server embed is a fallback; client-side is prod](temper://019f…)  [verified 2026-06-12 — UNVERIFIED 50d]
```

**The gate cannot be a CI job.** The index lives at `~/.claude/projects/<project>/memory/MEMORY.md`
— outside the repo, and per-machine — so there is no committed file for CI to diff against, unlike
the `agent-skills/` projection's `.github/scripts/check-skills-drift.sh`. The gate is instead
`temper memory check`: a local command that re-emits, diffs against what's on disk, and exits
non-zero on drift — so a hand-edited index is a failure a person or a hook catches by running it,
rather than a slow divergence.

**`emit` is a Claude Code concern, not a universal one.** Desktop, mobile and web read memories from
Temper natively; they need no index. This matters for scoping: the known MCP lag bites on
*authoring* a memory from those environments, not on reading one, so it does not block this work.
It is named here so its absence from scope reads as a decision.

## Adoption is opt-in, and per machine

Other machines and other projects must be able to *discover* this and evaluate their own memory set
in the same light — and must be able to decline. A Temper user who wants none of this should never
have to know it exists.

**Opt-in lives in temper config and is absent by default.** No `[memory]` section means the feature
is off and `emit` is a no-op that says why.

```toml
[memory]
# Contexts whose memories reach EVERY project on this machine.
shared_contexts  = ["@me/working-agreements"]

# Contexts for this project. A list, because a project legitimately spans contexts.
project_contexts = ["@me/temper"]

index_path       = "~/.claude/projects/-Users-.../memory/MEMORY.md"
```

`emit` renders the union of both lists, grouped by context — Phase 1's `all_contexts()` chains
`shared_contexts` then `project_contexts` and treats them identically; both are read-only inputs to
the render.

**Phase 2 (not built):** the two keys are meant to differ on the write side too — a new
cross-project memory defaulting to the first `shared_contexts` entry, a project-specific one to the
first `project_contexts` entry. Without that distinction the split is conventional only, and a
convention nothing enforces is how the reach problem comes back. Phase 1 has no write path for
memories at all (there are zero `memory` resources yet), so this has no consumer today — noted here
so it is read as planned, not shipped.

**Discovery is a read command that works whether or not you have opted in:**

```
temper memory status
```

For the machine it runs on: local memory files found, memories in Temper, opt-in state, and — the
part that makes it useful on a machine that has never adopted — **which local files have no
counterpart in Temper**. It is read-only, so trying it costs nothing.

### Discovery has two audiences, and only one of them has a MEMORY.md

- **Claude Code on another machine** has a memory directory, a config, and the CLI. It learns this
  exists from the **CLI-packaged skill** (`temper skill install`), runs `temper memory status`, sees
  its own divergence measured, and opts in when it wants to. This is the audience the section above
  is written for.
- **Claude Desktop, mobile, and web have no `MEMORY.md` and no CLI.** Nothing there needs an index or
  an opt-in — memories are simply `memory`-typed resources they can already read over MCP. What they
  need is to *know that convention exists*, which the **MCP skill artifact** can now carry: the skill
  ships as a versioned `temper-skill-v<ver>.zip` for Desktop. For them the guidance is "memories are
  resources of type `memory`; read them; honour them", not "reconcile your local files".

**One constraint this must not break.** The MCP skill tree is **config-free**, and that is precisely
what lets it be a committed projection with a drift gate. It must therefore describe the *convention*
and the *doc type* — never a user's context names. `@me/working-agreements` is a per-user config
value and belongs in the CLI-side skill only. Baking it into the MCP tree would silently remove that
tree's ability to be gated.

**And the latency is real:** the MCP skill artifact ships on a release cadence, so Desktop learns
about this one release behind the CLI, not immediately. Stated rather than discovered later.

### The duplication hazard, and what does not close it

**Two machines migrating independently will duplicate.** Separate machines have been learning the
same lessons and writing them down separately — different words, different slugs. If both migrate
into a shared context, Temper holds two memories saying nearly the same thing, the recall path
cannot tell they are siblings, and **both** render — one of them staler than the other.

Same shape as issue #581, generalized: **reconcile, do not blind-create.** `temper memory migrate`
searches the target context before each write and surfaces near-matches for a human decision rather
than resolving them itself. It is interactive by default on a collision and refuses to run
unattended without an explicit flag.

**This detects overlap; it does not eliminate it.** Semantic near-duplicates phrased differently
enough will pass the search and land twice. The residue is real and is named here rather than
claimed closed — a reconciler that silently merged on similarity would be worse, because it would
destroy one of two accounts that a human might have wanted to compare.

> **RETIRED — `[decided — 2026-08-02, Pete]`.** The two paragraphs above describe a mechanism that
> no longer exists. The pre-write search, `--collision-limit` and the per-collision prompt were
> deleted; `migrate` now confirms the batch once and states at the prompt that it detects nothing.
>
> **The measurement that retired it.** On the first run at real scale — the desktop migration,
> 2026-08-02, against a 184-memory store — the search surfaced **54 collisions, 3 genuinely
> overlapping**. The 51 false positives matched on *shared project vocabulary*, not on claim
> (`temper invocation` against `NEVER abbreviate a UUIDv7 to its prefix`). The paragraph above named
> the wrong residue: the risk was never what the search *missed*, it was that ~94% of what it
> *caught* was noise, which buried the three real ones and made switching the gate off attractive.
> On 2026-08-01 the final 23 memories were written with `--collision-limit 0`, under operational
> pressure from an unrelated failure, by the person who wrote the requirement.
>
> **The clause did not retire with the mechanism.** `a-near-duplicate-must-never-be-resolved-without-a-human`
> still holds; what moved is who forms the candidate set — an agent reading local memories against
> the store and comparing the **claim** rather than the wording, bringing a short list a human can
> adjudicate. A compiled-in gate is unskippable and this is not; the trade is defensible only because
> what was traded away was already being switched off, **not** because guidance is as strong as a
> gate. Two repairs are **Rejected**, not deferred: tuning the score or swapping full-text for
> embedding cosine (several of the 51 would score *higher* under embeddings, being genuinely about
> the same subsystem), and automatic merging above a confidence threshold (which the clause forbids).
> Tracked as *Retire the FTS collision gate — judgement, not lexical overlap, decides what is a
> near-duplicate*, resource `019fc062-5481-77f0-a0fa-e092d40759d4`. The operator-facing account is
> [docs/guides/operational-memory.md](../../guides/operational-memory.md).

### Sequencing across machines

One machine migrates; the others adopt at their own pace and reconcile against what is already
there. Two parallel first-migrations is the case the reconciler handles least well, and nothing
requires it — a later adopter runs a strictly smaller and safer job than the first.

## Migration

**Not a big bang, and not a full triage.** Both were considered and rejected:

- **Big-bang all 182** would migrate dead weight. Two entries are *known* stale and the other 180
  are unchecked; landing them wholesale gives unearned authority to unreviewed claims, inside the
  system whose entire premise is that claims carry standing. It also runs 182 sequential writes
  through the lost-acknowledgment hazard of issue #581.
- **Triage all 182 first** is a project in its own right, and triage quality is exactly what cannot
  be verified cheaply — a great deal of judgment spent deciding what deserves judgment.

The plan instead:

1. **Build the contract, `emit`, and the gate first.** Nothing migrates until the index can be
   rendered and checked.
2. **Migrate the 69 `feedback` memories as one deliberate, reviewed batch.** This is the
   cross-environment payoff: they are durable, they apply to every project, and they are the
   population with no Temper home at all today.
3. **The 107 `project` memories move lazily** — on next write, amendment, or correction. They are
   temper-specific and already sit beside the resources they discuss, so they lose least by waiting.
   During the transition `emit` renders the union of migrated and not-yet-migrated entries.
4. **The largest files are replaced by pointers, not converted.** `project_context_regions_goal.md`
   (23KB) and its siblings are research documents about goals that already exist in Temper. They
   become one-line pointers to the real resource. This resolves most of the size pressure without
   touching a single hook-sized entry.

## Excluded, with reasons

- **Promoting `status`/`verified` into `managed_meta`** — deferred to v2; the cost is stated above
  and carried by `emit` in the meantime.
- **Homing memories in the `Temper — self-cognition` cogmap** (713 resources) — considered and not
  taken. It adds per-memory authoring ceremony (invocation envelope, provenance, fold-then-recreate
  supersession) for what is often a one-line trap note, and the map is temper-scoped, so
  cross-project feedback would have no home in it. Revisitable once memories exist as resources.
- **Evidential standing / citation audits over memories** — the most temper-native staleness answer
  and deliberately not taken for v1. Heavy ceremony for a one-line note, and it would require the
  auditor persona to run over memories.
- **Automatic staleness re-verification** — `verified` is set by whoever checks the claim. Nothing
  re-checks automatically, and the index marks age rather than asserting wrongness. **An old date
  means unexamined, never false**, and the render must not blur the two.
- **The other projects' memory directories** (`learning-maths`, `storyteller`) — the design admits
  them at no extra cost, but migrating them is out of scope here.
- **Any central registry of which machines have adopted.** Opt-in is per-machine config and nothing
  aggregates it; `temper memory status` answers for the machine it runs on and no other. A fleet
  view would need a home for machine identity that does not exist yet.

## Open, and honestly unbounded

- **What threshold makes a memory "stale enough" to mark** is unmeasured. There is no evidence yet
  for 30 vs 90 days; it is a rendered warning, not a gate, so a wrong first guess is cheap.
- **Nothing forces the lazy tail to finish.** The 107 may sit half-migrated indefinitely. That is
  accepted: an untouched memory is one nothing has needed, which is weak evidence it was never
  load-bearing — but it is *weak* evidence, and this is a real remainder, not a solved problem.
- **Per-machine emit.** Something must run `emit` on each machine to refresh that machine's index.
  Whether that is a hook, a session-start step, or manual is unresolved.

## See also

- [T1 columns 1–3 — the query-shaped surface](temper://019fbe0f-762a-7ad1-81be-1e346a34ea0c)
- [T1 column 4 — misleading surface, 18 proven findings](temper://019fbe09-d2c9-7c70-981c-d97a62a344cc)
- Issue #581 — CLI write path: a lost acknowledgment reads as a failure
- PR #609 — agent-skills as a generated projection, the pattern this borrows
