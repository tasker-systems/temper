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
| Home + scope | **Split by reach: two contexts** | Reach is declared by *where a memory lives*, not by a per-memory field that can be set wrong and that nothing validates |
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
@me/working-agreements   ← feedback (69). Read by EVERY project's emit.
@me/temper               ← project (107) + reference (6), beside the goals they discuss.

emit for project X  =  @me/working-agreements  ∪  @me/<X>
```

Reach is a property of the home, so a memory cannot claim a reach it does not have. Onboarding a
second project (`learning-maths` and `storyteller` both exist) is one new context and one line of
emit configuration — no per-memory rework.

**Supersession replaces deletion.** A memory that turns out wrong takes `status: superseded`, keeps
its body — which is where the *principle* lives, distinct from the instance that expired — and drops
out of the index. The ledger keeps the history. This is the shape hand-written on 2026-08-01 for the
two stale CLI memories, which retained their generalizable traps ("a filter that does not filter
makes presence the lie"; "a verification that cannot fail is not a verification") while their
instances were marked fixed.

## Emit and the drift gate

A new CLI command, following `temper skill emit`:

```
temper memory emit --project <name> --path ~/.claude/projects/<project>/memory/MEMORY.md
```

Renders one line per **active** memory across the two contexts, grouped by section, with the
`verified` date carried through and anything past a staleness threshold marked:

```markdown
<!-- GENERATED by `temper memory emit` — do not edit -->
- [`--edge-type` filters KIND, not LABEL](temper://019f…) — silently kills graph expansion  [verified 2026-08-01]
- [server embed is a fallback; client-side is prod](temper://019f…)  [verified 2026-06-12 — UNVERIFIED 50d]
```

The gate re-emits and diffs against the committed file, failing on any difference — so a
hand-edited index is a build failure rather than a slow divergence.

**`emit` is a Claude Code concern, not a universal one.** Desktop, mobile and web read memories from
Temper natively; they need no index. This matters for scoping: the known MCP lag bites on
*authoring* a memory from those environments, not on reading one, so it does not block this work.
It is named here so its absence from scope reads as a decision.

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
