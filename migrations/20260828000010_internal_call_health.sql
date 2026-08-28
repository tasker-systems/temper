-- Whether temper-cloud's fail-open internal calls are reaching temper-api at all.
--
-- The three ways an IdP membership reconcile fails to happen are not equally visible. An error
-- INSIDE the endpoint reaches an operator: temper-api carries opentelemetry and ships spans to
-- Tempo. An assertion carrying no group signal is recorded as
-- `kb_saml_principal_reconcile.last_skipped_at` (20260827000030). The third -- temper-cloud never
-- reaching the endpoint at all -- was swallowed whole by the fail-open catch at
-- `packages/temper-cloud/src/oauth/endpoints.ts:369-374` into a `logger.error` on a surface with no
-- telemetry pipeline: `packages/temper-cloud/src/logger.ts` is five lines of pino to stdout. This
-- table is where that third case becomes a fact rather than a log line nobody drains.
--
-- WHY THE DATABASE AND NOT TELEMETRY FROM temper-cloud. The failure being reported IS an outbound
-- HTTPS call from a Vercel function failing -- transport, DNS, timeout, a rejected signature. An
-- OTLP export is the same shape over the same egress, so the reporter would share a failure domain
-- with the thing it reports on, and would additionally put a flush on the authentication path.
-- Postgres is the one dependency proven reachable at the moment of failure: the same request wrote
-- `kb_saml_replay` through the same client two statements earlier (`handleSamlAcs`, endpoints.ts).
--
-- THE GRAIN IS THE CHANNEL, NOT THE PRINCIPAL, and this is the whole reason the table is shaped
-- this way. Every failure it records is SYSTEMIC: an unset `INTERNAL_RECONCILE_URL`, a secret the
-- API disagrees with, a host that will not resolve. None of them is about one principal -- when
-- they occur they occur for everyone -- and `kb_saml_principal_reconcile` structurally cannot hold
-- them anyway, since it is keyed on `profile_id` and obtaining one is ANOTHER call to temper-api
-- (`/internal/principal/resolve`) that fails on exactly the path where these failures occur.
--
-- `channel` rather than a single row, because the reconcile call has a sibling: the principal
-- resolve beside it (endpoints.ts:381-395) is fail-open on the same rule and equally invisible.
-- Only `saml_reconcile` is wired by the binary that lands with this migration. The column is a key
-- choice, not a feature -- taking the second channel later must not require altering this table,
-- because an ALTER on a table temper-cloud writes to on the login path is a deploy-ordering hazard
-- and this one is avoidable for the cost of one TEXT column.
--
-- WHAT A SUCCESS CLEARS, AND WHAT IT DOES NOT. `failing_since` and `consecutive_failures` describe
-- the CURRENT unbroken run of failures, so a success ends them. `last_failure_at`,
-- `last_failure_cause` and `last_failure_detail` are historical -- they describe the last failure
-- that happened, not a live condition -- so a success leaves them alone. Collapsing the two kinds
-- into one would mean either losing the forensic record on the first success after an outage, or
-- reporting a channel as failing because it once failed.
--
-- ORDERING. `additive`: nothing reads or writes this table until the paired binary lands, and both
-- names are new so a lagging binary cannot reach them. The reverse order is NOT safe and the
-- consequence differs from 20260827000030's: the writes here happen inside temper-cloud's OWN
-- try/catch (see the binary), so a binary reaching this table before it exists would fail the
-- INSERT, be swallowed, and leave the channel silent -- observability configured and inert, which
-- is precisely the state this table exists to end. Apply this first.
CREATE TABLE kb_internal_call_health (
    channel              TEXT        PRIMARY KEY,
    last_success_at      TIMESTAMPTZ,
    last_failure_at      TIMESTAMPTZ,
    failing_since        TIMESTAMPTZ,
    consecutive_failures INTEGER     NOT NULL DEFAULT 0,
    failures_total       BIGINT      NOT NULL DEFAULT 0,
    last_failure_cause   TEXT,
    last_failure_detail  TEXT,

    -- A row exists because something happened. Both-NULL is not a state this table has.
    CONSTRAINT kb_internal_call_health_says_something
        CHECK (last_success_at IS NOT NULL OR last_failure_at IS NOT NULL),

    -- CLOSED VOCABULARY, ENFORCED HERE rather than trusted from the writer, because the cause is
    -- not decoration: it is what decides whether one occurrence is conclusive. `config_missing` and
    -- `unauthorized` are deployment facts and are never weather; `transport` and `endpoint_error`
    -- can be. A cause the reader does not recognize would fall to the weather branch and silently
    -- soften a conclusive failure into one awaiting recurrence, so an unknown value must fail the
    -- write rather than reach the read.
    CONSTRAINT kb_internal_call_health_known_cause
        CHECK (last_failure_cause IS NULL OR last_failure_cause IN
               ('config_missing', 'unauthorized', 'transport', 'endpoint_error')),

    -- The three columns describing one failure travel together or not at all.
    CONSTRAINT kb_internal_call_health_failure_is_whole
        CHECK ((last_failure_at IS NULL) = (last_failure_cause IS NULL)),

    -- "Currently failing" has one representation, not two that can disagree. A reader asking
    -- either question gets the same answer because the state cannot be expressed inconsistently.
    CONSTRAINT kb_internal_call_health_run_is_consistent
        CHECK ((failing_since IS NULL) = (consecutive_failures = 0)),

    CONSTRAINT kb_internal_call_health_run_started_before_it_continued
        CHECK (failing_since IS NULL OR last_failure_at IS NULL OR failing_since <= last_failure_at),

    CONSTRAINT kb_internal_call_health_failures_are_not_negative
        CHECK (consecutive_failures >= 0 AND failures_total >= consecutive_failures)
);

COMMENT ON TABLE kb_internal_call_health IS
  'Whether temper-cloud''s fail-open server-to-server calls into temper-api are reaching it, at '
  'grain (channel). Written by temper-cloud from inside the same catch that keeps a failed call '
  'from blocking a login; read by the reconcile-health cron, which is what turns it into a signal '
  'an operator receives. Deliberately NOT per-principal: every failure it records is systemic, and '
  'kb_saml_principal_reconcile cannot hold them because obtaining a profile_id is another call on '
  'the path that is failing.';

COMMENT ON COLUMN kb_internal_call_health.channel IS
  'Which server-to-server call this row is about. ''saml_reconcile'' is the only channel written '
  'today; the sibling principal-resolve call is the same shape and can be added without an ALTER.';

COMMENT ON COLUMN kb_internal_call_health.last_success_at IS
  'When a call on this channel last completed. NULL means none has been RECORDED, which for a '
  'deployment predating this table includes every call that succeeded before there was anywhere to '
  'record one. Not backfilled, for 20260827000030''s reason: the only available value is now(), and '
  'stamping it would assert a success nothing performed.';

COMMENT ON COLUMN kb_internal_call_health.failing_since IS
  'When the current unbroken run of failures began -- NOT when the last failure happened. Cleared '
  'by the next success. Together with consecutive_failures this is what separates a sustained '
  'failure from weather: one failure that has not recurred is not evidence of a stopped channel, '
  'and a run that has both recurred and outlived the window is.';

COMMENT ON COLUMN kb_internal_call_health.consecutive_failures IS
  'Failures since the last success. Zero exactly when the channel is not currently failing.';

COMMENT ON COLUMN kb_internal_call_health.failures_total IS
  'Every failure this channel has ever recorded. Monotonic -- a success does NOT reset it, which '
  'is the whole reason it exists beside consecutive_failures. An INTERMITTENT channel (say a flaky '
  'egress failing half of all logins) never accumulates a run: each success resets '
  'consecutive_failures to 0 and clears failing_since, so the pair reports a healthy channel while '
  'half of all de-provisioning is not happening. This column is the only thing that keeps rising '
  'through that, and it is what an operator query differentiates to see a failure RATE. It '
  'deliberately does not drive the default alert: a rate needs a threshold nobody can calibrate '
  'from here, and thresholds belong in the alert rule where they change without a deploy.';

COMMENT ON COLUMN kb_internal_call_health.last_failure_cause IS
  'Which of the four ways the call failed, from a closed vocabulary the CHECK enforces. '
  '''config_missing'' (requireEnv threw -- an unset INTERNAL_RECONCILE_URL or _SECRET) and '
  '''unauthorized'' (401/403 -- the API rejected the signature) are deployment facts: one '
  'occurrence is conclusive and the reader treats them as sustained immediately. ''transport'' (the '
  'fetch threw or timed out) and ''endpoint_error'' (any other non-2xx) can be weather and must '
  'recur to count. This column is therefore load-bearing for the alert, not descriptive.';

COMMENT ON COLUMN kb_internal_call_health.last_failure_detail IS
  'A bounded, classified detail for the operator -- the name of the missing environment variable, '
  'or the HTTP status. NEVER a raw error string: node-saml errors can embed assertion XML (NameID, '
  'email) in their message, which is why the ACS''s own outer catch logs only err.message and why '
  'this column is filled from the classifier rather than from the error.';

SELECT declare_migration(
    20260828000010,
    'additive',
    'One new table, kb_internal_call_health, at grain (channel), recording whether temper-cloud''s fail-open server-to-server calls into temper-api are reaching it (task 01a0453a-6272-7420-83d3-0529357464b2, goal 01a035eb-3aea-7ea0-9dd3-f13acdf8cb36, clause a-de-provisioning-that-did-not-happen-is-visible-to-an-operator). It closes the second of the three ways a reconcile fails to happen: an error inside the endpoint already reaches Tempo, and an assertion carrying no group signal is recorded by 20260827000030, but temper-cloud never REACHING the endpoint -- transport, a rejected signature, requireEnv throwing on an unset INTERNAL_RECONCILE_URL or INTERNAL_RECONCILE_SECRET -- was swallowed by the fail-open catch into a logger.error on a surface whose entire telemetry pipeline is five lines of pino to stdout. THE CARRIER IS THE DATABASE AND NOT TELEMETRY FROM temper-cloud, deliberately: the failure being reported is an outbound HTTPS call from a Vercel function failing, and an OTLP export is the same shape over the same egress, so the reporter would share a failure domain with the thing it reports on, while additionally putting a flush on the authentication path -- which the fail-open decision (spec 3.8) forbids regressing. Postgres is the one dependency proven reachable at the moment of failure, since the same request wrote kb_saml_replay through the same client two statements earlier. THE GRAIN IS THE CHANNEL, NOT THE PRINCIPAL: every failure recorded here is systemic -- an unset variable or a disagreeing secret fails for everyone at once -- and kb_saml_principal_reconcile structurally cannot hold them, being keyed on profile_id whose resolution is another call on the very path that is failing. Adding per-principal fields to the fail-open log line was considered and rejected for that reason, separately from an earlier security review rejecting idp_key on that line as unnecessary. The channel column is a key choice rather than a feature: the sibling principal-resolve call is fail-open on the same rule and equally invisible, and taking it up later must not require an ALTER on a table temper-cloud writes to on the login path. A success clears failing_since and consecutive_failures, which describe the current unbroken run, and deliberately leaves last_failure_at, last_failure_cause and last_failure_detail, which are historical -- and also leaves failures_total, which is monotonic and exists precisely because the run counter cannot see an intermittent channel: a flaky egress failing half of all logins never accumulates a run, so consecutive_failures and failing_since would report a healthy channel while half of all de-provisioning does not happen, and only an ever-rising total survives that -- collapsing the two kinds would either lose the forensic record on the first success after an outage or report a channel as failing because it once failed. The cause vocabulary is enforced by CHECK rather than trusted from the writer because it is load-bearing for the alert and not descriptive: config_missing and unauthorized are deployment facts that one occurrence settles, transport and endpoint_error can be weather and must recur, so an unrecognized value would fall to the weather branch and silently soften a conclusive failure into one awaiting recurrence. last_failure_detail is filled from the classifier and never from a raw error string, because node-saml errors can embed assertion XML. Not backfilled, for 20260827000030''s reason: the only available value is now() and it would assert a success nothing performed. Nothing reads or writes the table until the paired binary lands and both names are new, which is what makes this safe to apply first; the reverse order is not safe and its consequence differs from 20260827000030''s, because these writes sit inside temper-cloud''s own try/catch and a binary reaching a missing table would have its INSERT swallowed, leaving the channel silent -- observability configured and inert, which is exactly the state this table exists to end.'
);
