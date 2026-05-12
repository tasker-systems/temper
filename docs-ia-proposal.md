# Docs IA Proposal — temperkb.io

A first-pass information architecture for replacing the current "get started" framing of temperkb.io with the conceptual-first frame established by the three source documents (manifesto, semantic model, framing schema).

This is a proposal, not a decision. Section trees are annotated with rationale and with the questions they're waiting on.

---

## Constraints the IA is trying to honor

Pulled directly from the three source documents so the rationale below is checkable against them:

- **Attention is the teleological anchor.** A docs surface is itself a demand on attention. The IA should be navigable cold, surface what matters first, and not require the reader to construct a model from scratch.
- **Translation is irreducible.** The IA cannot produce a "neutral" or "canonical" simplification of the model. It can introduce the model progressively, but it must not pretend a simpler version is the real version.
- **Don't operationalize what's deliberately open.** The schema's "Intentionally Open" list is part of the model's discipline. The IA must not flatten it into "coming soon" or "roadmap" framing.
- **Don't smuggle product claims.** Pages should commit only to what the substrate-level model commits to. Where Temper-as-system instantiates a model commitment, the page should be explicit about which it is naming.
- **Efficiency-as-ethic.** Pages have a length budget. A page that takes 20 minutes where 5 would do has cost the reader 15 minutes they don't get back. The IA should be navigable by short hops, not long scrolls.

---

## Proposed tree

```
/                           What Temper is for             (manifesto layer)
/foundations                Working context: a model       (model overview + anchor)
/foundations/ontology         Data, intention, information, knowledge
/foundations/manifold         Positions, fields, streams
/foundations/time             Time, events, derivation
/foundations/deformation      Forming, forgetting, scarification
/foundations/perspectives     Who is asking
/foundations/translation      Irreducibility, bridges, observer-relativity
/primitives                 [pending] Schema reference     (framing layer)
/using-temper               [pending] How to use it        (existing content, scoped)
```

Three tiers, each operating at a different layer of the source documents. No tier pretends to be a simpler version of the tier above it.

---

## Tier 1 — `/` What Temper is for (manifesto layer)

**Source:** *An Attention Manifesto*

**Replaces:** the current `(public)/+page.svelte`, which leads with "knowledge work deserves structure" and walks through CLI verbs.

**What it is:** a single page that establishes attention as the teleological anchor and names the four commitments that follow from taking that seriously.

**What it deliberately is not:**
- A feature tour. The manifesto explicitly rejects framing the commitments as "features I want to ship."
- A landing page in the conventional sense. It does not ask the reader to install or sign up.
- A summary of the model. The model lives at `/foundations`; this page points to it without summarizing it.

**Rationale:** The manifesto is the only one of the three documents that names *why the rest exists*. A visitor who reads only this page should leave knowing what Temper takes a stand on, without having absorbed any of the model's machinery. A visitor who wants the machinery follows the link to `/foundations`.

The first-draft copy for this page is in `landing-draft.md`.

---

## Tier 2 — `/foundations` Working context: a model (semantic-model layer)

**Source:** *A Semantic Model of Working Context*

**What it is:** a six-page conceptual progression that introduces the model's primitives in the order the source document does. Each page is short enough to read in one sitting; the set is navigable as a sequence or as standalone pages.

**Section index:**

- `/foundations` — Overview and the teleological anchor. Restates what the model is and what it is for. Single short page. Approximately the first 300 words of the source document, edited for a reader who has not read the manifesto separately.

- `/foundations/ontology` — Data, intention, information, knowledge. The stratified ontology. Names the *knowledge-bases-are-misnomers* commitment explicitly — this is one of the model's load-bearing claims and deserves a page.

- `/foundations/manifold` — Positions, fields, streams. The geometry. Includes the bidirectional coupling claim ("attention is force, force deforms"), which is the model's most consequential commitment and is easy to mishandle if buried.

- `/foundations/time` — Time as primary axis. Events-as-primary. Why this is a substrate commitment, not an implementation detail. This page is where the model justifies append-only ledger semantics without yet naming any system that implements them.

- `/foundations/deformation` — Forming and forgetting. Strong vs. weak deformations, the recording threshold, decay/deformation/folding as three distinct forgetting mechanics, scarification as a property certain deformations carry, self-cohesion and background relaxation as the two primitives beneath everything else.

- `/foundations/perspectives` — Who is asking. Perspectives as trajectories not points; role-perspective vs. individual-perspective; access vs. expertise; weak observer-relativity.

- `/foundations/translation` — Translation is irreducible. Bridges, scars at translation points, knowledge as relationship. This page is the natural close of the model tier because it is where the model commits to *what the system will not do* — produce a canonical, neutral view.

**Rationale for splitting into six pages rather than one:** The source document is ~3000 words and rewards re-reading. Six pages let a reader stop at the level they need (someone interested in the forgetting mechanics doesn't need to scroll past perspectives), and let later pages link in at the right level of detail. The split follows the source document's own section breaks, so no editorial reshaping is needed at this stage.

**Open question deferred to Pete:**
> Q: Should `/foundations` include the source document's "Open Questions about the Model Itself" section, or is that internal-facing? See *Editorial questions surfaced* below.

---

## Tier 3 — `/primitives` Schema reference (framing-schema layer)

**Source:** *A Framing Schema for the Working Context Model*

**Status:** pending decision. This tier may not belong in the public docs in this first pass.

**If included:** a short reference surface — entity types, event structure, topic taxonomy, resolved stances. The reference is in tabular form in the source, and that form is appropriate as it is. The "Intentionally Open" and "Pending Opinionated Stance" lists are the editorial decision; see questions below.

**Rationale for the deferral:** The framing schema is the most internal-facing of the three documents. It is written for a reader who has internalized the model and wants to see its structural codification. A public-docs reader who has not done that work will read the schema's tabular form as a system spec — which it is not. The schema is closer to a working reference than a piece of public documentation.

**Cleanest first-pass disposition:** keep it as `docs/research/` or `/internals/` (or simply not on the public site yet) until the foundations tier has stabilized and we have a clearer sense of who reaches for the schema.

---

## Tier 4 — `/using-temper` How to use it (operational layer)

**Status:** pending decision.

**Current state:** the existing `/docs` page is a comprehensive CLI reference (install, commands, sync, MCP, etc.). It is operationally complete and useful. It is also conceptually orthogonal to the new framing — it describes a tool, not a model.

**Three options for Pete's call:**

1. **Total redirect.** The conceptual frame is sufficient on its own for this pass. CLI reference moves to a `README` or to `docs/guides/` in the repo, not on the public site.

2. **Deep-link retention.** The existing `/docs` content is retained at `/using-temper` (or similar) and linked from the foundations tier only at the points where it makes sense (e.g., the deformation page might link to `temper resource delete` as the system's instantiation of folding-vs-deformation; the perspectives page might link to contexts and profiles).

3. **Scoped subset.** Only the install + first-vault content survives publicly. The full CLI reference moves to the repo. This is the minimum that lets a reader who has decided to try Temper actually do so.

**Recommendation, weakly held:** Option 2 (deep-link retention) preserves the most existing work while keeping the conceptual frame primary. Option 3 is the most attention-economic but loses operational value.

---

## Editorial questions surfaced (not resolved)

These are the open questions the prompt instructed me to surface rather than answer. Each is a decision that affects the IA above.

1. **Operational content in this first pass.** Should the docs site have any "how to use Temper" content at all in this pass, or should it be purely conceptual until the conceptual frame is established? This determines whether Tier 4 exists.

2. **Naming what is not settled.** The source documents are honest about what they have not pinned down — "Open Questions about the Model Itself" in the semantic model, "Pending Opinionated Stance" in the framing schema. How explicitly should public docs name this? Options range from "not at all in this first pass" through "footnote on the relevant pages" through "a dedicated `/foundations/open-questions` page that lists them in the source documents' own words."

3. **Schema material as public docs.** Should the framing schema's "Intentionally Open" and "Pending Opinionated Stance" lists be exposed publicly, or held as internal-design references only? Related to Q2 but distinct — the framing schema is structurally more advanced material than the model's open questions, and public exposure invites different scrutiny.

4. **Audience and tone.** What is the right tone for the primary audience? Three plausible primaries:
   - Someone who has just heard about Temper (newcomer, no model)
   - A technical reader (engineer or researcher considering the model on its own merits)
   - Someone considering self-hosting (operationally minded; cares about substrate commitments)

   These are not the same reader and a single tone will privilege one. The current landing privileges (1) implicitly. The new framing tilts toward (2) but does not have to.

5. **Framing-schema material in this first pass.** Is there a place in the IA for the framing-schema material at all in this first pass, or is it too internal-facing? This is the Tier 3 disposition. My instinct is "not in this first pass" but it is not my call.

6. **Existing get-started content.** Should any of the existing "get started" content be retained as a deep-link from a later page, or is the redirect total? This is the Tier 4 disposition.

7. **Layout shell, navigation, breadcrumbs.** The current site has top-level pages (`/agents`, `/builders`, `/how-it-works`, `/docs`). I have not proposed touching those in this pass. If the new framing replaces the landing, those pages still exist and will pull against the new frame. Whether to leave them, redirect them, or rewrite them is downstream of the landing.

---

## What this proposal deliberately does not do

- **Does not write the foundations pages.** The brief was one draft page; this proposal does not preempt six.
- **Does not specify component structure or Svelte routing details.** Those follow from the IA, not the other way around.
- **Does not propose visual design changes.** The current site has an established register (font choices, palette, layout). The new frame can inhabit that register without redesigning it.
- **Does not propose changes to `/agents`, `/builders`, `/how-it-works`.** Those are out of scope until the landing is settled.
- **Does not commit the existing `/docs` page to deletion.** That is one of the open questions.

---

## What I'd want to see resolved before drafting more pages

In rough order of impact:

1. Q1 (operational content in this pass).
2. Q4 (primary audience).
3. Q5 (Tier 3 disposition).
4. Q2 + Q3 (handling of unsettled material).
5. Q6 (existing get-started fate).
6. Q7 (other public pages).

Resolving 1–3 unblocks roughly the whole tree. The rest are local decisions.
