# Charter-bootstrapping procedure × diverse charter corpus

**Status:** design approved (brainstormed 2026-06-10), ready for the first build task
**Date:** 2026-06-10
**Goal:** `substrate-kernel-to-cognitive-map`, workstreams 1+4 (braided)
**Task:** `2026-06-10-charter-bootstrapping-procedure-x-diverse-charter-corpus-braided-design-the-elaboration-session-by-authoring-real-charters`
**Builds on:**
[`domain-b-charter-questions-regulation-edge-semantics`](2026-06-04-domain-b-charter-questions-regulation-edge-semantics-design.md)
(telos-charter composition, regulation-as-`express`-edged resources, `cogmap_genesis`),
[`temper-next-d3-evolvable-telos-shape`](2026-06-09-temper-next-d3-evolvable-telos-shape-design.md)
(block roles `statement`/`question`/`framing`, questions-with-context grain, generic `resource_blocks` reads),
the seed/scenario document split (PR #126 — seeds at `schema-artifact/seeds/`, `seed.schema.json`,
load-path equivalence + charter roundtrip proofs), and the seed-skill session
`2026-06-01-seed-skill-scope-portable-vs-bound-awareness-access-bounded` (the elicitation results this
procedure transcribes).

> **Headline.** The substrate half of charter-bootstrapping exists (seed format, genesis, block
> lifecycle, roundtrip proofs). This spec designs the **procedural half** — the multi-session,
> resumable elaboration session, packaged as an agent skill — and the **diverse charter corpus**
> that is simultaneously its dogfood and the goal's confidence gate. The braid is deliberate:
> the procedure is refined *by* authoring real charters, and the lessons accrete as regulation
> in two tiers, not as up-front spec.

---

## 1. Problem

One scenario proves the machinery; only diversity proves the model. The goal's confidence gate —
drift-detection acceptance ("incremental ≡ full across the scenario corpus") and migration
confidence — inherits from a corpus of genuinely different, true-to-life telos-charters that does
not yet exist. And the procedure by which a charter comes to exist (elicitation → questions-with-context
→ framing → seed → re-entry) is designed in fragments across the lineage but has never been
assembled, packaged, or run.

Two strands, braided:

1. **Procedure** — the elaboration session as an agent skill + this written record. Underspecified
   half: resumability, the foundational-system vs smaller-concern-map differentiation, and
   memories-as-regulation *about bootstrapping itself*.
2. **Corpus** — 3–5 charters authored through the procedure, each a corpus entry: real guiding
   statements, questions-with-context, framing prose.

## 2. Decisions (settled in brainstorming, 2026-06-10)

| Decision | Choice |
|---|---|
| Approach | **Thin v0 skill first, then one elaboration session per charter.** The elicitation flow is transcription of earned design (seed-skill session), not invention; everything unearned accretes as regulation. Author-first-extract-later was rejected (the first sessions would lack the resumability and stopping-rule discipline that multi-session elaboration needs before it can safely pause); full-spec-first was rejected by the task's own framing. |
| Corpus domains | **Migration/initiative map, temper-foundational, storyteller, learning-maths, tasker-core (optional fifth).** |
| First dogfood | **Migration/initiative map** — the best-understood shape (the triage role-play exercised it); gentle first iteration before the harder foundational case. |
| Per-entry validation bar | **Seed-only now, scenarios later.** Each charter lands as a seed YAML that loads via `cogmap_genesis` and passes the proofs. Steps/expectations are authored when drift-detection work needs them — the bar here is charter truth, not runbook invention. |
| Regulation homes | **Both layers, by scope.** Charter-bound lessons → `cogmap_regulation` resources `express`-edged off that charter's telos, *in the seed* (dogfoods the substrate's own regulation shape). Process-portable lessons → the skill's `guidance/` files, human-gated. |
| Public-repo handling | **Publishable register, in-repo.** All seeds live in `schema-artifact/seeds/` (the repo is public). Charters are true-to-life in structure and questions but written at a register fit to publish: no sensitive specifics, handles instead of full names in `world.profiles`. Keeps the corpus reproducible and CI-runnable. |

## 3. The v0 skill

**Home:** `~/.claude/skills/charter-bootstrapping/` — `SKILL.md` + `guidance/` (the portable-lesson
layer), following the temper-skill pattern. The build task installs it in-session (dogfood requires
the actual artifact).

**SKILL.md is deliberately thin.** Every step transcribes an earned result; nothing is invented.
The skill names what it does NOT yet know (when foundational elicitation should diverge from
small-map elicitation; what a good framing block sounds like; promotion cadence) so dogfood
sessions recognize lessons as lessons.

### The elaboration session (8 steps)

1. **Open: name the map kind.** Foundational-context system (org topology, commitments, tools,
   modes-of-working as the telos) vs smaller-concern map. Sets elicitation breadth. The
   differentiation itself is one of the things dogfooding will sharpen.
2. **Elicit purpose + thin-who**, conversationally, one question at a time. Carried verbatim:
   *purpose-without-a-who is incoherent* — the who is structurally part of the purpose statement;
   thin who-*references*, never persona characterization.
3. **Stop at sufficiency-for-first-action.** The who is specific enough when an agent's first
   does-this-matter call would be *correctable in the right direction* — divergence from a
   specific-enough who is signal; divergence from a vague who is noise. Explicitly not a
   resolution target.
4. **Derive questions-with-context** — the load-bearing primitive of the seed. Each question
   encodes a who and a purpose in one breath ("When a Notion doc changes, does anything our
   on-call needs to know change?"). Context is the situating prose that rides as interior chunks
   of the question block (one `role='question'` block per question — D3 §3.6 grain).
5. **Framing prose** — grounding-of-purpose and positioning language: coordinates-with-X,
   relies-on-systems-A/B/C, domain bounds (D3 §6; `role='framing'` blocks).
6. **Author the seed YAML** against `seed.schema.json`: `cogmap.telos`
   (title/statement/questions/framing), `world` (profiles/entities at publishable register),
   initial `resources`/`edges` **only as far as they are real today** — no invented texture;
   `uses_lenses: [telos-default]` as the starting default (global lenses in `seeds/system.yaml`).
7. **Validate** — schema-valid; loads via `cogmap_genesis`; charter roundtrip holds
   (statement/questions/framing byte-exact through role-filtered `resource_blocks` reads).
   Run under `--features artifact-tests` (write-path nextest group).
8. **Session-end regulation sort** — the two-tier discipline enacted literally. For each lesson:
   apply the *directionality-under-transfer* test — would it still be right run against the
   maximally-different charter currently in the corpus (storyteller, once it exists)?
   Telos-parameterized → portable → `guidance/` (human-gated). Carries answers → charter-bound →
   a `cogmap_regulation` resource in this charter's seed, `express`/`operationalized_by`-edged
   off the telos. Recurrence across engineering-shaped charters is NOT evidence of portability.

### Resumability

The draft seed YAML on disk **is** the elaboration state; one temper task per charter tracks
stage (`backlog → in-progress → done` at validation); each elaboration session saves a temper
session note whose Next Steps carry the conversational thread. No new machinery. A paused draft
is allowed to be schema-invalid; validation gates *done*, not *pause*.

## 4. The corpus

Working order (adaptive — the roadmap guides sessions, it doesn't pin them):

| # | Charter | Shape it pressures |
|---|---|---|
| 1 | **Migration/initiative map** | The known shape (triage role-play); tests the mechanics. The real initiative is picked at elicitation time — temper's own workstream-6 migration/convergence is a natural candidate, but that is the session's call. |
| 2 | **Temper foundational** | The foundational-context-system end; pressures the kind-differentiation early, while the procedure is soft. |
| 3 | **storyteller** | First non-engineering shape — placed before the skill has seen too many engineering charters (the recurrence-is-not-portability guard); becomes the live falsification referent for the transfer test. |
| 4 | **learning-maths** | Mastery-trajectory telos; no team-attention structure at all. |
| 5 | **tasker-core** (optional) | Second foundational data point if the corpus still feels thin after four. |

**Corpus definition of done:** every seed in `seeds/` passes the sweep test (§5); the skill's
`guidance/` layer holds only transfer-tested portable lessons; at least one charter exercises the
rich questions-with-context + framing path end-to-end (all should; one is the floor — today only
the bespoke `cogmap_genesis_charter` test touches it, and `onboarding-cogmap.yaml` deliberately
leaves question context empty).

## 5. Validation harness (the one code touch)

Existing proofs hardcode the onboarding seed (`seed_load_path_equivalence.rs:25`,
`charter_yaml_roundtrip.rs`). The corpus wants a **sweep test** in temper-next's artifact tests:

- Glob `schema-artifact/seeds/*.yaml` (excluding `system.yaml`, the boot-seed).
- For each: schema-valid against `seed.schema.json`; loads through the standard seed path
  (`load_seed` after bootseed); charter roundtrip — statement, questions(+context), framing
  reproduce byte-exact through role-filtered `resource_blocks` reads.
- Lives in the `temper-next-write` nextest group (namespace-owning, serialized) under
  `--features artifact-tests`, per the established convention. No CI job runs it; local ritual.
- `.sqlx` regeneration only if it adds macro queries (`cargo make prepare-next`).

Per-seed proofs that exist only for the onboarding seed (cross-path membership equivalence, S6
expectations) stay onboarding-specific — they prove machinery, not charters; the sweep proves
charters.

## 6. Build decomposition / roadmap

1. **First build task (build/small) — "v0 charter-bootstrapping skill + corpus seed sweep test."**
   Write + install `SKILL.md` (the 8 steps, §3) with an empty `guidance/`; add the sweep test;
   prove it green against the existing onboarding seed. This is the only code-touching chunk.
2. **Elaboration sessions, one per charter** (§4 order), each its own temper task created when the
   session starts — the plan-large cadence: work the task, learn, evolve, create the next. Each
   session ends with a validated (or paused-draft) seed, the regulation sort, a skill revision if
   earned, and a session note.
3. **Corpus close-out:** goal status update (workstreams 1+4), promotion review of `guidance/`
   accretions, and a decision point on whether the procedure doc needs a public-facing
   (docs/cognitive-maps) rendering — out of scope until the skill stabilizes.

The braided plan task stays **in-progress** across all of this; it is done when the skill is
stable and 4–5 corpus charters validate.

## 7. Scope boundaries

**Out of scope:** scenario steps/expectations for corpus entries (drift-detection-time, by the
seed-only bar); the access scaffold (goal workstream 2); `cogmap_regulation` read demotion
(forward seam from D3 §4.2); tier-two promotion *mechanics* beyond human-gating (blocked on
computable cogmap-shape, carried open); any public-facing docs/UI rendering of the procedure;
scenario-corpus drift-detection work itself (workstream 5, unblocked by this corpus when it
lands).

## Connections

- **Goal:** `substrate-kernel-to-cognitive-map` (workstreams 1 second-half + 4; the confidence gate).
- **Discharges into:** drift detection (workstream 5) and migration confidence (workstream 6),
  both gated on this corpus.
- **Procedure lineage:** `2026-06-01-seed-skill-scope-portable-vs-bound-awareness-access-bounded`
  (sufficiency-for-first-action; question-set as irreducible primitive; scope-portable vs
  scope-bound; two-tier learning + regulation channel),
  `2026-05-31-definitional-fallacy-concept-as-basin-telos-resolves-threshold-primitive`
  (telos-authoring as forming-deformation).
- **Substrate it rides:** seed/scenario split (PR #126), D3 block roles + `resource_blocks`,
  domain-B charter composition + `cogmap_genesis`, system lenses (`seeds/system.yaml`).
