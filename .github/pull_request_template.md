## What

<!-- The change, present tense, a sentence or two. What the reader will find different. -->

## Why

<!-- The defect or the need. If a measurement motivated this, give the number — not the
     investigation that found it. -->

## Approach

<!-- Only where a reviewer would otherwise propose the alternative: say briefly why this shape
     and not that one. Delete this section when the shape is uncontroversial. -->

## Verification

<!-- What was run, and what a reviewer can re-run. Name the test that fails if this regresses. -->

<!-- ─────────────────────────────────────────────────────────────────────────────────────────
     Four rules. Full reasoning: internal/agents/conventions.md → "PR descriptions".

     1. THE CODE CARRIES THE DETAIL. If a rationale needs more than a paragraph here, it belongs
        in the migration header or the doc comment, next to the thing it explains — where it
        survives this PR being merged and forgotten. Link to it; don't restate it.

     2. NO SESSION NARRATIVE. What you tried first, which probe was wrong, what an agent
        reported. The reviewer needs the change, not the path to it.

     3. THIS REPO IS PUBLIC, AND A DESCRIPTION IS NOT A DISCLOSURE SURFACE. No specs, plans, or
        gap inventories — the same reasoning that moved them to temper-artifacts and that
        .github/scripts/check-no-process-artifacts.sh enforces for the tree. No production
        identifiers, tenant data, or operational state. Scope statements ("cost only; behavior
        unchanged") are useful and welcome; an enumerated list of what remains weak is not.

     4. SAY WHAT IT DOESN'T DO, IN SCOPE TERMS. Reviewers infer coverage from silence. One line
        naming what this deliberately leaves alone is worth more than a paragraph of caveats —
        and it is a different thing from rule 3's inventory.
     ───────────────────────────────────────────────────────────────────────────────────────── -->
