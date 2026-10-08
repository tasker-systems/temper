# Rate-limit an instance at the edge

**For operators** running Temper on Vercel with Neon Postgres who want per-IP rate limits in
the Vercel firewall, sized to what their own deployment can serve rather than copied from
someone else's.

## Outcome

By the end of this playbook your instance has edge rate-limit rules whose numbers come from
three measurements of your own deployment: where traffic enters, what one request costs the
database, and how much database you have. The rules start in log mode, and you will know how to
read a week of results before they start refusing requests.

## What an edge limit is for, and what it is not

Temper's own rule is that **the edge is a non-authority**: every control the instance relies on
ships in the code it deploys. An edge rule is defence in depth. It turns away floods and scanners
before they cost a function invocation; it does not decide who may do what.

Plan for three layers, because per-IP limits alone cannot separate people:

1. **A per-IP backstop at the edge**: high and coarse, sized so a whole office behind one NAT
   address does not trip it.
2. **Per-identity limits**: fairness between users, keyed on the token or principal, so many
   people behind one address do not share a budget. This playbook covers only the edge.
3. **Capacity headroom**: enough database that layers 1 and 2 can be generous.

## Prerequisites

- A running deployment ([self-hosting Temper](./self-host-temper.md)), and the Vercel CLI
  logged in to the team that owns it.
- Read-only SQL access to the production database, for `pg_stat_statements`. The extension is
  installed by Temper's migrations. Running the queries below against production is a decision
  for whoever owns that database: ask before you do.
- A secret store for one new value, `TEMPER_EDGE_PROXY_SECRET`.
- Vercel Deployment Protection on the API project's generated deployment URLs. The API project's
  rules below are scoped to one public host so crons stay out of their buckets; any other host
  that answers unprotected is a way around them.

## 1. Map where traffic enters

A per-IP rule is only as good as the address it sees. Find every hop where your own servers
call the API, because there the firewall sees your server's address, not the user's:

- **The web UI's reverse proxy.** If the [web UI](./deploy-the-web-ui.md) fronts the
  instance, its server forwards `/api`, `/mcp`, `/oauth` and `/.well-known` to the API project
  with a server-side `fetch`. The UI project's firewall sees real client addresses; the API
  project sees a few of the UI function's egress addresses for all of that traffic.
- **The web UI's page loaders.** Rendering a page (a vault listing, a search, the graph) makes the
  UI's server call the API directly — the same egress addresses again. The person behind it
  requested a page route, which the UI project's API-path rule below does not match, so that
  request meets no per-IP rule of its own.
- **The MCP relay.** Every MCP tool call is two requests: the caller to the MCP function, then
  the MCP function to the API. The second always comes from the MCP function's addresses.
- **Crons.** Vercel Cron calls the deployment's generated URL, not your domain.

Confirm the picture from the request metrics, per project:

```bash
vercel metrics vercel.request.count -p <api-project> --prod -s 7d \
  --group-by requestHostname --group-by route -l 20
```

Hostnames that are your domain or the project's production alias carry user traffic; hostnames
ending in the deployment-specific `.vercel.app` suffix are crons and internal calls.

**Every server-side hop marks its requests** — the UI's proxy and page loaders, and the MCP
relay — with the `x-temper-edge-proxy` header when `TEMPER_EDGE_PROXY_SECRET` is set, so the API
project's rules can exempt them. The header confers no access, and nothing in the API reads it.
But a leaked value lets its holder skip the API project's per-IP rules on that host, which are
the control for unauthenticated volume there, so keep it in your secret store, never in a
client, and rotate it if it leaks.

**Hosted MCP clients share addresses too.** An agent product's connector (a hosted chat
assistant calling your `/mcp`) reaches you from that vendor's egress range, so all of its users
share the UI project's per-IP buckets. Watch those addresses in step 7 as your user count grows.

## 2. Measure the traffic you have

Per-IP peaks over 5-minute windows, a week back. The metrics API coarsens buckets on long
ranges, so query in 6-hour slices to keep 5-minute resolution:

```bash
for i in $(seq 0 27); do
  vercel metrics vercel.request.count -p <ui-or-api-project> --prod \
    -s "$(( (i+1)*6 ))h" -u "$(( i*6 ))h" -g 5m --group-by clientIp -l 5 \
    -f '(requestPath:/api* OR requestPath:/mcp* OR requestPath:/oauth* OR requestPath:/.well-known*)' \
    --json 2>/dev/null
done > peaks.jsonl
jq -s -r '[.[].data[]? | {ip:.clientIp, n:.vercel_request_count_count}]
  | group_by(.ip) | map({ip:.[0].ip, peak:(map(.n)|max)}) | sort_by(-.peak) | .[:10][]
  | [.peak, .ip] | @tsv' peaks.jsonl
```

Note the busiest address and the busiest window overall. Then check whether latency moved in
that window (`vercel.function_invocation.function_duration_ms`, `-a p50` and `-a p95`, same
slice). A burst that ran fast tells you it was cheap, not where the limit is.

## 3. Measure what a request costs the database

The database is usually the bottleneck: functions scale out, a Neon compute does not beyond its
autoscaling ceiling. `pg_stat_statements` says where its time goes. Start with the window and the
totals:

```sql
SELECT (SELECT stats_reset FROM pg_stat_statements_info) AS since,
       sum(calls) AS calls,
       round(sum(total_exec_time)::numeric / 1000, 1) AS exec_s,
       sum(shared_blks_hit) AS hit, sum(shared_blks_read) AS read
FROM pg_stat_statements;
```

Split request-path time from background work and the platform's own monitoring. Adjust the
patterns to the crons your deployment runs:

```sql
WITH s AS (
  SELECT query, calls, total_exec_time,
    CASE
      WHEN query ~* '(pg_stat_|pg_settings|neon\.|pg_database_size|postgres_exporter|pg_catalog\.)'
        THEN 'monitoring'
      WHEN query ~* '(sensitivity|workflow_job_|embed|cogmap_region|erasure|reap)'
        THEN 'background'
      ELSE 'request_path'
    END AS class
  FROM pg_stat_statements)
SELECT class, sum(calls) AS calls, round(sum(total_exec_time)::numeric / 1000, 1) AS exec_s
FROM s GROUP BY class ORDER BY exec_s DESC;
```

Then see whether the request-path time is uniform or concentrated, by mean cost per statement:

```sql
SELECT CASE width_bucket(mean_exec_time, ARRAY[1, 10, 50, 200, 1000])
         WHEN 0 THEN '<1ms' WHEN 1 THEN '1-10ms' WHEN 2 THEN '10-50ms'
         WHEN 3 THEN '50-200ms' WHEN 4 THEN '200ms-1s' ELSE '>1s' END AS band,
       count(*) AS statements, sum(calls) AS calls,
       round(sum(total_exec_time)::numeric / 1000, 1) AS exec_s
FROM pg_stat_statements
WHERE NOT query ~* '(pg_stat_|pg_settings|neon\.|pg_database_size|postgres_exporter|pg_catalog\.)'
GROUP BY 1 ORDER BY min(mean_exec_time);
```

Divide each class's execution time by the API requests the same window served (step 2's
metric, `route` filtered to the API and MCP functions, `-s` set to the `since` timestamp). Count
an MCP tool call once: its relay hop also appears as an API request.

Expect two populations: **light requests** (authentication, visibility checks, single-resource
reads) at a few milliseconds of database time each, and a handful of **heavy reads** (listings
over a whole visible set, search) at hundreds of milliseconds. The heavy ones set your ceiling.
A cache hit rate above about 98% means execution time is mostly CPU, which is what the next
step assumes.

`pg_stat_statements` records a statement when it finishes, so these are lower bounds, and its
counters reset when the compute restarts. Note the window you measured.

## 4. Work out what the database can sustain

One Neon CU is one vCPU. Keep the database at or below about 60% busy so bursts have room:

```
sustainable requests/s ≈ max_CU × 1000 × 0.6 / database ms per request
```

Do it separately for light and heavy requests. With a light cost of about 7 ms and a heavy cost
of about 300 ms, for illustration:

| Max CU | Light/s | Heavy reads/s |
|---|---|---|
| 0.5 | ~40 | ~1 |
| 2 | ~170 | ~4 |
| 8 | ~690 | ~16 |

Use your own costs. If the heavy figure is uncomfortable, raising the compute ceiling is
cheaper than tightening limits, and making the heavy queries cheaper beats both. You pay above
the minimum CU only while load needs it.

## 5. Choose the limits

**Backstop, per IP, all API paths.** Size it to people, not to capacity: the number of people
you expect behind one address times one person's busy rate, with room above the busiest single
address you measured in step 2 (at least 1.5×, treating that address's 5-minute peak as if it
fell in one minute). It should come out at a small fraction of light capacity. A reasonable
starting point is a few thousand per 5 minutes.

**Heavy reads, per IP.** No single address should take more than about a quarter of heavy
capacity. At 4 heavy reads/s that is about 1/s, or 300 per 5 minutes, which still fits ten
people each doing a few listings or searches a minute. Today's heavy reads are
`GET /api/resources`, `GET /api/contexts`, `/api/search` and `/api/query`; check yours against
step 3's top statements.

These path rules do not see heavy reads made through MCP: a search an agent makes arrives as a
request to `/mcp`, and its relay hop to `/api/search` carries the marker. An agent's rate is
bounded by its model's turn time, so in practice it stays well under the backstop, but the
limit that would bind it is a per-identity one, not anything at the edge.

## 6. Create the rules in log mode

Rules are project configuration in the Vercel firewall, not part of `vercel.json`, so they
apply to the project you create them on and nowhere else.

**On the project that sees real client addresses** (the UI project when it fronts the instance;
otherwise the API project), the backstop:

```json
{
  "name": "Per-IP backstop: API, MCP and OAuth paths",
  "active": true,
  "conditionGroup": [
    { "conditions": [ { "type": "path", "op": "re", "value": "^/(api|mcp|oauth|\\.well-known)(/|$)" } ] }
  ],
  "action": { "mitigate": { "action": "rate_limit",
    "rateLimit": { "algo": "fixed_window", "window": 300, "limit": 3000, "keys": ["ip"], "action": "log" } } }
}
```

and the heavy-read rule, two condition groups (groups are OR-ed, conditions within a group
AND-ed):

```json
{
  "name": "Per-IP limit: heavy reads",
  "active": true,
  "conditionGroup": [
    { "conditions": [
      { "type": "method", "op": "eq", "value": "GET" },
      { "type": "path", "op": "re", "value": "^/api/(resources|contexts)/?$" } ] },
    { "conditions": [ { "type": "path", "op": "re", "value": "^/api/(search|query)/?$" } ] }
  ],
  "action": { "mitigate": { "action": "rate_limit",
    "rateLimit": { "algo": "fixed_window", "window": 300, "limit": 300, "keys": ["ip"], "action": "log" } } }
}
```

The inner `rateLimit.action` is what happens when an address exceeds the limit: `log` counts
and lets the request through.

```bash
vercel firewall rules add --project <project> --json "$(cat backstop.json)"
vercel firewall diff --project <project>       # only your rules should be pending
vercel firewall publish --project <project>
```

**On the API project, when something fronts it**, add the same two rules for callers that reach
it directly, with two extra conditions in every group: the host your users reach it on, and the
exemption for marked requests:

```json
{ "type": "host", "op": "eq", "value": "<the API project's public host>" },
{ "type": "header", "key": "x-temper-edge-proxy", "op": "eq", "value": "<TEMPER_EDGE_PROXY_SECRET>", "neg": true }
```

Before publishing these, generate the secret (`openssl rand -base64 32`). The API and MCP
functions refuse to boot when it is under 16 characters or, on the API, equal to another shared
secret; the refusal names the variable and never its value. Then set `TEMPER_EDGE_PROXY_SECRET`
on the UI project and on the API project (the MCP function reads it), and redeploy both: a
function's environment is fixed when it is deployed. Without the marker, the UI's and the MCP relay's traffic counts against a few
shared addresses. A value that cannot be a header value is never sent; the function logs a fixed
sentence saying so.

To rotate it: set the new value on both projects and redeploy both, then update the value in
both API-project rules and publish. Between the redeploy and the publish, marked requests carry
a value the rules do not know and count per address — harmless while the rules only log; while
they enforce, do it in a quiet hour.

## 7. Read a week, then enforce

After about a week, see what would have been refused:

```bash
vercel metrics vercel.firewall_action.count -p <project> -s 7d \
  --group-by wafRuleId --group-by wafAction
```

The firewall metric has no environment dimension, so leave `--prod` off; with it the request
is refused as invalid. If nothing legitimate would have been refused, switch each rule's
`rateLimit.action` from `log` to `rate_limit` (a `429`), and publish. If something would have, find out who it was before
raising the number: a raised limit is a decision about capacity, not a fix.

Repeat steps 2 to 5 when the compute ceiling changes, when a new kind of client starts calling
the API (a desktop app polls very differently from a person), or when traffic roughly doubles.
