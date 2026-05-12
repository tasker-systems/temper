# Time draft — `/theory/time`

First-pass draft for the time page. Establishes time as a primary axis and derives the events-as-primary commitment from the bidirectional coupling argument.

**Target length:** ~400 words.

---

## Page copy (draft)

---

# Time

Time is not an afterthought; it is co-equal with position. Every element of the model is temporally extended:

- A field's spatial profile drifts as the work it represents shifts.
- A field's weight rises and falls as concerns gain and lose currency.
- A stream is constitutively temporal — there is no stream without time.
- A projection has a *temporal lens*: it can be taken at the present moment, at a prior moment, or integrated over a window.
- Resources have temporal validity — they are true with respect to a field configuration that may have since changed.
- Perspectives are themselves trajectories rather than points ([perspectives](/theory/perspectives) returns to this).

## Why this forces events as the substrate

Bidirectional coupling forces a substantive substrate commitment. If the manifold's geometry is a function of stream history, then stream history is the primary data and the manifold is derived. **Event-sourcing is not one substrate option among many; it is the only substrate consistent with the model.** Distributed, ledger-flavored, or single-node implementations all remain viable, but events-as-primary is load-bearing for the semantics. Geometry is computed against history rather than maintained as a separate snapshot.

Two consequences follow that the model commits to. First, entity-state is itself derivable from events — role-changes, position-updates, membership-changes are all emissions of state-change topics, append-only like everything else. The substrate doesn't store entity-state; it stores the events from which entity-state is computed. Second, point-in-time truth is cross-cutting: for any event at time T, the state of every entity associated with that event — emitter, on-behalf-of scopes, observers — is projectable at T from their trajectories. The substrate carries the events; everything else, including the temporal truth of who-was-where-when, is computed.

Specific implementations remain downstream of the model. Caching, indexing, and materialization tradeoffs — when derived structure must be maintained rather than computed, and what reconciliation looks like when it is — are [open](/theory/open-questions#schema).

---

## Editorial notes

- This page is short by design. The substantive commitment (events-as-primary) is short to state but load-bearing; padding it would obscure it.
- The bullet list of temporally-extended elements flows directly from the opening sentence, matching the source's structure. An earlier draft introduced a "What that means concretely" header between them — removed.
- The page does not elaborate consequences-for-retrieval beyond what the source commits to. An earlier draft had a "consequence for retrieval" section that synthesized claims about query temporality; trimmed because the synthesis went beyond what the source argues.
- **The "Two consequences follow" paragraph folds in two schema-level resolved stances:** *"Entity state is derived from state-change events, not stored as denormalized state"* and *"Cross-cutting point-in-time truth: for any event at time T, all associated entities' states at T are projectable from the trajectory of state-change events."* The semantic model commits to events-as-primary; the schema names the entity-state and cross-cutting-truth consequences explicitly. The theory page should carry both, since they answer the natural follow-up question "what about state?"
- The forward link to open-questions on caching is deliberate — the caching/materialization question is genuinely open and the page should not pretend otherwise.
