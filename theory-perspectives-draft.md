# Perspectives draft — `/theory/perspectives`

First-pass draft for the perspectives page. Covers position, trajectory, characterization, role vs. individual, access vs. expertise, weak observer-relativity.

**Target length:** ~800 words. Densest source section; corresponding page is the longest.

---

## Page copy (draft)

---

# Perspectives

The model's account of *who is asking* mirrors its account of *what is asked about*. Perspectives are themselves on the manifold; they have positions, trajectories, characteristics, and visibility constraints that parallel the data side.

## Perspectives on the manifold

A perspective-point is not external to the topology — it has a position. It is a point capable of emitting intention-vectors, but it is in the same space as everything else. The same mechanics that govern resources govern perspectives: they have positions, they are affected by fields, they decay if not reinforced, they can be deformed.

This is the structural symmetry the model leans on: a perspective is not a meta-thing outside the substrate looking in. It is a thing in the substrate, with the same primitives.

## Perspectives are trajectories, not points

A given individual's perspective changes as they engage with the manifold. The perspective on a region today is the integral of past engagement with it; it is not a fixed identity but a moving locus. Perspective has the same temporal substrate as everything else — derived from event history, computed at projection time. Two queries from "the same person" at different times come from slightly different perspective-points.

## Perspective characterization

A perspective-point has at least:

- **Identity** — who or what this perspective is.
- **Reliability profile** — the prior on intentions from this perspective producing accurate information.
- **Characteristic intention-vectors** — what kinds of intentions this perspective typically emits, with what magnitudes.
- **Domain-specificity** — reliability is not uniform; it has a *spatial profile* over the manifold, with high-confidence regions and low-confidence ones.

Domain-specificity matters most. A single perspective has different reliability in different regions of the manifold. An expert in distributed systems is a high-magnitude perspective in that region and a low-magnitude one in cell biology. Perspective itself has spatial structure — much like fields do.

## Role-perspective and individual-perspective

There are two related but distinct kinds of perspective the model represents.

A **role-perspective** is a characterizable perspective-class: a kind of perspective characterized by its concerns, accountabilities, characteristic intention-vectors, and characteristic field-set. Examples: "ops department head," "product manager," "technical lead," "cloud-agent session working a ticket." A role is a specification of expected patterns.

An **individual-perspective** is a specific perspective-trajectory that may instantiate one or more role-classes at different times. A particular person inhabits a role; an agent session occupies a role; over time, both develop their own trajectory within and across roles.

The two relate the way priors and likelihoods relate in Bayesian inference. The role-persona is a *prior* on the kinds of intention-vectors this perspective is likely to emit. The individual's trajectory provides *likelihoods* — actual evidence of what queries this particular instantiator makes. The working model of a perspective at any given time is the *posterior*: the role-prior updated by accumulated individual evidence. New individuals in a role inherit the prior (low cold-start cost); over time, their actual trajectory updates the model for them specifically — without ever fully discarding the role-persona that anchors what kind of perspective this is at all.

Role-changes are *strong deformations* of the perspective-point's position — discrete, intentional, performed with awareness, substantially shifting the perspective's characteristic intention-vectors. Individual evolution within a role is *trajectory accumulation* — continuous, integrated against decay rates. The two operate at different temporal scales but with the same primitives that govern the rest of the manifold ([deformation](/theory/deformation)).

## Visibility: access and expertise

A perspective's available manifold is bounded by two distinct mechanics.

**Access** is hard, mechanical, topological. Certain regions of the manifold are not present from a given perspective — folded out of reach by RBAC, encryption, organizational compartmentalization. Access produces perspective-specific topology in the weak observer-relativity sense: each perspective has its own *visible manifold*, a sub-region of the underlying substrate. Access can be enforced mechanically and reasoned about formally.

**Expertise** is soft and resolutional. Data may be present and accessible, but the perspective lacks the *resolution apparatus* to read it as information. A novice and an expert see the same data and produce different information from it. Expertise connects directly to domain-specificity: it is the perspective's resolving power over different regions of the manifold, varying spatially.

Expertise is itself a function of perspective-trajectory. It accumulates through engagement; it can in principle be computed from event history by integrating past engagement against decay rates. Expertise that isn't reinforced fades. This gives a non-magical account of why expertise has the shape it has at any given time.

Access creates *what is in the manifold for this perspective*. Expertise creates *what can be made into information from what is in the manifold*. Both bound knowledge production, but at different layers.

## Observer-relativity (weak version)

The model adopts the *weak* version of observer-relativity. A strong version — that each observer has a fully private manifold and there is no global geometry — may be true in some deep information-theoretic sense, but is not actionable. Designing as if it were true makes collaboration incoherent.

The weak version: there is a shared substrate of events, but each observer's projection includes their own accumulated deformation history. Two observers who have engaged with the same regions in similar ways will converge on similar projections; observers with divergent histories will project differently even given identical substrate. Shared understanding is therefore an emergent property of convergent projection histories, not something guaranteed by shared substrate.

The mechanics of how perspectives bring their information shapes into productive contact — and what fails when they cannot — live on [the translation page](/theory/translation).

---

## Editorial notes

- This is the longest theory page because the source section is the longest and densest. Cutting further would lose substance.
- The Bayesian prior/likelihood/posterior framing for role vs. individual stays as the source has it. The audience can handle it; replacing it with looser phrasing would weaken the claim.
- Granularity of perspective (individual vs. team vs. organization) is deliberately not resolved here — that's an item on [open-questions#model](/theory/open-questions#model).
