# Subagent Guidance

Principles for any subagent dispatched during temper workflows. When dispatching subagents,
include all applicable principles verbatim in the subagent prompt. Do not summarize or
paraphrase — subagents need the full text to follow them.

## Foundational Principles

### SG-1: Follow Existing Patterns
Before writing anything, read the file you're modifying AND a sibling in the same module.
Match the style you find: naming, imports, structure, error handling. Don't invent new patterns.

### SG-2: Single Responsibility
Each function does one thing. If it constructs AND processes AND formats — split it.
Follow the project's existing layering.

### SG-3: No Logic Duplication
Would two implementations drift independently over time? Extract. Otherwise leave inline.
Don't create premature abstractions for one-time operations.

### SG-4: Test Strategy
Unit tests co-located with code. Integration tests separate. One behavior per test with
descriptive names. Tests must actually run — verify, don't assume.

### SG-5: Don't Over-Build
Implement exactly what the task says. No speculative features, no defensive code for
impossible cases, no "nice to have" extras.

### SG-6: Verify Before Claiming Done
Run the verification command. Read the output. Don't claim success based on what you
think the code does.

## Friction-Derived Principles

### SG-7: Prefer Native Solutions
Don't invent when the framework, language, or platform provides. If a proper tool exists,
use it over a hand-rolled alternative. The idiomatic solution is almost always better than
the clever workaround.

### SG-8: Front-Load Constraints
Before proposing anything: (1) existing abstractions for this? (2) platform/deployment
limits? (3) async/performance requirements? List findings before writing code.

### SG-9: Don't Dismiss Owned Failures
If the user owns both sides of an interaction, debug the full stack. Never declare
"not our problem" without proving external causation.

### SG-10: Checkpoint Before Continuing
After each major step, report: what's done, what's next, any concerns about approach drift.

## Quick Reference

| Wrong | Right |
|-------|-------|
| Silently swallow errors with defaults | Return specific errors with context |
| Build a new abstraction for one use | Inline it, extract later if repeated |
| Claim "tests pass" without running them | Run, read output, report result |
| Propose complex solution without checking | List existing tools/abstractions first |
| Declare a failure "not our problem" | Prove external causation before dismissing |
| Skip reading sibling files before editing | Read the file AND a neighbor first |

## Domain Applicability

- **Software tasks:** All 10 principles apply.
- **Non-software tasks:** SG-1 (follow patterns), SG-5 (don't over-build), SG-6 (verify), SG-10 (checkpoint) apply. The rest are software-specific.
