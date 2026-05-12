# Open questions draft — `/theory/open-questions`

First-pass draft for the consolidated open-questions page. Two anchored sections: `#model` (from the semantic-model doc) and `#schema` (intentionally-open + pending-opinionated-stance from the framing-schema doc).

**Target length:** ~700 words.

---

## Page copy (draft)

---

# Open questions

This page gathers the questions the model and its schema have deliberately not resolved. Items move off this page as they resolve, with the resolution landing in the appropriate `/theory` or `/theory/schema` section.

The two sections below correspond to the two source documents. The boundary between them is real but soft — a question about field sub-typing in the model corresponds to a question about field-class typing in the schema; resolving one tends to resolve the other.

## #model — open questions about the model itself {#model}

These are not engineering questions. They concern whether the model itself is complete and coherent.

- **Is *field* too undifferentiated?** Goals, decisions-in-force, constraints, and tolerances all behave somewhat differently as fields. They may be subspecies sharing mechanics, or they may genuinely be a single primitive at this level.
- **Is retroactive correction its own primitive, or fully captured by deformation-with-scar?** Currently treated as the latter. The answer may depend on whether retroactive corrections need different relaxation behavior than forward corrections.
- **What is the relationship between projection and resource?** A document is in some sense a projection of past stream activity, written down. A search result is a projection of current field configuration. The asymmetry is suggestive but not yet pinned down.
- **Are manifolds composable?** Cross-project, cross-team, cross-owner reference is an open question. The model as stated allows for it but does not prescribe the composition mechanic.
- **How do scars themselves decay?** A scar near a region that hasn't been engaged in a long time should presumably fade with the region. But heavy scars (catastrophic past errors) may resist decay. The mechanic is not specified.
- **What is the right granularity of perspective?** A single individual is one perspective; a team is arguably another; an organization yet another. Whether perspectives compose hierarchically, federatively, or both, is open.
- **How are role-personas themselves authored and evolved?** They are priors used for cold-start; how they originate and update at the role-class level (rather than the individual level) is not in the model.
- **What is the model's account of trust?** If knowledge isn't stored, then trust isn't trust-in-information; it's trust-in-perspective-to-produce-reliable-information-from-data-in-region. The shape of this is implied but not yet developed.

## #schema — schema-level unsettled material {#schema}

The framing schema distinguishes two flavors of openness. Both live here.

### Intentionally open (downstream design)

These are deliberately not specified by the model; they are answered by particular system designs.

- Storage substrate (single-node, distributed, ledger-flavored, CRDT — events-as-primary required, particular implementation not).
- Specific implementation of as-of queries and trajectory projection.
- Query language and retrieval mechanics.
- Specific aboutness computation.
- Field declaration, weighting, and evolution mechanics.
- Authority model for who may emit which topic-classes.
- Persona/role library specifics for any organization.
- Whether manifold is global, per-owner, per-project, or composable across scopes.
- Scar-informed engagement policy (source-agnostic at model level, source-aware at policy level).
- Topic taxonomy management — how new topics are introduced and payload-schemas evolve.
- Visual/interaction surface design.

### Pending opinionated stance

Gaps the schema needs to close before it is fully coherent. Each wants pressure before stabilizing.

- Minimum-viable core schema for state-change events (what is the irreducible structure of a role-change, position-update, or membership-change event?).
- Whether on-behalf-of is single-valued or multi-valued (instinct: multi-valued).
- Whether on-behalf-of chains nest as a property on the emission or as a separate graph between aggregates (instinct: separate graph).
- Whether authority always grounds out at a discrete human/organizational entity (instinct: yes).
- How scars themselves decay; do heavy historical scars resist decay differently? *(mirrors #model.)*
- Whether manifolds compose hierarchically, federatively, or both. *(mirrors #model.)*
- How role-personas at the class level originate and update. *(mirrors #model.)*
- Caching tradeoffs — when, if ever, derived structure must be maintained rather than computed, and what reconciliation looks like.
- Whether the field primitive needs sub-typing (goals vs. decisions-in-force vs. constraints vs. tolerances). *(mirrors #model.)*
- Retroactive correction as its own primitive vs. fully captured by deformation-with-scar. *(mirrors #model.)*
- The right granularity for attribution-judgment payload.
- The mechanic for practice-emergent aggregate-perspectives (those without a discrete forming-event).
- Aggregates-of-aggregates: different in kind from first-order aggregates, or only in degree?

## A note on this page

This page exists for two reasons. First, the manifesto's commitment to making perspective-differences visible applies recursively to the docs themselves — a documentation surface that hides what is unsettled forces attention to be spent re-discovering the gaps. Second, the items here are work-in-progress. Some will resolve into the model and schema pages. Some will move from "intentionally open" to "pending opinionated stance" or vice versa. Some will turn out to dissolve under closer reading rather than resolve.

The list will be wrong in different ways over time. That is the point.

---

## Editorial notes

- Anchors (`{#model}` and `{#schema}`) are explicit so cross-links from the schema page resolve correctly. Implementation detail at SvelteKit time — the syntax may need to change depending on the markdown renderer.
- Items that appear under both #model and #schema are marked `(mirrors #model.)` rather than duplicated in full. Resolving the schema-level version typically resolves the model-level version and vice versa.
- The closing note is honest about the page's own provisionality. Symmetric with the schema's WIP framing.
