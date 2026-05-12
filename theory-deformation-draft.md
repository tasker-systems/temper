# Deformation draft — `/theory/deformation`

First-pass draft for the deformation page. Covers forming/forgetting, recording threshold, scarification, self-cohesion, relaxation.

**Target length:** ~750 words.

---

## Page copy (draft)

---

# Deformation: forming and forgetting

Deformations are the events that shape the manifold's geometry. They come in two broad classes — *forming* and *forgetting* — with several mechanics within each. Two structural primitives — *self-cohesion* and *background relaxation* — operate beneath both classes.

## Forming deformations

A deformation that adds to or alters the manifold's geometry is a forming deformation. Forming deformations exist on a continuum of strength.

**Strong / authoritative deformations** are decisions, supersessions, declared topology changes. They are discrete, deliberately performed, often with awareness of their effects. They overcome the manifold's self-cohesion and substantially bend its geometry.

**Weak / cumulative deformations** are attention, questioning, repeated investigation, sustained focus. They are continuous, mostly performed without awareness, and individually subtle. They do not overcome self-cohesion on their own, but they exert ongoing pressure that, when reinforced, can crystallize into something the system represents.

The boundary between weak and strong is the **recording threshold**: the point at which sub-recording attention crystallizes into a tracked artifact. The act of recording — writing down the question, capturing the note, opening the ticket — is itself a strong-enough deformation to track. Below the threshold, weak deformations exist and have effects, but for design purposes their effects are *integrated into* the recorded artifacts that subsequently emerge from sustained attention. The model does not pretend sub-recording attention is invisible; it accepts that the recording threshold is the natural and principled design boundary.

## Forgetting mechanics

Forgetting is not deletion or annotation; it is geometric. There are three distinct mechanics, plus a property that can attach to certain deformations.

**Decay.** A resource that no longer participates in any active field drifts away from the active region of the manifold. Its findability falls off as a function of distance and time-since-reinforcement. Decay is passive, continuous, and reversible: re-engagement reinforces position. Decay answers *no longer relevant*.

**Deformation (in the forgetting sense).** A decision, supersession, or correction can change the manifold's topology itself. The superseded region becomes locally less reachable — not because it has been labeled, but because the geometry no longer routes through it. Deformation is active, discrete, and is performed by an agent with the authority to alter topology. Deformation answers *no longer true*.

**Folding.** Some resources must be preserved for audit, reversibility, or historical understanding, but should not surface in default projection. These are folded onto a different *sheet* of the manifold: accessible by deliberate time-travel queries, invisible to default projection. Folding answers *preserved but not present*.

## Scarification

When a deformation is performed because something was *wrong* — whether through human error, agent hallucination, latent ambiguity hardening into false certainty, or any other source — it carries an additional property called a **scar**. The scar is not the corrected information; it is the *audit of the correction*: a structured memory of what was wrong, what was assumed because of it, what had to be reworked.

Future engagement with the region gets the corrected state plus the awareness that this region has been wrong before — appropriately raising scrutiny and lowering confidence in adjacent claims that may have rested on the original error.

Scarification is not a fourth forgetting mechanic; it is a property certain deformations carry. The model is deliberately source-agnostic: hallucination, misremembering, conflation, and false certainty all leave the same kind of scar. Policy of how scars *inform* engagement — an agent's hallucination region may warrant different scrutiny than a human's misremembering region — is a system-design question that builds atop the model.

The model needs to support not just *what is true now* but the *epistemic history of how regions came to be considered true*. This falls out naturally from events-as-primary ([time](/theory/time)).

## Self-cohesion and background relaxation

Two structural primitives operate beneath the deformation mechanics.

**Self-cohesion** is the manifold's structural resistance to bending under weak forces *at any given moment*. A weak deformation might not bend the manifold meaningfully even immediately, because the structural integrity of accumulated recorded geometry resists it. Self-cohesion is what makes the recording threshold meaningful: weak forces alone don't crystallize; they require either accumulation or escalation to a strong-enough event.

**Background relaxation** is the manifold's tendency to return toward an unweighted ambient state *over time* when deformations aren't reinforced. Strong deformations relax slowly or not at all; weak deformations relax quickly. Relaxation is what keeps the manifold from accumulating into an unreadable noise floor of every question ever asked.

Self-cohesion and relaxation answer different questions. Self-cohesion: *does this deformation register at all?* Relaxation: *does it stick?* Both are needed.

How scars themselves decay — whether catastrophic past errors resist decay differently from incidental ones — is [an open question](/theory/open-questions#model).

---

## Editorial notes

- The page uses *deformation* in two senses (the class of geometry-changing events, and the specific forgetting mechanic). The source document does this; readers handle it; renaming one of them would lose connective tissue with the schema.
- The scar discussion stays source-agnostic, as the source does. Policy variation (agent vs. human; catastrophic vs. incidental) belongs downstream.
- An earlier draft had an editorializing paragraph after the three forgetting mechanics ("Conflating them — letting *deprecation* stand in for all three..."). The source distinguishes the three mechanics clearly without that editorial reinforcement, and the spirit of the critique lives in the manifesto already; trimmed.
- Forward link to open-questions on the scar-decay point is honest — that's a real unresolved item.
