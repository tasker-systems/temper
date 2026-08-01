# Research spike — a failed write does not tell you whether it landed, and the correct recovery is opposite in the two cases

- **Date:** 2026-08-01
- **Status:** research spike — ends in a recommendation; changes no production behaviour
- **Task:** `019fbf6a-b3f2-7ea0-b2e8-8e4ff1a3db31` (temper `@me/temper`)
- **Inputs:** [issue #581](https://github.com/tasker-systems/temper/issues/581) (lost acknowledgment), [PR #610](https://github.com/tasker-systems/temper/pull/610) (proxy 502/504 classification)
- **Sits under:** *OpenTelemetry across the deployable surface — one trace per user action* (`019f9404-2a4e-7530-8744-92ae4ab6d83e`)

## The problem, in one paragraph

A resource-mutating call can fail two ways that are **indistinguishable at the client** and whose
**safe recoveries are opposite**. In the *lost-acknowledgment* mode (#581) the write committed and
the ack was lost — the safe move is **reconcile, never retry**, because a blind retry mints a
duplicate. In the *never-dispatched* mode (observed 2026-08-01) the request never reached the
upstream — the safe move is **retry**, because there is nothing to reconcile against. Guessing wrong
is destructive in one direction (duplicate) and useless in the other (a reconcile that finds
nothing). The distinguishing evidence exists at the proxy hop and is thrown away before the client
sees it.

## The two modes, measured — and how to tell which one an observation is

Never write "a write failed." Every observation below is classifiable. The discriminators, in
descending order of reliability, are: (1) the underlying connection error **phase** (connect vs
post-dispatch), which lives in undici's `err.cause.code` at the proxy and is currently discarded;
(2) the **timing**; (3) the **error type the client receives** (`network error` vs `server error
(502)`), which correlates but does not prove.

| | **#581 — lost acknowledgment** | **2026-08-01 — never dispatched** |
|---|---|---|
| what happened | request reached the upstream, **committed**, connection dropped before the ack returned | proxy could not open a connection to the upstream; the upstream **never saw it** |
| where the drop is | on the **response** path (post-commit trailing work on the same request; instance killed mid-tail) | on the **connect** path (TCP/TLS churn temper-ui → temper-cloud) |
| evidence | resource present on re-inspection; one session hit it **3 of 3** errored writes, every one committed | `proxy: upstream unreachable … timedOut: false, error: 'fetch failed'`, **~130 ms**; write verified **absent** across 3 attempts |
| distinguishing signal (if surfaced) | post-dispatch socket death — `err.cause.code` is `ECONNRESET`/`UND_ERR_SOCKET` *after bytes were written*, timing past the commit | connect-phase failure — `ECONNREFUSED` / connect-timeout / reset during handshake; sub-second, no upstream `x-vercel-id` |
| trigger | deploy windows, cold-start latency, post-commit trailing work | connection churn under steward `/mcp` load |
| what the CLI sees **today** | `network error: …` (`ClientError::Network`, no HTTP status) → non-zero exit | `server error (502): Bad Gateway: upstream API unreachable` (`ClientError::Server{502}`) → non-zero exit |
| **correct recovery** | **reconcile — never retry** (a retry mints a duplicate) | **retry — reconciling finds nothing** |

### Why the modes are only *partially* distinguishable at the client today

The error **type** correlates with the mode but does not prove it, for two reasons rooted in the
code:

1. **The proxy collapses both connect-phase and post-dispatch failures into one opaque 502.**
   `forwardRequest` catches every non-timeout `TypeError: fetch failed` and returns the identical
   body `{"message":"Bad Gateway: upstream API unreachable"}` with status 502
   (`packages/temper-ui/src/lib/server/proxy.ts:248-255`). Its own comment states the proxy "cannot
   tell which" (`proxy.ts:70-78`). So a 502 means "the proxy's fetch to the upstream failed at the
   connection layer" — which **includes** the post-commit socket-death case (#581 through the
   proxy), not only the never-dispatched case.

2. **A genuine `network error` can arise in either mode**, depending on which hop dies. If the
   CLI↔proxy hop drops (not the proxy↔upstream hop), the CLI's `reqwest` call errors directly →
   `ClientError::Network` (`crates/temper-client/src/error.rs:53`), with no HTTP status at all —
   and that can happen whether or not the upstream committed.

The one thing that **is** reliable is asymmetric: a **connect-phase** failure
(`ECONNREFUSED`, DNS failure, connect timeout, TLS handshake failure) is *provable* evidence the
request was never dispatched — no request bytes were ever written to the upstream. Everything else
(post-dispatch resets, whole-hop network errors) is genuinely ambiguous. This asymmetry is the
whole basis of the recommendation: the system can *positively* identify the retry-safe mode, and
must treat everything it cannot positively identify as reconcile-first.

## What the 2026-08-01 incident actually showed

`temperkb.io` is temper-ui, which reverse-proxies `/api`, `/mcp`, `/oauth`, `/.well-known` to
temper-cloud (`proxy.ts:28`, `hooks.server.ts:54-56`). Across a ~45-minute window the proxy logged
42 × 502 — `/mcp` ×34 (steward-agent), `/api/ingest` ×4, `/api/resources/{id}` ×3, `/api/search`
×1 — all `fetch failed` at connect level. The API itself was healthy throughout (every route
answered `401` to an unauthenticated probe; `list` and frontmatter-only `PATCH` succeeded in the
same minutes). PR #610 had shipped the 502/504 **classification** earlier that day; the churn
itself recurred hours later. `attempts: 1` on those writes is **correct, not a bug** —
`RETRYABLE_METHODS = {GET, HEAD}` (`proxy.ts:79`), because replaying a non-idempotent write could
double-apply it.

**The signal that distinguishes the modes already exists at the proxy and is thrown away.** undici
raises `TypeError: fetch failed` with a `.cause` carrying the underlying Node system error
(`ECONNREFUSED`, `ECONNRESET`, `UND_ERR_SOCKET`, `UND_ERR_CONNECT_TIMEOUT`, `ENOTFOUND`, …). The
proxy logs only `lastErr.message` — the string `"fetch failed"` — and **never inspects
`err.cause`** (`proxy.ts:246`; confirmed by grep: nothing in `temper-ui/src` reads `.cause` or any
`ECONN*`/`UND_ERR*` code, only `proxy.test.ts:241` references `ECONNRESET` in a fixture). A
`fetch failed` with `timedOut: false` at ~130 ms is near-proof of a connect-phase failure — which
would make a retry *safe*. The proxy has the evidence in hand; the CLI receives an opaque 502.

## The current client behaviour, mapped

Retry policy is consistent and correct at both hops, and both refuse to retry writes for the same
reason — no idempotency key:

| hop | retries | scope | on writes |
|---|---|---|---|
| CLI (`reqwest`) | `MAX_ATTEMPTS = 3`, 200 ms backoff doubling | `should_retry`: GET/HEAD only, on `Network` or 5xx (`crates/temper-client/src/http.rs:52-59`) | **never** (POST/PATCH not safe) |
| proxy (`undici`) | 1 | `RETRYABLE_METHODS = {GET, HEAD}` (`proxy.ts:79,205-207`) | **never** |

A proxy 502 lands at the client as `ClientError::Server { status: 502, message }` via
`map_status_to_error` (`http.rs:422-430`), rendered `server error (502): Bad Gateway`. A genuine
transport error lands as `ClientError::Network` (`error.rs:53`), rendered `network error: …`.

**A reconcile hint already ships — and it has a blind spot this spike surfaces.**
`crates/temper-cli/src/reconcile_hint.rs` prints "reconcile, don't retry" guidance after a
lost-ack write failure. But it fires **only** on `TemperError::Network(_)` and deliberately
suppresses on any status the server returned, on the stated premise that "a 4xx/5xx the server
actually returned is unambiguous" (`reconcile_hint.rs:66-71`, and the doc comment at lines 20-23).
**That premise is false for the proxy's synthetic 502.** A `502 Bad Gateway: upstream API
unreachable` is not the *upstream* saying what went wrong — it is the *proxy* saying it could not
reach the upstream, which is exactly ambiguous about whether the write landed. Consequence: the
2026-08-01 mode (which arrives as `Server{502}`) triggers **neither** the reconcile hint **nor**
any retry guidance. The CLI prints `server error (502): Bad Gateway` and exits — "true and
unactionable," precisely the state the task names.

## Does `temper memory migrate`'s idempotency generalize?

`temper memory migrate` is idempotent against both modes by construction. It builds an
`already_migrated` set from `open_meta.source_file` across the target contexts
(`crates/temper-cli/src/commands/memory/migrate.rs:229-241`) and, in the pure planner, skips any
file whose name is already present (`migrate.rs:665-668`); the key is written back on create
(`migrate.rs:447-471`). A re-run therefore skips whatever landed and creates whatever did not — a
**client-side read-before-write keyed on a natural key**, not a synthetic token, enforced by no DB
constraint.

**It generalizes only where a natural key exists — which is exactly where the ordinary
create/update path has none.** The server already does natural-key idempotency in several
specialized places:

- segmented ingest append — idempotent in SQL on `(resource, seq, block merkle)`
  (`writes.rs:1274`; `UNIQUE (resource_id, seq)` in `canonical_schema.sql:553`);
- `kb_ingestion_records` — upsert `ON CONFLICT (resource_id) DO UPDATE` (`writes.rs:1350-1368`);
- cogmap reconcile — diffs on the pre-generated stable landmark id, so a re-run fires zero mutation
  events (`db_backend.rs:1006-1101`);
- edge / property re-assertion — `uq_kb_edges_assertion`, `uq_kb_properties_active`.

The ordinary **one-shot create mints a fresh id every time** (`writes.rs:193-246`,
`resource_id: None` → "mint a fresh id"), and `origin_uri` is documented repeatedly as "loose,
non-unique attribution, NEVER a key." So `create` has, by design, no natural key to reconcile
against — which is the whole reason a blind retry duplicates. Memory-migrate's trick works because a
migration *has* a natural key (the source filename); a fresh authored `create` does not. That gap is
what a **synthetic** idempotency key fills.

> Prose-defect noted in passing (belongs to *Prose is a defect surface*, not this spike): the
> OpenAPI doc on `POST /api/ingest` already advertises "(or existing on dedup)"
> (`handlers/ingest.rs:50`), but the one-shot create path does **not** dedup. The string is only
> truthful for the reconcile/kernel path.

## Should writes carry a client-supplied idempotency key?

**Recommendation: yes — and the wire cost is near-zero because the carrier already exists end to
end.** `ActInput` already flattens a caller-minted `correlation_id` onto **every** write DTO
(`crates/temper-core/src/types/authorship.rs:115-139`), threads it through `into_act_context` →
substrate `EventContext` → `kb_events.correlation_id` (`events.rs:594-639`), and it is reachable
from the CLI, the API, and every MCP write tool today. It is explicitly *not* an idempotency key —
"a correlation aid, NEVER authorization … nothing gates on it"
(`authorship.rs:14-16`; migration `20260709000050`) — and `kb_events.correlation_id` is a **plain,
non-unique** index (`canonical_schema.sql:472,491`). So the *plumbing* is done; what is missing is
(a) a dedup semantic and (b) a decision about whether to reuse `correlation_id` or add a dedicated
`idempotency_key`.

### Cost across every write surface

| surface | DTO / entry point | wire change | dedup change |
|---|---|---|---|
| `POST /api/ingest` | `IngestPayload` (+ flattened `ActInput`) → `CreateResource` (`handlers/ingest.rs:55-166`) | none if reusing `correlation_id`; one optional field if dedicated | server-side: dedup on key before mint |
| `PATCH /api/resources/{id}` | `ResourceUpdateRequest` (+ `ActInput`) → `UpdateResource` (`handlers/resources.rs:286-367`) | as above | update is *closer* to idempotent already; key makes re-apply a no-op returning current state |
| `POST /api/resources` | `ResourceCreateRequest` (+ `ActInput`) (`resources.rs:223-268`) | as above | as ingest |
| MCP `create_resource` / `update_resource` / `update_resource_meta` | `*Input` structs each flatten `ActInput` (`tools/resources.rs:65,218` …) | inherited via `ActInput` | inherited via `CreateResource`/`UpdateResource` |
| MCP `ingest_begin` (+ append/finalize/blocks) | `IngestBeginInput` flattens `CreateResourceInput` → `ActInput` (`tools/ingest.rs:63-65`) | inherited | append/finalize already natural-key idempotent |

**The real cost is server-side and singular, not per-surface:** one dedup mechanism at the
`CreateResource`/`UpdateResource` command boundary (where all surfaces already converge through
`DbBackend`), plus one uniqueness constraint. Two shapes to choose between:

- **Reuse `correlation_id` as the dedup key.** Zero wire change; every surface already carries it.
  Cost: it stops being purely provenance — a partial UNIQUE index (e.g. `UNIQUE (author_profile,
  correlation_id) WHERE kind = 'resource_created'`) changes its contract, and callers that reuse one
  `correlation_id` across *distinct intended writes* (legal today, since it is act-grain and
  provenance-only) would suddenly collide. This overloads a field whose docs promise it gates
  nothing.
- **Add a dedicated `idempotency_key` on `ActInput`** (recommended). Keeps `correlation_id`'s
  contract intact; the key means exactly "these two requests are the same write, return the first
  result." Cost: one nullable field on `ActInput`, one column on the dedup table (or a dedicated
  `kb_idempotency_keys(key, author, first_result_ref, created)` with a UNIQUE on `(author, key)`),
  and the create/update commands consult it before minting. When present and seen, return the
  existing resource (the "(or existing on dedup)" contract, made real); when absent, today's
  behaviour exactly (self-roots, mints fresh) — so it is strictly additive and safe on `main`.

**With an idempotency key, both modes collapse to "retry" and the whole distinction stops
mattering** at the client: a retry with the same key is a no-op returning the committed resource in
the lost-ack case, and a fresh apply in the never-dispatched case. It also unlocks *proxy-level*
retry for keyed writes — the `RETRYABLE_METHODS` restriction exists solely because replay could
double-apply, so a keyed `PATCH`/`POST` becomes retryable and the 2026-08-01 write failures (7 of
42) would have self-healed transparently at the proxy.

## Recommendation

Ordered by value-per-cost. None of this is implemented by this spike.

1. **Make the honest-CLI behaviour correct now (cheap, no protocol change).** Fix the
   `reconcile_hint.rs` blind spot: a `server error (502)` whose body is the proxy's
   `"Bad Gateway: upstream API unreachable"` is **not** unambiguous and should surface guidance,
   not silence. Because that 502 is currently un-disambiguated, the honest guidance for it is the
   *same* reconcile-first hint (reconcile is always safe; it only wastes a read in the
   never-dispatched case), with a note that a retry is safe *if* reconcile finds nothing. This
   removes the "true and unactionable" state without any server or proxy change.

2. **Surface the phase signal the proxy already holds (small, high-value).** Read `err.cause`
   (`.code` / undici error name) in `forwardRequest`'s terminal path (`proxy.ts:239-255`) and emit a
   machine-readable discriminator on the 502 — e.g. a header `x-temper-upstream-phase: connect`
   (provably never-dispatched: `ECONNREFUSED`/`ENOTFOUND`/connect-timeout/TLS) vs
   `x-temper-upstream-phase: dispatched` (ambiguous: post-write socket death) — and include it in
   the JSON body. Then the CLI can map `phase: connect` → **retry-safe** and everything else →
   **reconcile-first**. This is one-directional by construction: only positively-connect-phase
   failures are advertised as retry-safe; the ambiguous remainder defaults to reconcile. It closes
   most of the client-side gap without an idempotency key and needs no schema change.

3. **Add a dedicated `idempotency_key` on `ActInput`, deduped server-side at the
   `CreateResource`/`UpdateResource` boundary (the complete fix).** Strictly additive; absent-key
   behaviour is unchanged. Makes both modes collapse to "retry," makes the OpenAPI dedup contract
   true, and unlocks keyed proxy/CLI retry for writes. This is the acceptance-criterion answer #581
   asks for. Prefer a dedicated key over overloading `correlation_id`, whose provenance-only
   contract callers already rely on.

Do **1** immediately (it is a doc/hint fix, not production write behaviour). Do **2** alongside the
sibling TypeScript-telemetry work (it is the same "make the failed hop legible" theme and shares the
`err.cause` read). Do **3** as its own change under #581 — it is the load-bearing one, and it makes
1 and 2 into a graceful-degradation ladder rather than the whole answer.

## Distinguishability, stated plainly (acceptance criterion)

- **Today, at the client:** partially, and unreliably. The error *type* (`network error` vs
  `server error (502)`) correlates with the two modes but proves neither, because the proxy collapses
  connect-phase and post-dispatch failures into one 502 and a whole-hop network error can occur in
  either mode. The only reliable classifier — connect-phase = never-dispatched — lives in
  `err.cause` at the proxy and is discarded before the client sees it.
- **After recommendation 2:** the never-dispatched mode becomes positively identifiable at the
  client (retry-safe); the lost-ack / ambiguous remainder is correctly defaulted to reconcile-first.
- **After recommendation 3:** the distinction stops mattering — both modes are safe to retry.

## Open questions (named, not omitted)

- **Why is Vercel→Vercel connect failing at all, and is it load-shaped?** The steward is both the
  dominant traffic and the dominant victim (34 of 42). Whether this is undici connection-pool
  exhaustion, Vercel edge→function cold-connect churn, or Neon-adjacent backpressure is
  undetermined — this spike classifies the failure, it does not root-cause the infrastructure.
- **What is the exact `err.cause.code` distribution for the two real incidents?** The ~130 ms /
  `timedOut: false` shape strongly implies connect-phase, but the actual codes were not captured
  (the proxy discards them). Recommendation 2 would make this measurable going forward; until then
  the phase claim is inferred from timing, not read from the error.
- **Does a post-dispatch `ECONNRESET` reliably differ from a pre-dispatch one in `cause.code`
  alone,** or is timing/phase-tracking also needed? `ECONNREFUSED`/`ENOTFOUND`/connect-timeout are
  unambiguously pre-dispatch; `ECONNRESET`/`UND_ERR_SOCKET` may need "were request bytes written?"
  context the current single-`fetch` shape does not track. The safe design treats only the
  unambiguous codes as retry-safe.
- **Should `idempotency_key` reuse or replace `correlation_id`?** This spike recommends a dedicated
  key to preserve `correlation_id`'s provenance-only contract, but the trade-off (one more field vs
  overloading an existing one) is a design decision, not settled here.
- **The stale-response sub-hazard in #581** (a *successful* update returning a body that does not
  reflect the mutation; three candidate mechanisms — pre-write projection, read-after-write delay,
  combined-flag partial application) is a **separate phenomenon** from the failed-write modes and is
  out of scope here. It needs the controlled probe set #581 describes, against a warm backend.

## Cross-hop correlation note

The distinguishing evidence is cross-hop: it lives in the proxy's log, not the API's, and
correlating them is exactly *one trace per user action* (the parent goal). The sibling session
building the TypeScript half (`@opentelemetry` behind a `packages/temper-telemetry-ts` wrapper) is
the right consumer for recommendation 2 — coordinate, don't duplicate. Note the limit already
observed: when the request never reaches the upstream there is **no upstream span to correlate
against**; the absence *is* the signal, and a trace view must represent "the hop that never
happened" rather than showing a gap that reads as missing instrumentation.
