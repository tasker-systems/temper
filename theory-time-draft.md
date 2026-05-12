# Time draft — `/theory/time`

First-pass draft for the time page. Establishes time as a primary axis and derives the events-as-primary commitment from the bidirectional coupling argument.

**Target length:** ~400 words.

---

## Page copy (draft)

---

# Time

Time is not an afterthought; it is co-equal with position. Every element of the model is temporally extended.

## What that means concretely

- A field's spatial profile drifts as the work it represents shifts.
- A field's weight rises and falls as concerns gain and lose currency.
- A stream is constitutively temporal — there is no stream without time.
- A projection has a *temporal lens*: it can be taken at the present moment, at a prior moment, or integrated over a window.
- Resources have temporal validity — they are true with respect to a field configuration that may have since changed.
- Perspectives are themselves trajectories rather than points ([perspectives](/theory/perspectives) returns to this).

## Why this forces events as the substrate

[The manifold page](/theory/manifold) commits to bidirectional coupling: streams shape the manifold they flow through; the manifold's geometry is constituted by stream history. That commitment, taken seriously, forces a substantive substrate decision.

If the manifold's geometry is a function of stream history, then stream history is the primary data and the manifold is derived. Event-sourcing is therefore not one substrate option among many; it is the only substrate consistent with the model. Distributed, ledger-flavored, single-node, CRDT-flavored — all remain viable implementations. What is load-bearing is *events-as-primary*: geometry is computed against history rather than maintained as a separate snapshot.

The framing schema captures this as a resolved stance: *append-only ledger; cross-cutting point-in-time truth*. Anything anyone needs to know about any entity at any time — its position, its role, its membership, its trajectory — is derivable from events. State is not stored separately from the event history that produced it.

## A consequence for retrieval

Queries against the substrate are inherently temporal. "What is the case" is shorthand for "what is the case as of this moment, integrated over the visible event history, from this perspective." A retrieval surface that does not honor this conflates *now* with *forever* and pretends the integration window is unbounded.

This does not mean every query has to be expensive. The model commits to events-as-primary; it does not commit to recomputing every projection from scratch on every read. Caching, indexing, and materialization tradeoffs belong to system design, not to the model. The model's commitment is to what the derived structures answer to: they may be cached, but the event history is canonical and the cache is not.

Which derived structures must be maintained rather than computed, and what reconciliation looks like when they are — that is on [the open-questions page](/theory/open-questions#schema).

---

## Editorial notes

- This page is short by design. The substantive commitment (events-as-primary) is short to state but load-bearing; padding it would obscure it.
- The page does not get into specific implementations. That belongs at `/using-temper` and below; here we only commit to the substrate property.
- The forward link to open-questions is deliberate — the caching/materialization question is genuinely open and the page should not pretend otherwise.
