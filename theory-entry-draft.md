# `/theory` entry draft

First-pass draft for the `/theory` entry page. Merges what were `landing-draft.md` and `theory-overview-draft.md` into a single page: the values frame (manifesto), one upfront commitment about what is stored, and the shape of the model with sub-page index.

**Target length:** ~800 words. Longer than a typical entry page; justified because the entry to the tier is doing the work of orienting a reader to *both* the why and the what.

**Important framing:** this is **not** the temperkb.io landing. The existing landing at `/` is unchanged. `/theory` sits alongside it for readers who have opted into "tell me what this is for, at the level the work is thinking about."

---

## Page copy (draft)

---

# What Temper is building toward

Attention is the most precious thing a working perspective brings to its work. It is finite, costly, and does not regenerate. When it is spent — on re-discovering what a system could have made present, on reconstructing context that has scattered, on relitigating decisions whose history has been lost — it is spent for good.

The pages under `/theory` describe what working context *is* once you take that seriously, and what kind of system can serve it. This is a semantic specification, not an engineering brief. It does not describe what Temper-as-currently-built does. It describes the direction Temper is building toward — what working context is, what the model commits to, and what kind of substrate properties a system serving it has to honor.

## Why this matters

If attention is the medium through which any of us are present to our work, then a system that wastes attention is treating something precious as fungible. The design of an information system is not value-neutral; it inherits the obligation that follows from what attention actually is.

Four commitments fall out of taking that obligation seriously. They are not features. They are the standard the rest of the work is held to.

### Common queries should not require fresh attention each time

Most people in a given role ask roughly the same kinds of questions, with the same kinds of intent behind them. Making those queries cheap by default is not paternalism. It is a system doing work that does not need to be done freshly each time, so attention can land on what is actually new.

### Perspective-differences are real and should be made visible

Different people working on the same thing produce genuinely different information from the same data, because they engage it from different positions with different concerns. A system that pretends otherwise — that produces a single canonical view — forces attention to be spent re-discovering those differences in every conversation. Surfacing them is how a system spends attention once and saves it forever.

### Information past its time should fade

Information that is no longer relevant should grow harder to find. Information that is no longer true should not surface as if it were. Information that needs to be preserved for audit should not pollute default retrieval. None of this is unusual to want. What is unusual is taking it seriously enough to design for, rather than letting deprecation tags pile up while everything stays equally findable.

### Where the system has been wrong, future engagement should know

Confidence and accuracy are not the same thing. Without a trace of where errors have lived before, attention cannot land on what needs scrutiny. A system that hides its own past errors is asking attention to do work the system should have done.

## One commitment worth naming up front

The model's most consequential claim is about what is *stored*. A system serving the model stores data and traces of past intentional acts — recorded questions, notes, decisions, which themselves become further data. It does not store *knowledge*. Knowledge is the relationship between a perspective and the information that perspective produces through engagement with data.

The label "knowledge base" is a misnomer in light of this. Knowledge is always potential, never actual, until activated by a perspective. A system serving the model is never trying to *be right about what something means* — meaning is not in the system. Its job is to faithfully represent data, faithfully record intentions, and faithfully compute projections such that perspectives engaging with those projections are well-equipped to produce knowledge.

Everything downstream — the manifold, the fields, the deformations, the perspectives, the translation problem — sits inside this frame.

## The shape of the model

The pages here introduce the model in the order the source document does. Each is short; each can be read alone; the sequence is the most coherent path through.

- **[Ontology](/theory/ontology)** — Data, intention, information, knowledge. The stratified layers.
- **[Manifold](/theory/manifold)** — Positions, fields, streams. The geometry.
- **[Time](/theory/time)** — Time as a primary axis. Events-as-primary. Why this is a substrate commitment.
- **[Deformation](/theory/deformation)** — Forming and forgetting. Strong vs. weak. Scarification. Self-cohesion and relaxation.
- **[Perspectives](/theory/perspectives)** — Trajectories, not points. Role-perspective vs. individual. Access vs. expertise.
- **[Translation](/theory/translation)** — Why translation is irreducible. Bridges. Knowledge as relationship.

Two reference surfaces sit alongside:

- **[Schema](/theory/schema)** — The structural codification: entity types, event structure, topic taxonomy, resolved stances. Work in progress.
- **[Open questions](/theory/open-questions)** — What is not yet settled. Updates over time as items resolve.

The model is provisional. It captures a mental picture clearly enough to be argued with. Pages link to the open-questions surface where a section's framing depends on an unresolved decision.

---

This is a commitment, not a sales pitch. It is written so that when implementation pressure tempts a shortcut that costs a reader some attention they will not get back, there is something to read that reminds the work why it started. The commitment is more honest if it is shared.

---

## Editorial notes

- **Merge:** consolidates what were `landing-draft.md` and `theory-overview-draft.md`. The four commitments now sit between the anchor and the model overview rather than constituting their own page. One scroll, one URL, one reading-arc.
- **Tense and aspirational framing.** Earlier drafts had "the system pre-pays..." in the present tense, which risked smuggling a claim that Temper-as-currently-built behaves this way. The merged copy uses "a system serving the model..." where the source manifesto used "the system" — making explicit that these are properties a system has to honor, not properties Temper has already shipped.
- **The four commitments are kept close to the manifesto's wording.** The manifesto's prose is doing real work — the rhythm of "not paternalism," "not unusual to want," "asking attention to do work the system should have done" — and re-voicing it would lose the texture. One light edit ("It is *a* system doing work" rather than "*the* system doing work") to be consistent with the aspirational tense.
- **No CTA, no install link, no second-person imperative.** The page assumes the reader has opted into the theory tier and is here to engage on those terms. A "get started" button would contradict the page's premise.
- **No mention of CLI, vault, markdown, frontmatter, agents, or doc types.** Those terms belong on `/using-temper`. The conceptual frame establishes the why and the what without leaking product vocabulary.
- **The `knowledge-bases-are-misnomers` commitment is named here and on `/theory/ontology`.** The overlap is deliberate. The entry previews; ontology lands it in its full form (with the formula). A reader who stops after the entry should still take it away.
- **The closing paragraph is a tightened version of the manifesto's own closing.** Removes the "anyone considering this tool" framing (landing-page-y; doesn't fit a tier-entry context) but keeps the commitment-not-sales-pitch register.

## Things considered and rejected (carried forward from the earlier landing draft)

- **Opening with a product-category definition.** ("Temper is a knowledge base for…") This is the conventional landing move and the source documents resist it on principle. The page opens with what the model is *for* and lets the category emerge.
- **Comparative framing against named alternatives.** Not the register. The page makes a positive case for an ethic; it does not run down alternatives.
- **A diagram of the manifold or fields.** Belongs at `/theory/manifold`. On the entry it would smuggle structural claims past the values frame.
- **A code or CLI sample.** Belongs at `/using-temper`. On the entry it would smuggle product claims past the conceptual frame.
