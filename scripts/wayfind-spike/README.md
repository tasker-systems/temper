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

## Scope of what these measure

The funnel is `wayfind_region_scores` (Stage-1 region selection) → `wayfind_scope_reach` (scope
assembly: winning regions' members ∪ region-less anchors' homed resources) → `unified_search`
(Stage-2 blend: `1.0·fts + 1.0·vec + 0.5·graph`).

`07-region-orphans.sql` carries one hardcoded resource id — the worked exemplar from the write-up
(a distilled node that squarely answers a query and never surfaces). Replace it to check another.
