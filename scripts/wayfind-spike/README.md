# wayfind-spike — measurement harness for "why raw out-competes distilled"

Read-only probes behind the spike recorded in task `019fb55b-6148-7ac2-b23d-ee42d895959c`
(goal `019fb559-7191-75a3-99d4-879090c60e94`, issue
[#585](https://github.com/tasker-systems/temper/issues/585)).

**These scripts change nothing.** `prod-readonly.sh` opens every session with
`SET default_transaction_read_only = on`, so the connection cannot write even if a query were edited to
try. There are no migrations here and nothing writes to the vault.

They are committed rather than thrown away for one reason: the remedy task
(`019fbb32-c426-7903-9198-2aa869086f50`) has to **re-measure the same quantities after it changes the
blend**, and a before/after is only worth anything if both halves are the same measurement.

## Two probe families, and why they are not interchangeable

| family | what it is | what it may conclude |
|---|---|---|
| **document vectors** | real `kb_chunks.embedding` rows, chosen deterministically by `md5(id)` | **counterfactuals** — the shipped rules are server-side and can only be varied by recomputing them in SQL |
| **real queries** | natural-language text through the shipped `temper search … --wayfind` | **what a caller receives** |

A claim about caller experience needs the second. Do not mix their conclusions.

## Running

`prod-readonly.sh` takes a `.sql` file and never prints the connection string (it embeds a password;
command-substitute it straight into `psql`, never `echo` it).

```bash
cd scripts/wayfind-spike
./prod-readonly.sh 01-verify-deployed.sql     # ALWAYS run this first — see below
./prod-readonly.sh 02-anchors-and-regions.sql
./prod-readonly.sh 03-homing-partition.sql
./prod-readonly.sh 04-length-and-ts-rank.sql
./prod-readonly.sh 05-best-of-n-counterfactual.sql
./prod-readonly.sh 06-fts-match-rate-and-scope.sql
./prod-readonly.sh 07-region-orphans.sql
```

**`01-verify-deployed.sql` is not optional.** A migration in the tree is not a function in production.
Running it first is what caught that `wayfind_scope_ids` on prod is now a two-line delegate to
`wayfind_scope_reach` (`20260731000060`), so the scope-assembly body that both the task brief and the
register cite by filename (`20260731000050`) is no longer where the work happens.

### The caller-side half

```bash
./run-queries.sh 3 50 w3.jsonl      # default width
./run-queries.sh 1 50 w1.jsonl      # narrow width, where the anchor-kind prior is decisive
python3 parse.py w3.jsonl rows > w3_rows.csv
python3 parse.py w3.jsonl ids  > w3_ids.txt

# classify every returned resource by home anchor (distilled vs raw) and arm
python3 classify.py w3_ids.txt > classify.sql
./prod-readonly.sh classify.sql > classify_out.txt

python3 analyze.py w3_rows.csv 10          # funnel tables, arm decomposition, boundary margins
python3 counterfactual.py w3_rows.csv w1_rows.csv   # re-rank under alternative blend weights
```

`analyze.py` and `counterfactual.py` both read `classify_out.txt` from the working directory. When
comparing two widths, classify the **union** of both id sets, or rows from the width you did not
classify come back `unknown`.

## Gotchas paid for once already

- The probe SQL uses **CTEs, not temp views** — `CREATE VIEW` fails inside a read-only transaction.
- `run-queries.sh` writes a stream of concatenated JSON values, **not** JSON-lines, because the CLI
  pretty-prints. `parse.py` uses `raw_decode` in a loop; `jq -s` and `while read` both mis-parse it.
- Live rows only. Every region query carries `NOT r.is_folded`; folded regions are the majority of the
  table and inflate every region statistic several-fold.
- `neonctl connection-string` needs `--role-name neondb_owner --database-name neondb --org-id …`, or it
  writes an error to **stderr and nothing to stdout** — leaving an empty string that makes `psql`
  silently fall back to a local socket. `prod-readonly.sh` fails loudly on an empty string instead.
- `kb_chunks` holds no text column (only `content_hash` + `embedding`). The right length measure for
  the FTS arm is `length(si.search_vector)` — the lexeme count `ts_rank` actually operates on.
- There is no `kb_doc_types` table; `doc_type` lives in `kb_properties`, which is how
  `unified_search`'s `corpus` CTE reads it.

## The region-formation arms (task `019fbb78`, research `019fbbc0`)

A second question re-used this directory: whether re-forming a cogmap's regions makes its content
reachable. That one **cannot be answered read-only** — `salience` and `centroid` are computed by the
producer during materialize — so it needs writes, and writes never touch prod.

**Two probe sets live here and they are NOT interchangeable.** `queries.txt` is the original
20-query set for the spike above. `queries-24.txt` is the 24-query set (6 each M/C/S/N) that
research `019fbb52` and `019fbb76` both cite as "fixed and re-runnable" — it had never actually been
a file, only prose in `019fbb52`'s Appendix A, which is why it is committed now. Results across the
two are not comparable; say which you used.

```bash
# 1. query vectors, from the same embed_text the CLI uses
TEMPER_ONNX_MODEL_PATH=crates/temper-ingest/models/bge-base-en-v1.5/model_quantized.onnx \
  cargo run --release -p temper-ingest --no-default-features --features embed-download \
  --example query_vectors < queries-24.txt > vectors-24.tsv

# 2. probes (reach.sql / returned.sql are generated artifacts — regenerate, don't commit.
#    vectors-24.tsv IS committed; see "Why the vectors are committed" below.)
python3 gen_reach.py    vectors-24.tsv 'Temper — self-cognition' > reach.sql      # scope level
python3 gen_returned.py vectors-24.tsv 'Temper — self-cognition' > returned.sql   # returned rows

# 3. reference on prod, read-only; then the same probe on a branch, which MUST agree
./prod-readonly.sh reach.sql
./branch-readonly.sh <branch-id> reach.sql

# 4. arms — the only writes in this tree
./branch-write.sh <branch-id> none            # incumbent; validates the producer is reproducible
./branch-write.sh <branch-id> resolution-0.2  # declared-only granularity
./branch-write.sh <branch-id> w-cos-1.0       # admits similarity
./branch-write.sh <branch-id> workflow-default
```

**Writing to prod is impossible, not merely discouraged.** `region_materialize_arms` requires
`--expect-host` and refuses unless it matches the host it actually connected to (an allowlist, so a
stale string fails closed), and refuses prod `main`'s endpoint unconditionally on top — including
when you deliberately name prod in `--expect-host`. `DATABASE_URL` is never exported.

**Gotchas paid for once, here too:**

- `wayfind_region_scores` is **not lens-scoped** — its candidate CTE is `WHERE NOT r.is_folded` with
  no lens predicate; `p_lens` only supplies the `s_*` blend weights. Materializing a second lens
  leaves both partitions live in one candidate pool. Keep one `lens_id` across arms.
- `default_transaction_read_only = on` forbids **temp tables** as well as views, so a multi-block
  probe must be one statement over `MATERIALIZED` CTEs.
- **Scope is not returned rows, and the two disagree on which arm wins.** Run both.
- Region id turnover depends on the **previous partition**, not the arm: re-running the incumbent
  from its own state minted 0 of 385; from another arm's state it minted 338 of 385.

## Scope of what these measure

The funnel is `wayfind_region_scores` (Stage-1 region selection) → `wayfind_scope_reach` (scope
assembly: winning regions' members ∪ region-less anchors' homed resources) → `unified_search`
(Stage-2 blend: `1.0·fts + 1.0·vec + 0.5·graph`).

`07-region-orphans.sql` carries one hardcoded resource id — the worked exemplar from the write-up
(a distilled node that squarely answers a query and never surfaces). Replace it to check another.

## `ef_search` and candidate admission (task `019fbd30`, research `019fcf37`)

A third question re-used this directory: how much material the unscoped vector arm never admits.
Answer: `hnsw.ef_search` defaults to 40, below `unified_search`'s `vector_k = 100`, so the ANN
returns ~33 chunks per query instead of 100 and admits 24.3 resources against an exact scan's 66.2.

```bash
# real query vectors for the 20-query set (committed — see "Why the vectors are committed" below)
TEMPER_ONNX_MODEL_PATH=crates/temper-ingest/models/bge-base-en-v1.5/model_quantized.onnx \
  cargo run --release -p temper-ingest --no-default-features --features embed-download \
  --example query_vectors < queries.txt > vectors-20.tsv

python3 gen_recall.py  vectors-20.tsv hnsw  > recall_hnsw.sql    # production path
python3 gen_recall.py  vectors-20.tsv exact > recall_exact.sql   # exact-scan counterfactual
./prod-readonly.sh recall_hnsw.sql  > out_hnsw.txt
./prod-readonly.sh recall_exact.sql > out_exact.txt
python3 analyze_recall.py                                        # reads both out_*.txt

python3 gen_efsweep.py vectors-20.tsv > efsweep.sql              # admission + latency per ef
```

**Gotchas paid for once, here too:**

- **The ORDER BY must compare against a constant or a correlated parameter.** `ORDER BY e <=> q.e`
  across a plain join plans as `Seq Scan` + `Sort` — an exact scan wearing an ANN's clothes. Three
  rounds of local "recall" figures were that, and only `EXPLAIN` caught it. **Print the plan.**
- **Never probe recall with a document vector.** A query point that is itself in the index sits in a
  dense neighbourhood and the traversal keeps going; a real query vector sits off the manifold and
  terminates early. The same query measured 100 chunks with a document vector and 8 with a real one —
  a document-vector probe would have missed the defect entirely.
- **`SHOW hnsw.ef_search` fails on a fresh connection.** pgvector registers its GUCs in `_PG_init`,
  which runs on first use of a vector type in that backend. Run any `<=>` first. This is also why a
  Rust test must acquire ONE connection rather than using the pool.
- **Above ~ef 200 the planner abandons the index** for a seq-scan over every chunk. Results stay
  exact; latency goes up ~40x. Check the plan before reading a high-ef number as an ANN result.

## The `sal_norm` hinge — is salience a property of the region or of the asker? (task `019fd25e`, research `019fd275`)

A fourth question re-used this directory, and it is the hinge research `019fbd21` named and left
unrun. `wayfind_region_scores` normalizes salience with
`percent_rank() OVER (PARTITION BY home_anchor_table ORDER BY sal_eff)` over `cand`, and `cand`
joins `visible_region_anchors(p_principal)` — so the normalization **domain is the asker's visible
set**. The candidate alternative is one line: `PARTITION BY home_anchor_table, home_anchor_id`.

Answer: the candidate is **exactly** principal-invariant (0 differing regions across every reach
shape measured), so salience normalization *can* be a property of the region — but adopting it
re-allocates 16.7% of slots at the shipped default width and swaps a large-anchor top-of-scale
guarantee for a small-anchor one. A re-allocation, not a refactor.

```bash
./prod-readonly.sh h1-orient.sql        # deployed signatures + the reach landscape, per principal
./prod-readonly.sh h2-reach-shapes.sql  # which anchors each principal holds; pairwise overlap
./prod-readonly.sh h3-body.sql          # the full deployed body — reproduce from THIS
./prod-readonly.sh h5-narrow-real.sql   # the one real narrow principal: resolution, not delta
./prod-readonly.sh h6-mechanism.sql     # per-anchor salience distribution under both rules

# the differential itself — 8 reach shapes x 2 sal_eff arms x 24 queries, ONE candidate set
python3 gen_salnorm_reach.py vectors-24.tsv > salnorm_reach.sql
./prod-readonly.sh salnorm_reach.sql > out_salnorm_reach.txt
```

`gen_salnorm_alt.py` is the **earlier, unrun draft** of the same probe. It is superseded by
`gen_salnorm_reach.py` and kept only as the record of what was drafted before the arm below was
noticed. Do not run it for figures.

**Gotchas paid for once, here too:**

- **`p_lens IS NULL` does NOT mean "the default lens".** It means `sal_eff = r.salience`, the stored
  column — a different quantity from the telos blend. And NULL *is* the shipped default: `lens_id`
  is `None` unless the caller passes `--lens` (`crates/temper-cli/src/actions/search.rs:97`). A probe
  that models only the lens path is not measuring what a caller gets. Run both arms.
- **A real narrow principal is not automatically a usable one.** `lohjishan` has the narrowest reach
  in the corpus and shares **zero** regions with anyone else, so it admits no cross-principal
  differential at all. Check the overlap (`h2` §B) before designing around a principal; the
  narrow-reach deltas have to come from controlled restrictions of a *shared* anchor set.
- **`count(*)` over a `LEFT JOIN` to `kb_cogmap_region_members` counts region–MEMBER pairs, not
  regions.** They coincide only while every selected region holds exactly one member — which is the
  thing being measured, so it cannot be assumed. Count regions from the pre-join CTE.
- **`aggregate function calls cannot contain window function calls`** — `count(DISTINCT percent_rank()
  OVER …)` is rejected. Compute the window in a CTE, aggregate over it.
- **psql `\set` with a multi-line quoted value is fragile.** A shared CTE preamble cannot be hoisted
  into a variable *or* a view (read-only forbids views and temp tables), so emit it inline into every
  statement from the generator.
- **The corpus drifts inside one session.** The self-cognition map read 400 live regions during the
  main probe and 399 twenty minutes later. Timestamp every figure; do not reconcile across probes.

## Per-anchor vs global width — what still has a job after the split (task `019fd25e` step 0, research `019fd325`)

A fifth question re-used this directory. `p_regions_n` reaches **exactly one place** in the deployed
`wayfind_region_scores`: `top_regions … LIMIT (SELECT regions_n FROM n)`. Everything upstream —
`cand`, `scored` (`sal_norm`), `ranked`, `ranked_rr` (`map_rank`) — is computed over the full
candidate set regardless of width. So `ranked_rr.map_rank` **is** the per-anchor rank, and a
per-anchor width is `WHERE map_rank <= n`: a `LIMIT` deleted, not new machinery.

Answer: dissolving the global width leaves round-robin jobless and κ inert, and dissolves the
`no-structural-shutout` mechanism (`Storyteller System Design` 1 slot of 72 → 72 of 72). It does
**not** make the per-anchor `sal_norm` frame free — 23 of 360 slots and 14 of 24 queries still
change at the shipped default width. Scope cost: +60% members for the widest real principal.

```bash
python3 gen_peranchor_width.py vectors-24.tsv > peranchor_width.sql
./prod-readonly.sh peranchor_width.sql > out_peranchor_width.txt

# THE GUARD — run it, and read it before reading any figure above.
python3 gen_peranchor_equiv.py vectors-24.tsv > peranchor_equiv.sql
./prod-readonly.sh peranchor_equiv.sql            # `differs` must be 0 on every row
```

**Gotchas paid for once, here too:**

- **Reproducing the deployed body is not the same as validating the model.** `gen_peranchor_width.py`
  copies the body verbatim from `h3-body.sql` — and that is still only evidence. `gen_peranchor_equiv.py`
  is the check: it *calls* `wayfind_region_scores` on prod and compares `in_top_n` against the
  modelled cut (0 differing over 22,080 region-query pairs per width). Every selection figure in the
  sibling probe is a claim about a model until that guard is green.
- **A "0 differences" result needs a witness that the probe can see the thing at all.** The κ test
  asserts κ cannot change a *within-anchor* ordering. Run per-anchor only, `0` is equally consistent
  with a κ that is inert everywhere and a probe that is broken. It is run in **both** frames, and the
  global arm must be non-zero (it is: 30 differing selections at width 1) for the per-anchor `0` to
  mean anything.
- **A single-anchor shape is a control, not a datum.** Shapes 3/4/6 return exactly 0 delta because
  with one anchor the two width frames are definitionally the same rule. Their agreement validates
  the probe; it says nothing about the question.
- **Two rules that partition on the same set are indistinguishable.** Per-kind and per-anchor
  `sal_norm` differ only when a kind partition holds >1 anchor. Of the two real principals, only
  Pete's shape qualifies — shapes 2/3/4/5/6 read 0 in both frames at every width. Any claim about
  asker experience from this probe is a claim about a population of one; the *width-cost* and
  *shutout* figures do not inherit that limit.

## What is committed here, and why the vectors are among it

`.gitignore` carries the rule; this is the reasoning behind its one non-obvious clause.

Committed: hand-written probes (`NN-*.sql`, `pN-*.sql`, `hN-*.sql`), generators (`*.py`), runners
(`*.sh`), the probe sets (`queries*.txt`), and **the query vectors (`vectors-*.tsv`)**.
Ignored: generated SQL (a pure function of generator + vectors, and megabytes of inlined floats) and
all probe output (whose durable home is the temper research resource that cites it, provenance
stamp attached).

### Why the vectors are committed

**This reverses what earlier sections of this README said**, and the reversal is the point.

"Regenerate, don't commit" is right whenever regeneration is deterministic. For the vectors it is
not — or rather, it is deterministic only with respect to a **110 MB ONNX model that is not in git**.
`vectors-24.tsv` is produced by `cargo run … --features embed-download`, so the model is fetched, not
versioned. Ship a new quantized BAAI release and the same command emits *different* vectors from the
same `queries-24.txt`.

That breaks the one rule this directory exists to serve — *a before/after is worth nothing unless
both halves are the same measurement*. And it breaks it in the worst available way: **silently**. The
regenerated probe runs fine, prints plausible numbers, and is not comparable with the figures any
prior research note cites. Committing 420 KB of float text that should never change buys a loud
failure (a diff) in place of a silent one.

So: **the vectors are the identity of a probe set, not a build output.** Treat a change to
`vectors-*.tsv` as an event that invalidates cross-note comparability, not as noise.
