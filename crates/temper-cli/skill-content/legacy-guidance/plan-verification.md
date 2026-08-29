# Plan/Reality Verification Before Subagent Dispatch

When executing a written plan (superpowers or otherwise) via subagent dispatch, the controller must verify the plan's code-analysis claims against the actual code **before** writing implementer prompts. The plan is a hypothesis. The code is ground truth.

## Why this matters

Plans often contain claims that look like specs but are actually unverified guesses by the planner: function names, signatures, struct field lists, "find where X is converted into Y" descriptions, call graph assumptions. When those guesses are wrong, the implementer either hits compile errors (cheap to recover from) or, worse, builds the wrong thing because they followed the plan literally instead of the code's actual shape (expensive to recover from, sometimes only caught at code review).

The failure mode is recognizable: **confident-but-unverified API claims**. The plan author remembered the general shape of the codebase but didn't grep-check the specifics. Common signatures of this failure:

- A function name that's *almost* right (`load_or_default` instead of `load_manifest`)
- A function signature with the wrong shape (`fn(path, key, value)` when the real fn takes `fn(content, key, value)`)
- A described data flow that doesn't exist in the code at all (e.g. "find where ValidationIssues become FixActions" when there is no such conversion path)
- An assumed call graph that's flat when it's actually layered (e.g. "wire into `sync::run`'s upload loop" when run is a CLI wrapper that delegates to a separate orchestration function)

These are low-effort to detect with `rg` or a quick `Read`, but they cost a full implementer round-trip (and sometimes a re-review cycle) to recover from if missed.

## When this applies

Any time you are executing a plan task that names specific code:
- Function names ("call `vault::set_frontmatter_field`")
- Struct field lists ("extend `ApplyReport` with these fields")
- Call sites ("find where X is converted into Y")
- Architectural claims ("the `run` function has an upload loop you should wire into")

It applies regardless of who wrote the plan — you, a brainstorming subagent, a planning subagent, or another conversation — and regardless of how recent the plan is. Plans go stale. The controller's job is to revalidate at dispatch time.

## How to apply

Before crafting any implementer prompt for a code-touching task:

1. **Read the actual files the task touches.** Don't trust the plan's line numbers or function names — open the file and confirm. Note the real signatures, the real struct fields, the real call sites.
2. **Grep for every API the plan names.** If the plan says "the existing `build_plan` function", run `Grep "fn build_plan"` first. If it returns nothing, the plan is wrong about that name and you need to design the right approach yourself before dispatching.
3. **Verify call graphs.** If the plan says "wire into `sync::run`", confirm that `sync::run` actually does what the plan thinks it does. Trace one level of indirection. The CLI command may delegate to a different action function.
4. **Treat plan code sketches as starting points, not specs.** Field lists, function signatures, and example bodies are hypotheses to verify. Where reality differs, rewrite them in the implementer prompt with the real shapes.
5. **Surface gaps explicitly in the implementer prompt.** Use "⚠️ Plan/reality gap" sections that tell the implementer "the plan says X, but the real API is Y — use Y." This protects them from blindly following bad spec text and gives the spec reviewer something concrete to verify against.
6. **Don't blame the implementer when the plan was wrong.** If a subagent goes off the rails because the plan referenced a function that doesn't exist, that's the controller's verification gap, not the implementer's failure.

## Universal principle

If verification is too expensive due to capacity issues or context pressure, that's an unavoidable cost — but the response is to skip the work or pause, not to push through with unverified claims. Confident-sounding API names without a grep behind them are the same anti-pattern that good engineers coach their teams away from in human code review. Apply the same standard to plan-driven dispatch.

The cost of grep-checking before dispatch is much lower than the cost of an implementer fixing a phantom call site, a spec reviewer reapproving the wrong thing, and a code reviewer catching it after the fact.
