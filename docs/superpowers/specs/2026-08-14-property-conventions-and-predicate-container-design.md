# Property conventions and the predicate container — where a key's convention lives

**Status:** design, session 2026-08-14. Ships nothing by itself. It resolves the fork
[the follow-from mechanic design](./2026-08-14-follow-from-mechanic-design.md) §7 left OPEN, and it
is the thing that unblocks both halves of that fork.

**Tasks:** [EdgeFilter grows property predicates — and properties(subject: edge) gets one
home](./01a000c2-033c-7451-8b13-b7aa7469d217) ·
[follow-from's mechanic](./01a00163-c0bb-7651-909f-73e3f33d8a46) ·
[find-resources-with: property selection becomes an act](./01a0003c-7468-7b31-b6b3-81913b78d150)
(shipped, and the thing this completes).

**Contract authority:** §12 of
[the compositional design](./2026-08-05-query-builder-compositional-design.md) — *"Properties are
queryable — open keys, closed operators."* This document does not re-open §12; it answers a question
§12 did not ask.

## Provenance discipline

- **`[verified — 2026-08-14]`** — read first-hand against the working tree at `98071084`, quoted
  with `file:line`.
- **`[measured on prod — 2026-08-14]`** — a read-only query against `temper-cloud` (Neon, PG17).
- **`[decided — 2026-08-14, Pete]`** — ruled in session.

---

## 1. The question changed twice, and both changes matter

It started as *"one spelling or two"* — `find-resources-with` has seven named narrowing fields, and
an open-key `PropertyPredicate` mechanism would express some of them again. Two spellings drift.

**First change.** The named fields are not redundant spellings. They are **convention-bearing** —
each encodes how *that key's* values are shaped and compared — while an open-key predicate is
shape-agnostic by construction. So the question became **where a key's convention lives**, because
an open-key mechanism can only subsume the named fields if the convention is somewhere it cannot
lose.

**Second change.** The write path has already answered it, for one axis, silently (§5). So this
document is substantially **writing down a decision the code already made** rather than taking a new
one.

---

## 2. What the conventions actually do — the jobs, from the registry

`open_meta.schema.json` (v2) is self-describing and is served to callers by both surfaces —
`temper resource describe-open-meta` and the MCP `describe_open_meta` tool, sharing one type *"so
the guidance can never drift between them"* `[verified — schema.rs:492-521]`. Nine recognized keys,
`additionalProperties: true`.

Sorted by what each convention **does**:

| job | keys | is this filter semantics? |
|---|---|---|
| **FTS indexing at a weight** | `keywords`@C, `tags`@C, `descriptor`@D | **No** — a projection concern, answered at a *different door* |
| **Shape declaration** | all nine | **No** — normalization, which is what a view does |
| **Ordered scalar** | `date` (`^[0-9]{4}-[0-9]{2}-[0-9]{2}$`) | **Yes — and it is the only one** |

`[decided — 2026-08-14, Pete]` **The established conventions do not encode filter semantics.** They
declare shape and search membership. The single exception is `date`, whose ISO-8601 pattern exists
precisely so lexicographic comparison works — and that is a missing **operator**, not a per-key
convention. It is named in §8 and not solved here.

**A structural fact worth carrying forward: five of the nine recognized keys are soft
relationships** — `relates_to`, `derived_from`, `preceded_by`, `references`, `depends_on` — and the
schema says of the first, *"Parallel to the hard edge model."* The same relationship vocabulary
already exists twice: soft in `kb_properties`, hard in `kb_edges`.

### 2.1 What the corpus is actually shaped like

`[measured on prod — 2026-08-14]` `kb_properties WHERE NOT is_folded`: **16,629 rows on
`kb_resources` over 70 distinct keys**, 37 on `kb_content_blocks`, **0 on `kb_edges`**.

Three value shapes, and **the three named fields are one instance each**:

- **string** — `doc_type` (3,743), and twelve more at n≥88: `temper-provenance`, `temper-stage`,
  `temper-effort`, `temper-mode`, `date`, `temper-llm-run`, `temper-llm-model`, `descriptor`,
  `status`, `verified`, `source_file`, `temper-status`, `temper-branch`.
- **array** — `tags` (428), and five more: `relates_to` (419), `keywords` (153), `derived_from`,
  `preceded_by`, `references`.
- **object** — `facet` (1,281) and `enables` (37).

**Type instability is real and rare**: three keys carry two shapes — `derived_from` (21 string /
112 array), `preceded_by` (1 / 46), `temper-pr` (68 string / 7 numeric). That is the population
`PropertyOp::Contains`' asymmetry note is about `[verified — filter.rs:133-151]`.

**`doc_type` is structurally unremarkable.** It is the largest string key and nothing more. That is
the empirical form of *"our current cases are just special cases"* `[decided — 2026-08-14, Pete]`.

### 2.2 A live consequence: `ResourceFilter.status` does not mean `status`

`stage` and `status` read `kb_resource_workflow_props`, whose key names are `temper-stage` and
`temper-status` — *"the key names live in ONE place and this is not it"*
`[verified — 20260814000010:134-143]`. But a plain `status` key also exists, with **337 live rows**,
and no filter reaches it. Under an open-key mechanism the two become distinguishable, which is a
gain rather than a migration.

---

## 3. The refuted hypothesis, recorded because it was nearly built on

**Claim (mine, before reading the bodies): `PropertyOp::Contains` already expresses `doc_type`,
`tags` and `facet`, so the named fields are pure redundancy.**

**False, and it fails on `tags`** `[verified — 20260814000010:133-213]`:

- `doc_type`, `stage`, `status` — **view-mediated**, not raw `kb_properties` reads.
- `facet` — inner-key object containment, `property_value @> jsonb_build_object(…)`. Genuinely a
  `Contains` special case.
- **`tags` is not containment at all.** It `lower()`s both sides, and **whitespace-splits a bare
  string**, deliberately — *"because that is what the same value already means to the FTS half of
  the system… A filter that answered otherwise would make two doors disagree about one value."*

A plain containment probe would silently stop matching case-variant and bare-string tags. The
equivalence was reasoned, not executed, and the source refuted it — GD-2's *"executing a claim is
not the same as validating it"*, in the version where nothing was executed at all.

---

## 4. The `tags` convention is already in three places, and they already differ

| site | bare string becomes |
|---|---|
| FTS projection `[verified — 20260711000060:52-62]` | **passed through whole**; splitting delegated to the text-search parser |
| `find_resources_with` `[verified — 20260814000010:181-189]` | `regexp_split_to_array(trim(…), '\s+')`, plus `lower()` |
| `filtered_visible_page` | a third copy — *"carries the identical defect and is fixed in the same change"* |

Plus the rebuild-gate key set `('keywords','descriptor','tags')` restated in two migrations
`[verified — 20260711000060:105, 20260730000010:216]`.

The first two agree in **intent** and differ in **mechanism**, which is the divergence duplication
always produces. `[measured on prod — 2026-08-14]`:

```
to_tsvector('english','ci-auth deploy')  ->  'auth':3 'ci':2 'ci-auth':1 'deploy':4
regexp_split_to_array('ci-auth deploy',…) ->  {ci-auth, deploy}
```

A bare-string tag `"ci-auth deploy"` would be **findable by searching `ci`** and **not matchable by
filtering for `ci`** — the exact disagreement the split exists to prevent.

**It is latent, not live: zero bare-string tags exist in prod today**
`[measured on prod — 2026-08-14]`. Stated in that direction on purpose. The polarity measurement in
the sibling design (§4.2) *made* its case; this one **withholds** it, and a finding that reports
only the direction it likes is not a measurement.

---

## 5. The decisive finding — the write path has already made the split

`[verified — 20260730000010]` One migration contains both gates, and they are scoped differently:

| axis | gate | scope |
|---|---|---|
| **shape / grain** | `IF v_key = 'facet'` (`:195`), with `v_owner_tbl` read from the payload and used verbatim (`:198-200`), fold scoped to `owner_table = v_owner_tbl` (`:133-140`) | **owner-agnostic** |
| **FTS membership** | `IF v_owner_tbl = 'kb_resources' AND v_key IN ('keywords','descriptor','tags')` (`:216`) | **resources only** |

**So an edge-owned facet already receives the inner-key grain, and nobody decided that.** The
edge-property write path shipped three days earlier `[verified — 20260727000030]` against a schema
whose DDL comment had said *"§4a edges carry facets"* since the beginning, with prod at
`kb_edges 0`.

This is the whole argument, delivered by the code rather than by reasoning: **the system already
treats shape conventions as owner-agnostic and projection conventions as owner-scoped.** The two
jobs §2 separates are separated in the projector.

### 5.1 The rule this establishes — convergence by declaration

`[decided — 2026-08-14, Pete]`

> **Binding edge properties to the same shapes as resource properties is not a speculative seam. It
> is writing down a standard the write path already implements, before the undeclared half drifts —
> and because the tier stays `additionalProperties: true`, it tells callers *how to use them* rather
> than preventing anything. Attention-preservation, not foreclosure.**

Recorded as a rule because it **overrode** a correctly-quoted but wrongly-applied one. *"Do not
template a file that does not diverge — a seam with nothing on either side of it is cost without
benefit"* guards against templating **divergence that does not exist**. This is the opposite move:
convergence stated ahead of use, so a standard never has to be reverse-engineered from whatever
people happened to write. §5's finding is the evidence that the reverse-engineering problem is
already beginning.

---

## 6. The design

### 6.1 One registry, two axes — structurally, not in prose

`open_meta.schema.json` today distinguishes the two axes **in each key's `description` sentence**:
*"FTS-indexed at weight C (convention v2)"* versus *"Shape-convention (not FTS-indexed)."* A test
already pins the indexed set against drift `[verified — schema.rs:709-725]`, which is the tell that
the distinction is load-bearing and is being carried by prose.

Making it structural is what lets **one registry serve both owners honestly**: shape applies to
every owner, indexing applies to resources. An edge carrying `tags` gets the shape convention and no
FTS, and a caller reading the registry can see that rather than discover it.

### 6.2 A shape convention lives in a view, and the view is owner-agnostic

A normalizing relation over `kb_properties`, keyed by **owner table, owner id, key, and element**,
is what a predicate reads instead of the table. It serves both owners because the shapes are the
same shapes, which is why there is no edge-side seam standing empty.

**Views rather than predicate functions, and the reason is measured, not stylistic:** *"a
`LANGUAGE sql STABLE` predicate whose body contains a sublink does not inline: measured, the
`doc_type` EXISTS loses its Index Only Scan on `uq_kb_properties_active` and becomes a per-row call,
while the view form is plan-identical to the incumbent"* `[carried — 20260808000020:308]`. The
established view set is already `kb_resource_doc_type` and `kb_resource_workflow_props`; this
finishes the pattern rather than introducing one.

### 6.3 The rule that answers the fork

> **Shape conventions live in a view, owner-agnostic. Projection conventions live in the projector,
> owner-scoped. A predicate reads the view, and therefore can neither lose a convention nor wrongly
> inherit one.**

That is what makes an open-key mechanism safe to subsume the named fields: the convention is not in
the predicate, so a shape-agnostic predicate cannot drop it.

### 6.4 What it does to `PropertySubject`

With both halves given containers — edge predicates in `EdgeFilter`, open-key resource predicates in
`ResourceFilter` — **`PropertySubject` disappears**, taking `Other(String)` and its
`UnknownFilterValue` arm with it `[verified — filter.rs:106-115]`. A subject tag exists only because
a predicate floats free of a container; give it a container and the tag has no job.

**This is the "both halves" branch of the sibling design's §7 fork, and this document is the
argument for taking it.** The branch was previously blocked on not knowing where conventions live.

### 6.5 What each half then is

- **Edge half** — shape-agnostic containment over the same view, no conventions to preserve, zero
  live rows. The easy half, and independently shippable.
- **Resource half** — the convention work: `tags` (and `facet`) join the view pattern that
  `doc_type`/`stage`/`status` already use, collapsing §4's three encodings into one and closing the
  latent divergence at the same time.

---

## 7. What remained open about `tags` specifically — RULED 2026-08-15

`tags`' whitespace-split is the sole hold-out from a **universal** normalization (explode arrays,
pass scalars), which would serve the other eight recognized keys and the 61 unrecognized ones
unchanged.

The split exists only to agree with FTS — and **FTS does not split; it delegates to the tokenizer**
(§4). So the behaviour the split imitates is not the behaviour it produces. That makes it look less
like a convention worth centralizing and more like a **write-time normalization living in the read
path**, duplicated across readers because it was never given a home.

`[WAS OPEN — 2026-08-14]` Whether bare-string `tags` should be normalized at write, at the view, or
left per-reader. Not ruled. The zero-live-instances fact (§4) means nothing is currently wrong, and
the decision is cheap in every direction today.

`[RULED — 2026-08-15, Pete]` **At write, and it is ONE tag.** `tags: "concept design"` stores as
`["concept design"]`. Shipped in `20260815000030`: `_property_value_normalized`, called by both
projectors, with the ruling recorded on the migration and on `kb_property_elements`' `COMMENT`
rather than implied by them.

Three things followed, and the third was not foreseen here:

- **The view became universal.** No `tags` branch — explode arrays, pass scalars — which is the
  normalization this section said the split was the sole hold-out from. One rule for all 70 keys.
- **The split's justification was measurably false**, which is what decided it. It existed to agree
  with FTS, and the paragraph above already says FTS does not split; what that paragraph stopped
  short of is the conclusion — a mechanism that does not achieve its own goal is not a convention
  worth centralizing. So the split was **deleted rather than moved**: three encodings collapse to
  none, because there is nothing left to encode.
- **The read-side split could not be held accountable by a behavioural test** `[found — 2026-08-15]`.
  Once nothing can STORE a bare string the branch is unreachable, so a witness for the new behaviour
  passes with or without it. What holds it is a probe that writes the shape the projector can no
  longer produce — the row a deployment predating the migration already holds. Recorded because the
  same asymmetry will recur for every convention moved from read time to write time.

**The residue is real and was accepted, not discovered:** someone who wrote `tags: "ci auth"`
meaning two tags now has one. Zero such rows exist `[measured on prod — 2026-08-15]`, so it changes
nothing today, and the literal reading is the one a caller can predict from what they wrote.
`open_meta.schema.json` continues to accept both shapes — this rules what a bare string MEANS, not
whether it is legal.

---

## 8. The operator gap — `date`, and ordering

`PropertyOp` is `HasKey` and `Contains` `[verified — filter.rs:117-151]`. Neither expresses a range,
so *"resources dated after 2026-07-01"* is inexpressible against a key whose convention
(`^\d{4}-\d{2}-\d{2}$`) exists **so that lexicographic comparison is correct**.

Named, not solved. It is an **operator** question — §12's *"open keys, closed operators"* with the
closed set being one member short — and it is orthogonal to where predicates live, so settling it
here would bundle two decisions that can be taken apart.

---

## 9. Declared holes and what is not measured

- ~~**§7 is OPEN**~~ **§7 is RULED and shipped** `[2026-08-15, 20260815000030]` — see §7. **§8 is
  named-not-solved** and is still not a filed task.
- ~~**The open-key resource half still has no filed task.**~~ It is task
  `01a00502-a774-7001-b5b2-0ce462158f1c` `[filed — 2026-08-15]`, whose first PR is
  `20260815000030` (this section's §7 ruling and the view). **67 of the 70 live property keys remain
  unreachable by any narrowing on any act** — `doc_type`, `tags` and `facet` are the three that are,
  and that is unchanged by the ruling: the view is the *relation* the open-key predicate will read,
  not the predicate. That is the task's second PR.
- **No edge-side behaviour is exercised.** Zero edge-owned properties exist, so §5's owner-agnostic
  grain is correct-by-construction and **witnessed by nothing**. A witness is part of the build, not
  of this document.
- ~~**No cost measurement for the view.**~~ **Measured** `[on prod — 2026-08-15]`. Against the real
  corpus the exploding form and the incumbent expression read an identical **26,970 blocks per
  call**, at 34.17 ms versus 34.71 ms mean over three calls each (σ 2.7 / 2.1) — a difference inside
  one standard deviation of either. So the shape costs what the thing it replaces cost, on its own
  number rather than on `20260808000020`'s.

  Two limits on that measurement, stated rather than left to be assumed. It was taken against the
  predicate **inlined**, because the migration is not on prod yet; a view is expanded into the query
  tree by the rewriter, so the inline form is the faithful stand-in, but it is a stand-in. And
  26,970 blocks is dominated by `resources_visible_to`, not by the tag predicate — this measures
  that the new shape adds nothing, not what the read costs overall.

  `pg_stat_statements` **is installed and collecting on prod** (616 statements): `20260814000020`
  applied. The repeated claim that it is unavailable is stale as of 2026-08-15.
- **`MAX_FACET_PREDICATES` moves with the answer.** The cap exists because a facet predicate is a
  per-row `NOT EXISTS` and *"an authenticated caller chooses the second factor"*
  `[carried — capability.rs]`. Open-key predicates have the identical cost shape, so the cap becomes
  theirs. Not designed here.
- **The element relation cannot answer `has_key`, and that is a hole the open-key half must not
  fall into** `[found while building — 2026-08-15]`. An empty array explodes to **no rows**, so a
  resource carrying `tags: []` is indistinguishable in `kb_property_elements` from one carrying no
  `tags` row at all. Eleven such rows exist on prod `[measured — 2026-08-15]`. A `PropertyOp::HasKey`
  predicate must therefore read `kb_properties` directly — which is what its own doc already says
  for a different reason (*"a row-existence check on the `property_key` btree"*), so the two
  operators of one closed set legitimately read two different relations. Recorded on the view's
  `COMMENT`, because the predicate that would get this wrong is not written yet.
- **`kb_edge_properties` deliberately does NOT converge onto the element relation** `[2026-08-15]`.
  §6.5 called the edge half containment *"over the same view"*, and that is now the one part of this
  design not taken literally: the edge predicate needs `property_value` **whole**, because
  containment over an exploded element is a different question (`'["a"]'::jsonb @> '["a"]'` is true;
  `'"a"'::jsonb @> '["a"]'` is false). Converging it would be a behaviour change to a shipped
  predicate, not a refactor. Two relations over one table, each with a stated reason — not drift,
  but the next reader will have to be told that, which is why it is here.
