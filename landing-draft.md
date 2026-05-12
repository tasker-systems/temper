# Landing page draft — `/`

First-pass draft of the new top-level page. Replaces the current marketing-flavored landing with a manifesto-anchored frame.

**Status:** review draft. Not committed to the SvelteKit routes. See `docs-ia-proposal.md` for the IA this page sits inside.

**Target length:** short enough to read in one sitting without scrolling fatigue. The current draft is ~550 words of prose. This is deliberate — attention-economy applies to the docs surface itself.

**Tone notes for review:**
- No promotional verbs ("transform", "unlock", "supercharge"). No second-person imperative ("get started", "try it now").
- The page does not ask the reader to install or sign up. It points to where the model lives and stops.
- It uses "Temper" as a proper noun and avoids treating it as the subject of every sentence — the *commitments* are the subject; Temper is what tries to keep them.

---

## Page copy (draft)

---

# What Temper is for

Attention is the most precious thing a working perspective brings to its work. It is finite, costly, and does not regenerate. When it is spent — on re-discovering what a system could have made present, on reconstructing context that has scattered, on relitigating decisions whose history has been lost — it is spent for good.

Temper is built around taking that seriously.

This is not a productivity claim. It is closer to an ethical one. If attention is the medium through which any of us are present to our work, then a system that wastes attention is treating something precious as fungible. The design of an information system is not value-neutral; it inherits the obligation that follows from what attention actually is.

Four commitments fall out of taking that obligation seriously. They are not features. They are the standard the rest of the work is held to.

## Common queries should not require fresh attention each time

Most people in a given role ask roughly the same kinds of questions, with the same kinds of intent behind them. Making those queries cheap by default is not paternalism. It is the system doing work that does not need to be done freshly each time, so attention can land on what is actually new.

## Perspective-differences are real and should be made visible

Different people working on the same thing produce genuinely different information from the same data, because they engage it from different positions with different concerns. A system that pretends otherwise — that produces a single canonical view — forces attention to be spent re-discovering those differences in every conversation. Surfacing them is how a system spends attention once and saves it forever.

## Information past its time should fade

Information that is no longer relevant should grow harder to find. Information that is no longer true should not surface as if it were. Information that needs to be preserved for audit should not pollute default retrieval. None of this is unusual to want. What is unusual is taking it seriously enough to design for, rather than letting deprecation tags pile up while everything stays equally findable.

## Where the system has been wrong, future engagement should know

Confidence and accuracy are not the same thing. Without a trace of where errors have lived before, attention cannot land on what needs scrutiny. A system that hides its own past errors is asking attention to do work the system should have done.

---

These commitments are what the rest of the work answers to. They sit above the design of any particular feature, and they are the standard a feature has to meet to belong.

There is a model underneath the commitments — a description of what working context actually is, prior to any system that handles it. That model is where the commitments become precise enough to build against: how *fading* works geometrically, what *perspective* means as a substrate property, why *events* are the primary substrate and the rest is derived.

**Read the model →** `/theory`

---

This document is a commitment, not a sales pitch. It is written so that when implementation pressure tempts a shortcut that costs the reader some attention they will not get back, there is something to read that reminds the work why it started. It is published because the commitment is more honest if it is shared, and because anyone considering this tool deserves to know what its author thinks the tool is *for* before they decide whether to spend their attention on it.

---

## Editorial notes for the reviewer

A few decisions in the draft that are worth flagging explicitly rather than leaving implicit:

- **The four commitments are quoted close to the manifesto's own wording.** The manifesto's prose is doing real work — the rhythm of "not paternalism," "not unusual to want," "asking attention to do work the system should have done" — and I have not tried to tighten or re-voice it. If a re-voice is wanted, that is a substantial editorial decision and I'd want to confirm before making it.

- **No call-to-action button.** A "Get Started" or "Install" CTA would contradict the page's premise. The only outbound link is to `/foundations`, which is the next layer of the same thinking rather than a conversion target. This is a real decision and easy to override if Pete wants a softer one.

- **The closing paragraph is adapted from the manifesto's own closing paragraph.** It is the only place the page is in a first-person register, and it is borrowed wholesale from the source. If the public landing should not be in first person at all, this paragraph either rewrites in third person or comes out.

- **No mention of CLI, vault, markdown, frontmatter, agents, or doc types.** This is deliberate. The conceptual frame must be established before the product vocabulary is introduced. Those terms belong on `/theory` pages and on `/using-temper`.

- **No mention of the model's primitives (manifold, fields, deformations, perspectives, scars).** Also deliberate. The landing page's job is to establish the *why*. The model's job is to establish the *what*. Conflating them on the landing is what the current site does and is what the new frame is trying to undo.

- **Heading hierarchy.** Four H2s, one per commitment. No H3s. The page is short enough that nested structure would be overkill.

- **The page does not claim Temper implements any of the four commitments yet.** It says they are "what the rest of the work answers to." This is honest to the manifesto, which is explicit that the commitments are a standard, not a shipped state.

## Things I considered and rejected

- **Opening with a definition of "Temper."** ("Temper is a knowledge base for…") This is the conventional landing-page move and it is precisely what the manifesto rejects. A definition flattens the why into a product-category description. The page opens with what the tool is *for* and lets the category emerge.

- **A side-by-side "before/after" of conventional knowledge tools.** Tempting because it concretizes the critique, but it is comparative marketing and the source documents resist that posture.

- **Naming named alternatives (Obsidian, Notion, etc.).** Same reason. The page makes a positive case for an ethic; it does not run down the alternatives.

- **Embedding a code or CLI sample.** Belongs on `/using-temper` if it exists. On the landing it would smuggle product claims past the conceptual frame.

- **A diagram.** A manifold diagram or fields-and-deformations sketch is appealing but lives properly at `/foundations`. The landing's claim is values, not geometry.
