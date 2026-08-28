# Reconcile-channel health (TraceQL) — and the alert that pages on it

The queries an operator runs to answer *is IdP de-provisioning still happening?*, and the rule that
asks the question for them.

Companion to [the internal reconcile channel](../auth/reconcile-channel.md) and
[OpenTelemetry setup](../../docs/playbooks/send-traces-to-an-otlp-backend.md). Datasource: the Tempo
datasource (`grafanacloud-traces`). Sibling of
[drain-operator-queries.md](./drain-operator-queries.md), whose verification marks this file reuses.

---

## What is being watched, and why it needed watching

A SAML membership reconcile can fail to happen three ways. Two of them already reached somebody:

| Failure mode | Reaches an operator? |
|---|---|
| The reconcile endpoint errors | **Yes** — `temper-api` carries opentelemetry and ships spans to Tempo |
| **temper-cloud never reaches the endpoint** — transport, a rejected signature, `requireEnv` throwing on an unset `INTERNAL_RECONCILE_URL`/`INTERNAL_RECONCILE_SECRET` | **This file.** Before it: swallowed by the fail-open catch into a `logger.error` on a surface with no telemetry pipeline |
| The assertion carried no group signal ⇒ reconcile declined | **Yes** — `kb_saml_principal_reconcile.last_skipped_at` (migration `20260827000030`) |

The middle row is what `kb_internal_call_health` and `/api/internal-calls/health` exist for.

**A surface cannot report on itself when it is the broken part.** The failure is an outbound HTTPS
call from a Vercel function not completing, and an OTLP export from that function is the same shape
over the same egress. So temper-cloud writes a fact to Postgres — the one dependency proven
reachable at the moment of failure, since the same request wrote `kb_saml_replay` two statements
earlier — and temper-api reads it on a cron and emits the span these queries read.

---

## Read this before trusting a query

Verification marks, identical in meaning to the drain file's:

| Mark | Means |
|---|---|
| **`[live]`** | Executed against production Tempo and returned real data, quoted with its date |
| **`[shape]`** | The *aggregation form* was executed live and works — but this exact query has not returned data |
| **`[blind]`** | Written against the design, never executed |

**Every query below is `[shape]`, and will stay that way until a login on the enterprise deployment
runs the cron at least once.** The forms were executed against production Tempo on 2026-08-28 and
parse and run; they return an empty series because the spans do not exist yet. The control run in
the same session — `{ name = "region_dispatch" } | count_over_time()` returning **59** — is what
distinguishes *"this query is wrong"* from *"this span has not shipped"*. Re-run and re-mark each as
its case actually occurs.

**These spans are `internal` kind**, so none of this appears in `traces_spanmetrics_*`. That is by
design, exactly as for the drains: they are observations, not request boundaries, and TraceQL
metrics reads any span.

---

## H1 — Is any channel sustained? `[shape]`

```traceql
{ name = "internal_call_health" && span.state = "sustained" } | count_over_time()
```

**The alert rule's own query.** Non-zero means de-provisioning through that channel has stopped.

## H2 — What is each channel's state right now? `[shape]`

```traceql
{ name = "internal_call_health" } | count_over_time() by (span.channel, span.state)
```

The orientation query, and the five states it can return:

| `state` | Means | Alerts? |
|---|---|---|
| `healthy` | A call completed, and none has failed since | No |
| `transient` | Failing, but not yet on evidence that separates a blip from a stopped channel | No |
| `sustained` | De-provisioning through this channel has stopped | **Yes** |
| `no_attempt_recorded` | Nothing has ever been recorded | No — see below |
| `stale` | Something was recorded, but not within a day, so it is not a claim about now | No — see below |

`saml_reconcile` is the only channel today; the sibling principal-resolve call is the same shape and
can be added without a migration.

## H3 — Which cause, so you know what to do? `[shape]`

```traceql
{ name = "internal_call_health" && span.state = "sustained" } | count_over_time() by (span.failure_cause, span.failure_detail)
```

The cause is the operator action, and the four are three different jobs:

| `failure_cause` | What happened | What to do |
|---|---|---|
| `config_missing` | `requireEnv` threw. `failure_detail` **names the variable** | Set `INTERNAL_RECONCILE_URL` / `INTERNAL_RECONCILE_SECRET` on the temper-cloud deployment |
| `unauthorized` | temper-api answered 401/403 | The two sides' secrets disagree, or the clock skew exceeds the verifier's window |
| `transport` | The request got no answer; `failure_detail` is the error's name | Reachability between the Vercel function and the API host |
| `endpoint_error` | Any other non-2xx; `failure_detail` is `HTTP <status>` | Read the endpoint's own spans — this one **does** reach Tempo on its own |

## H4 — Is it failing intermittently? `[shape]`

```traceql
{ name = "internal_call_health" } | max_over_time(span.failures_total)
```

**The query the state field cannot answer.** `consecutive_failures` is reset by every success, so a
channel failing half its logins never accumulates a run and reports `healthy` on most ticks while
half of all de-provisioning is not happening. `failures_total` is monotonic and keeps rising through
exactly that. A rising line under `state = "healthy"` is an intermittent channel, and it is the one
case the default alert deliberately does not fire on — a rate needs a threshold nobody can calibrate
from inside the code, and a threshold belongs here, where it changes without a deploy.

## H5 — How long has it been failing? `[shape]`

```traceql
{ name = "internal_call_health" } | max_over_time(span.failing_for_seconds)
```

## H6 — When did anything last succeed? `[shape]`

```traceql
{ name = "internal_call_health" } | min_over_time(span.seconds_since_success)
```

> **This series has gaps, and they are not missing data.** `seconds_since_success` is recorded only
> when a success has ever been recorded — the same discipline `oldest_pending_age_ms` follows in the
> drain file. A zero would read as *succeeded just now*, which is the opposite of the truth for a
> channel that never has.

---

## The alert

**Created 2026-08-28.** Folder `Temper alerts` (`temper-alerts`), group `internal-call-health`,
rule uid `dfwhxiq8y7d34c`, `for: 15m`, `severity=critical`. Its query is H1; `noDataState` and
`execErrState` are both `OK`.

### It detects; wiring it to a person is one manual step

At creation this Grafana had **no contact points**, and the root notification policy routed to a
receiver named `empty`. The rule therefore fires into nothing. Closing that is two actions in
**Alerting**, and the indirection is deliberate — the address lives in exactly one place, so
changing who is paged never touches the rule or the policy:

1. **Contact points → Add contact point.** Name it `Temper operators`, type Email, address
   `pete.jc.taylor@hey.com`. A self-hosting operator substitutes their own here and changes nothing
   else in this file.
2. **Notification policies → New child policy.** Matcher `severity = critical`, contact point
   `Temper operators`.

> This could not be done from here: the Grafana service-account token that created the rule has no
> alert-notification scope — `POST /api/v1/provisioning/contact-points` answers 403 asking for
> `alert.notifications.provisioning:write`, and the Alertmanager config endpoint answers 403 asking
> for `alert.notifications.config-history:read`. Granting the token the **Notifications Writer**
> role would make both steps scriptable.

### `noDataState: OK` is a deliberate narrowing

No data means the cron is not running *or* the deployment has never logged in, and this rule cannot
tell those apart — the same reason it does not alarm on `no_attempt_recorded`. **So a cron that
stops running is not covered by this rule**, and covering it needs a heartbeat on the cron itself,
which is a different mechanism from this one.

---

## Three things this deliberately does NOT alert on, and why

**Silence.** `no_attempt_recorded` means nothing has ever been recorded for the channel. In a
login-triggered system that is indistinguishable from nobody having logged in, so alarming on it
would fire on every quiet weekend of every enterprise deployment. `20260827000030` refused the same
shape for its own table — *"a permanent false alarm from a table built to surface real ones."*
Nothing is lost by the refusal: every failure mode in the table above occurs **during** a login and
therefore leaves a positive record.

**A verdict nobody has refreshed.** A row is only touched by a login, so a channel that stops being
exercised freezes whatever it last said and repeats it forever. That bites in both directions and
the loud one is worse: an operator paged for `config_missing` who responds by turning group
provisioning off stops the reconcile being attempted at all, so no success is ever written and the
rule would page every fifteen minutes **after the fix that resolved it** — which teaches them to
ignore the one signal this mechanism exists to send. The quiet direction flatters instead: a channel
that recorded one success and was then switched off would report `healthy` forever while nothing
reconciles. So evidence older than a day is reported as `stale` rather than believed. A day, not an
hour: long enough that a real failure pages for a full twenty-four hours first, and long enough to
outlive the gap between logins on a deployment that is genuinely in use.

**A single weather-capable failure.** `transport` and `endpoint_error` must both recur and outlive
an hour before they are called sustained (`MIN_RECURRENCES`, `SUSTAINED_AFTER_SECONDS` in
`internal_call_health_service`). `config_missing` and `unauthorized` skip both, because an unset
variable does not heal.

> **The stated cost.** A deployment with very little login traffic can hold a weather-capable
> failure at `transient`, because the second occurrence that would settle it has not happened. H4 is
> the fallback — `failures_total` keeps rising whatever the run counter does — but no default rule
> reads it, deliberately: an operator who knows their own traffic can set that threshold here
> without a deploy, and nobody can set it from inside the code.
