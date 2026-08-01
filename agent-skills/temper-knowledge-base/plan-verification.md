# Plan/Reality Verification Before Subagent Dispatch

When executing a written plan via subagent dispatch, the controller must verify the plan's
code-analysis claims against the actual code **before** writing implementer prompts. The plan is a
hypothesis. The code is ground truth.

> **Read this as a principle, not a checklist.** The worked examples below are evidence for the
> principle, never the scope of it. If you catch yourself thinking *"this doesn't cover my case"* —
> that thought is the failure mode, not a finding.

## Why this matters

Plans often contain claims that look like specs but are actually unverified guesses by the planner:
function names, signatures, struct field lists, "find where X is converted into Y" descriptions,
call graph assumptions. When those guesses are wrong, the implementer either hits compile errors
(cheap to recover from) or, worse, builds the wrong thing because they followed the plan literally
instead of the code's actual shape (expensive, sometimes only caught at code review).

The failure mode is recognizable: **confident-but-unverified API claims**. The plan author
remembered the general shape of the codebase but didn't grep-check the specifics. Common signatures:

- A function name that's *almost* right (`load_or_default` instead of `load_manifest`)
- A signature with the wrong shape (`fn(path, key, value)` when the real fn takes `fn(content, key, value)`)
- A described data flow that doesn't exist in the code at all
- An assumed call graph that's flat when it's actually layered (a CLI wrapper that delegates to a
  separate orchestration function)

These are low-effort to detect with `rg` or a quick `Read`, but they cost a full implementer
round-trip to recover from if missed.

## The half this does NOT cover — read this before concluding you are done

Everything above is about **names the plan borrows**. It is a name-checker, and it is blind to the
more dangerous species: **predicates the plan authors itself**.

Restated logic *names no API*, so `rg` finds nothing to check. And it is usually a **true statement
about real columns**, so printing the object it names *confirms* it. Both rituals pass; the bug
ships. The catching question is not *"is this claim true?"* but:

> **"Does this codebase already have a name for this concept?"**

That is a comparison against the incumbent, not a verification of the claim. Any place a plan
*inlines* logic the system already exposes under a name is a drift site — the inlined copy will
diverge from the thing it was supposed to mirror, silently, because nothing links them.

**And check the document against itself.** When a plan's prose and its code sketch disagree, **the
sketch is what ships** — implementers build the code block, not the paragraph above it.

## When this applies

Any time you are executing a plan task that names specific code, or that authors logic the system
may already have. It applies regardless of who wrote the plan — you, a brainstorming subagent, a
planning subagent, or another conversation — and regardless of how recent it is. **Including when
you wrote it yourself, in this session, and believe you verified it.** Plans go stale; authors are
not exempt from their own drafts. The controller's job is to revalidate at dispatch time.

## How to apply

Before crafting any implementer prompt for a code-touching task:

1. **Read the actual files the task touches.** Don't trust the plan's line numbers or function
   names — open the file and confirm the real signatures, fields, and call sites.
2. **Grep for every API the plan names.** If it says "the existing `build_plan` function", run
   `rg "fn build_plan"` first. Nothing returned ⇒ the plan is wrong and you must design the right
   approach before dispatching.
3. **For every predicate the plan authors, find the incumbent.** Ask what the system already uses
   for that concept and call it, rather than restating it. If the plan restates, that is the finding.
4. **Verify call graphs.** Trace one level of indirection; the command may delegate elsewhere.
5. **Treat plan code sketches as starting points, not specs.** Where reality differs, rewrite them
   in the implementer prompt with the real shapes.
6. **Surface gaps explicitly in the implementer prompt.** Use "⚠️ Plan/reality gap" sections: "the
   plan says X, but the real API is Y — use Y."
7. **Don't blame the implementer when the plan was wrong.** If a subagent goes off the rails because
   the plan referenced a function that doesn't exist, that's the controller's verification gap.

## Universal principle

If verification is too expensive due to capacity or context pressure, that's an unavoidable cost —
but the response is to skip the work or pause, not to push through with unverified claims.
Confident-sounding API names without a grep behind them are the same anti-pattern good engineers
coach their teams away from in human code review. Apply the same standard to plan-driven dispatch.

The cost of grep-checking before dispatch is much lower than the cost of an implementer fixing a
phantom call site, a reviewer reapproving the wrong thing, and a code reviewer catching it after.
