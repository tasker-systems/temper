//! Span export: the part of telemetry the OpenTelemetry SDK does *not* decide for itself.
//!
//! Deliberately small, because most of what a telemetry config usually contains is already owned by
//! the SDK and re-implementing it would create two copies that drift:
//!
//! | Concern | Owner | Evidence |
//! |---|---|---|
//! | protocol, endpoint, headers, timeout | **the SDK** | `SpanExporter::builder().build()` resolves `OTEL_EXPORTER_OTLP_(TRACES_)*`, signal-specific first (`opentelemetry-otlp-0.32.0/src/span.rs:78-93`, `exporter/http/mod.rs:287,738`) |
//! | `service.name`, resource attributes | **the SDK** | `OTEL_SERVICE_NAME` / `OTEL_RESOURCE_ATTRIBUTES` (`opentelemetry_sdk-0.32.1/src/resource/env.rs:80`) |
//! | sampler | **the SDK** | `OTEL_TRACES_SAMPLER{,_ARG}` (`opentelemetry_sdk-0.32.1/src/trace/config.rs:60-61`) |
//! | whether to export at all | **us** | see below |
//! | Vercel deployment identity | **us** | not a thing the SDK could know |
//! | flushing before the sandbox freezes | **us** | see [`force_flush_spans`] |
//!
//! ## Why "is an endpoint configured" is ours and matters
//!
//! The OTLP spec — and therefore the SDK — defaults the endpoint to `http://localhost:4318`. Taken
//! literally that means an unconfigured process *still exports*, at a collector that is not there,
//! on every request, in every local dev shell and every CI job.
//!
//! So export turns on only when an endpoint is **explicitly** configured. With no `OTEL_*` variables
//! at all, this module builds nothing and temper logs to stdout exactly as it did before. That is
//! the promise `docs/guides/open-telemetry-setup.md` makes to anyone running the stack locally, and
//! it is enforced here rather than assumed.
//!
//! ## `OTEL_SDK_DISABLED` is ours because the SDK does not implement it
//!
//! The spec defines it; `opentelemetry_sdk` 0.32.1 contains **zero** occurrences of the name —
//! not in code, not in its own env-var documentation table. So the guide's "turn the exporter off
//! without a deploy" is only true because `export_disabled` implements it here.
//!
//! ## Sampling, and the one way this could violate the trust decision
//!
//! Decision `019f95ff-e216-7dd1-b2aa-a49d20b1cd6c` rule 3: *"Never let an inbound `sampled` flag
//! drive sampling. Record it; do not obey it."*
//!
//! The SDK's default sampler is `ParentBased(AlwaysOn)`
//! (`opentelemetry_sdk-0.32.1/src/trace/config.rs:33`), and `ParentBased` **does** obey a remote
//! parent's sampled flag — that is its entire purpose. We satisfy rule 3 not by configuring a
//! different sampler but because **there is never a remote parent**: nothing here or in
//! [`crate::record_inbound_trace_context`] constructs a `SpanContext` from the inbound header, so
//! the parent branch is unreachable and the root sampler always decides.
//!
//! That is an invariant held by *absence*, which is the fragile kind. `inbound_context_never_becomes_a_parent`
//! in this module's tests pins it, so that someone later "improving" extraction into a real parent
//! trips a test rather than silently handing every caller the ability to force-sample us.

use std::sync::OnceLock;
use std::time::Duration;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions::attribute::{
    DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_INSTANCE_ID, SERVICE_VERSION,
};

/// `cloud.region` and `cloud.provider` are still `semconv_experimental` in
/// `opentelemetry-semantic-conventions` 0.32.1, so importing them would mean enabling that whole
/// feature for two strings. The names are the spec's; spelling them here keeps the experimental
/// surface out of our dependency graph without inventing our own attribute keys.
const CLOUD_REGION: &str = "cloud.region";
const CLOUD_PROVIDER: &str = "cloud.provider";

/// Spec variables that name where traces go. Either one being set is what "configured" means.
const OTLP_ENDPOINT_VARS: [&str; 2] = [
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_ENDPOINT",
];

/// The spec's kill switch, which `opentelemetry_sdk` 0.32 does not implement — so we do.
const OTEL_SDK_DISABLED: &str = "OTEL_SDK_DISABLED";

/// The CLI's own opt-in to span export. See [`cli_export_opted_in`].
const TEMPER_CLI_TRACE: &str = "TEMPER_CLI_TRACE";

/// The undocumented Vercel in-sandbox collector variables. Logged, never designed around: research
/// `019f943a` §5e records that the documentation for this path has been withdrawn and that nothing
/// states what would be listening. If one of these ever shows up in a real deployment's log line,
/// that is the signal to investigate — not a code path to build speculatively.
const VERCEL_COLLECTOR_VARS: [&str; 3] = [
    "VERCEL_OTEL_ENDPOINTS",
    "VERCEL_OTEL_ENDPOINTS_PORT",
    "VERCEL_OTEL_ENDPOINTS_PROTOCOL",
];

/// The installed provider, kept so the request path can flush it.
///
/// Process-global for the same reason the `tracing` subscriber it accompanies is: there is exactly
/// one per process, installed once at startup. Threading a handle from `main` through `create_app`
/// into a tower layer would change the signature of every router constructor and every test that
/// builds one, to express something that is a process singleton either way.
static PROVIDER: OnceLock<SdkTracerProvider> = OnceLock::new();

/// True when `OTEL_SDK_DISABLED` is set to `true` (case-insensitive), per the spec.
///
/// Any other value — including `1`, `yes`, and typos — leaves telemetry **on**. The spec names
/// exactly one true value, and guessing at others would make a typo silently disable observability,
/// which is the failure you least want to debug later.
fn export_disabled() -> bool {
    std::env::var(OTEL_SDK_DISABLED)
        .map(|v| v.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// True when the CLI has been told it may export spans.
///
/// A **second** switch on top of [`configured_endpoint`], and it exists because
/// `OTEL_EXPORTER_OTLP_ENDPOINT` is ambient on a developer's machine. Someone with it exported for
/// another project would otherwise have `temper` start shipping their vault activity to that
/// collector the moment this shipped — surprising, and not theirs to have asked for. The servers need
/// no such switch: a deployment's environment is configured deliberately, per project.
///
/// Same true-value discipline as [`export_disabled`]: exactly `true`, case-insensitive, so a typo
/// leaves export off rather than silently on.
///
/// ## Why environment-only, with no `[cli]` config key
///
/// Author's judgment, not a settled decision — and reversible, so here is the obstacle rather than a
/// verdict. The CLI's `[cli]` section (`CliSection` in temper-core) is the documented home for
/// defaults like `format` and `color`, resolved *flag → env → config → default*, and this switch
/// would belong there by symmetry. But the subscriber must be installed before anything logs, and
/// `temper_core::types::config::load_config_from` itself emits a `tracing::warn!` for a config with
/// validation issues (`crates/temper-core/src/types/config.rs:395-400`). Reading config early enough
/// to feed this would drop that warning at the CLI's default level — trading a user-facing diagnostic
/// for a config key. Adding a `[cli]` key needs that ordering solved first, not just the key defined.
fn cli_export_opted_in() -> bool {
    std::env::var(TEMPER_CLI_TRACE)
        .map(|v| v.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// The configured OTLP endpoint, if any. Signal-specific wins, matching the SDK's own precedence.
fn configured_endpoint() -> Option<String> {
    OTLP_ENDPOINT_VARS
        .iter()
        .find_map(|var| std::env::var(var).ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Resource attributes describing *which deployment* emitted a span.
///
/// `service.name` is deliberately absent: the SDK fills it from `OTEL_SERVICE_NAME`, and setting it
/// here would override the operator's choice with ours. What we add is the Vercel identity the SDK
/// cannot know — and `VERCEL_DEPLOYMENT_ID` in particular is the bridge between a trace and a
/// `dpl_…`, which is how the last production measurement under this goal was correlated at all.
fn deployment_resource() -> Resource {
    let mut attrs = Vec::new();

    if let Ok(env) = std::env::var("VERCEL_ENV") {
        attrs.push(KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, env));
    }
    if let Ok(region) = std::env::var("VERCEL_REGION") {
        attrs.push(KeyValue::new(CLOUD_REGION, region));
        attrs.push(KeyValue::new(CLOUD_PROVIDER, "vercel"));
    }
    if let Ok(deployment) = std::env::var("VERCEL_DEPLOYMENT_ID") {
        attrs.push(KeyValue::new(SERVICE_INSTANCE_ID, deployment));
    }
    attrs.push(KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")));

    Resource::builder().with_attributes(attrs).build()
}

/// Build the tracer provider, or `None` when export is off or cannot be set up.
///
/// **Fail-open is the whole contract here.** A telemetry backend is not a dependency of serving
/// traffic: an unreachable or misconfigured exporter logs a warning and the process runs without
/// export. Refusing to start, or propagating the error to a caller who would `expect()` it, would
/// let an observability vendor take down the API.
fn build_provider() -> Option<SdkTracerProvider> {
    if export_disabled() {
        tracing::info!(
            reason = "OTEL_SDK_DISABLED",
            "span export off; logging to stdout only"
        );
        return None;
    }

    let Some(endpoint) = configured_endpoint() else {
        tracing::debug!(
            "span export off; no OTEL_EXPORTER_OTLP_(TRACES_)ENDPOINT configured. \
             Not defaulting to localhost:4318 — see temper_telemetry::export docs"
        );
        return None;
    };

    // Protocol, endpoint, headers and timeout all come from the spec env vars; `with_http` only
    // pins the transport family so the HTTP/protobuf path is used rather than gRPC.
    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
    {
        Ok(exporter) => exporter,
        Err(error) => {
            tracing::warn!(
                %error,
                %endpoint,
                "OTLP exporter could not be built; continuing without span export"
            );
            return None;
        }
    };

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(deployment_resource())
        .build();

    // A telemetry init should state what it resolved — otherwise "why are there no spans" is
    // answered by redeploying with more logging. This line also happens to answer the probe's
    // question about the Vercel collector variables, which is why they are read here and nowhere
    // else.
    let vercel_collector: Vec<&str> = VERCEL_COLLECTOR_VARS
        .iter()
        .copied()
        .filter(|var| std::env::var(var).is_ok())
        .collect();
    tracing::info!(
        %endpoint,
        protocol = "http/protobuf",
        vercel_collector_vars = ?vercel_collector,
        "span export on"
    );

    Some(provider)
}

/// Install span export and return the `tracing` layer that feeds it.
///
/// Returns `None` when export is off, and `Option<Layer>` is itself a `Layer`, so the caller adds
/// the result unconditionally and the disabled case costs nothing.
pub(crate) fn export_layer<S>(
) -> Option<tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::SdkTracer>>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    let provider = build_provider()?;
    let tracer = provider.tracer("temper");

    // Store before returning: the flush path must never observe a layer that is live while the
    // provider it drains is still unset.
    let _ = PROVIDER.set(provider);

    Some(tracing_opentelemetry::layer().with_tracer(tracer))
}

/// [`export_layer`], but for the CLI: additionally gated on the operator opting in.
///
/// Split from `export_layer` rather than given a boolean parameter because the two callers differ in
/// *what they are*, not in a setting — a server exports whenever an endpoint is configured, the CLI
/// only when its user has said so ([`cli_export_opted_in`]). Returning `None` before
/// [`build_provider`] runs also means an un-opted-in CLI never constructs an exporter, never resolves
/// the OTLP environment, and never logs the "span export on" line.
pub(crate) fn cli_export_layer<S>(
) -> Option<tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::SdkTracer>>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    if !cli_export_opted_in() {
        return None;
    }
    export_layer()
}

/// Flush pending spans, blocking until they are exported or the SDK gives up.
///
/// **Call this after a response is produced, on every request.** Vercel *freezes* the sandbox
/// rather than exiting the process (`vercel_runtime` 2.1.1 `run` is a long-lived hyper server —
/// `src/lib.rs:238-401`), so a batch processor's timer may simply never fire again: spans queued at
/// freeze are lost silently and non-deterministically. Batching is still right — it makes one HTTP
/// request per flush instead of one per span, which is what `SimpleSpanProcessor` would do — but
/// the *timer* cannot be trusted, so we drain it ourselves at a point we control.
///
/// A no-op when export is off, which is what makes it safe to call unconditionally from the
/// transport layer.
pub fn force_flush_spans() {
    let Some(provider) = PROVIDER.get() else {
        return;
    };
    if let Err(error) = provider.force_flush() {
        // Warn, never propagate: a failed flush costs visibility, and turning that into a failed
        // response would trade the thing being observed for the observation of it.
        tracing::warn!(%error, "span flush failed; some spans may be lost");
    }
}

/// Install a provider that exports into `exporter`, for tests that need to observe what was
/// actually exported.
///
/// Behind `test-support` — the repo's existing convention for test-only surface (`test-db`,
/// `test-embed`) — so it cannot be reached from a production build. It exists because the flush
/// seam's correctness is a property of *layer ordering* in the real router, and the only honest
/// way to check that is to run a real request through `create_app` and look at what came out. A
/// replica of the layer stack inside this crate would be testing tower, not temper.
///
/// Returns `false` if a provider is already installed, since [`PROVIDER`] is set once per process.
/// **Batch, not simple, and that is what makes the flush test able to fail.** A
/// `SimpleSpanProcessor` exports on span *close*, so a test using one would see the span whether
/// the flush ran before or after the root span ended — it could not tell a correct layer ordering
/// from a broken one. With batching, an un-flushed span stays in the queue and never reaches the
/// exporter, which is exactly the production failure being guarded against. It also means the test
/// exercises the same processor production uses.
#[cfg(feature = "test-support")]
pub fn install_test_provider(exporter: opentelemetry_sdk::trace::InMemorySpanExporter) -> bool {
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();
    PROVIDER.set(provider).is_ok()
}

/// The `tracing` layer for a test provider installed by [`install_test_provider`].
#[cfg(feature = "test-support")]
pub fn test_export_layer<S>(
) -> Option<tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::SdkTracer>>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    let provider = PROVIDER.get()?;
    Some(tracing_opentelemetry::layer().with_tracer(provider.tracer("temper-test")))
}

/// Shut the provider down, draining anything outstanding.
///
/// For the long-lived server (`temper-api`'s own `main`), which does exit normally. The Vercel
/// functions never reach this — that is what [`force_flush_spans`] is for.
pub fn shutdown_telemetry() {
    let Some(provider) = PROVIDER.get() else {
        return;
    };
    if let Err(error) = provider.shutdown() {
        tracing::warn!(%error, "telemetry shutdown failed");
    }
}

/// How long a flush may block a response before we accept losing spans instead.
///
/// Unused until the measurement described in task `019f943d` says it is needed; recorded here so
/// the number has one home when it is.
#[allow(dead_code)]
const FLUSH_BUDGET: Duration = Duration::from_millis(500);

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec names exactly one true value. Everything else leaves telemetry ON, because a typo
    /// that silently disables observability is the worst possible failure mode for this variable.
    #[test]
    fn only_the_literal_true_disables_the_sdk() {
        temp_env::with_var(OTEL_SDK_DISABLED, Some("true"), || {
            assert!(export_disabled());
        });
        temp_env::with_var(OTEL_SDK_DISABLED, Some("TRUE"), || {
            assert!(export_disabled(), "the spec value is case-insensitive");
        });
        temp_env::with_var(OTEL_SDK_DISABLED, Some(" true "), || {
            assert!(
                export_disabled(),
                "surrounding whitespace is not meaningful"
            );
        });

        for surprising in ["1", "yes", "on", "ture", ""] {
            temp_env::with_var(OTEL_SDK_DISABLED, Some(surprising), || {
                assert!(
                    !export_disabled(),
                    "`{surprising}` must NOT disable telemetry — only the literal `true` does, so a \
                     typo cannot silently blind us"
                );
            });
        }
        temp_env::with_var_unset(OTEL_SDK_DISABLED, || assert!(!export_disabled()));
    }

    /// The SDK would default this to `http://localhost:4318`. If we inherited that, every local
    /// `cargo make run` and every CI job would export at a collector that is not there.
    #[test]
    fn no_endpoint_configured_means_no_export() {
        temp_env::with_vars(
            [
                ("OTEL_EXPORTER_OTLP_ENDPOINT", None::<&str>),
                ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", None),
            ],
            || {
                assert!(
                    configured_endpoint().is_none(),
                    "unset must mean unconfigured, NOT the spec's localhost default"
                );
                assert!(build_provider().is_none());
            },
        );
    }

    /// An empty or whitespace-only value is someone's unset variable, not an endpoint.
    #[test]
    fn a_blank_endpoint_is_not_configured() {
        temp_env::with_vars(
            [
                ("OTEL_EXPORTER_OTLP_ENDPOINT", Some("   ")),
                ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", None),
            ],
            || assert!(configured_endpoint().is_none()),
        );
    }

    /// Signal-specific wins, matching the SDK's own precedence rather than inventing our own.
    #[test]
    fn the_traces_specific_endpoint_wins() {
        temp_env::with_vars(
            [
                ("OTEL_EXPORTER_OTLP_ENDPOINT", Some("http://general:4318")),
                (
                    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                    Some("http://traces:4318"),
                ),
            ],
            || assert_eq!(configured_endpoint().as_deref(), Some("http://traces:4318")),
        );
    }

    /// `OTEL_SDK_DISABLED` outranks a configured endpoint — it is the kill switch, so it has to win.
    #[test]
    fn the_kill_switch_beats_a_configured_endpoint() {
        temp_env::with_vars(
            [
                ("OTEL_EXPORTER_OTLP_ENDPOINT", Some("http://collector:4318")),
                (OTEL_SDK_DISABLED, Some("true")),
            ],
            || assert!(build_provider().is_none()),
        );
    }

    /// `service.name` must be the operator's, from `OTEL_SERVICE_NAME`.
    ///
    /// `Resource::builder()` — as opposed to `builder_empty()` — merges the SDK's own detectors,
    /// which is how the operator's value arrives without us reading the variable at all. The thing
    /// worth pinning is that we never *overwrite* it: each Vercel project sets its own service name
    /// so the three surfaces are distinguishable, and a hardcoded name here would collapse them
    /// into one and make "which deployable emitted this" unanswerable.
    #[test]
    fn the_service_name_is_the_operators_not_ours() {
        temp_env::with_var("OTEL_SERVICE_NAME", Some("temper-mcp-under-test"), || {
            let resource = deployment_resource();
            assert_eq!(
                resource
                    .get(&opentelemetry::Key::from_static_str("service.name"))
                    .map(|v| v.to_string())
                    .as_deref(),
                Some("temper-mcp-under-test"),
            );
        });
    }

    /// The Vercel identity the SDK cannot know — and `service.instance.id` carrying the deployment
    /// id is what lets a span be tied back to a `dpl_…`.
    #[test]
    fn the_deployment_resource_carries_vercel_identity() {
        temp_env::with_vars(
            [
                ("VERCEL_ENV", Some("production")),
                ("VERCEL_REGION", Some("iad1")),
                ("VERCEL_DEPLOYMENT_ID", Some("dpl_test123")),
            ],
            || {
                let resource = deployment_resource();
                let get = |k: &'static str| {
                    resource
                        .get(&opentelemetry::Key::from_static_str(k))
                        .map(|v| v.to_string())
                };
                assert_eq!(
                    get(DEPLOYMENT_ENVIRONMENT_NAME).as_deref(),
                    Some("production")
                );
                assert_eq!(get(CLOUD_REGION).as_deref(), Some("iad1"));
                assert_eq!(get(SERVICE_INSTANCE_ID).as_deref(), Some("dpl_test123"));
            },
        );
    }

    /// Off Vercel — a laptop, a container, CI — the Vercel attributes are simply absent rather than
    /// present-and-empty. An empty `deployment.environment.name` is worse than none: it groups every
    /// non-Vercel deployment under one meaningless bucket.
    #[test]
    fn the_deployment_resource_omits_what_it_cannot_know() {
        temp_env::with_vars(
            [
                ("VERCEL_ENV", None::<&str>),
                ("VERCEL_REGION", None),
                ("VERCEL_DEPLOYMENT_ID", None),
            ],
            || {
                let resource = deployment_resource();
                for absent in [
                    DEPLOYMENT_ENVIRONMENT_NAME,
                    CLOUD_REGION,
                    SERVICE_INSTANCE_ID,
                ] {
                    assert!(
                        resource
                            .get(&opentelemetry::Key::from_static_str(absent))
                            .is_none(),
                        "{absent} must be absent off-Vercel, not empty"
                    );
                }
            },
        );
    }

    /// Flushing with no provider installed is a no-op, not a panic. The transport layer calls this
    /// on every response including in tests, local dev, and CI, where export is off.
    #[test]
    fn flushing_without_a_provider_is_a_no_op() {
        force_flush_spans();
        shutdown_telemetry();
    }

    /// **The gate for decision `019f95ff` rule 3.**
    ///
    /// Builds a real tracer over an in-memory exporter, drives it exactly the way
    /// `apply_transport_layers` does — a root span with the inbound trace fields recorded onto it —
    /// and inspects the span the SDK actually exported.
    ///
    /// Two assertions, and the second is the one with money attached. The exported span must not
    /// carry the caller's trace id (otherwise a stranger names the trace an incident is
    /// investigated under), and it must have no remote parent — because the default sampler is
    /// `ParentBased`, and a remote parent is the *only* route by which an inbound `sampled` flag
    /// could reach our sampling decision and let anyone force every span to export at our cost.
    #[test]
    fn inbound_context_never_becomes_a_parent() {
        use opentelemetry_sdk::trace::InMemorySpanExporter;
        use tracing_subscriber::layer::SubscriberExt as _;

        const INBOUND_TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
        const INBOUND_PARENT_ID: &str = "00f067aa0ba902b7";

        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer = provider.tracer("temper-test");
        let subscriber =
            tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));

        let mut headers = http::HeaderMap::new();
        headers.insert(
            "traceparent",
            // `-01`: upstream says sampled. We must record that and not obey it.
            format!("00-{INBOUND_TRACE_ID}-{INBOUND_PARENT_ID}-01")
                .parse()
                .expect("valid header value"),
        );

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "http_request",
                trace_id = tracing::field::Empty,
                parent_span_id = tracing::field::Empty,
                trace_sampled = tracing::field::Empty,
                vercel_id = tracing::field::Empty,
                vercel_invocation_id = tracing::field::Empty,
            );
            crate::record_inbound_trace_context(&span, &headers);
            let _entered = span.enter();
        });

        provider.force_flush().expect("flush");
        let spans = exporter.get_finished_spans().expect("finished spans");
        let exported = spans.first().expect("the root span was exported");

        assert_ne!(
            exported.span_context.trace_id().to_string(),
            INBOUND_TRACE_ID,
            "temper rooted its span under a caller-supplied trace id — decision 019f95ff rule 1"
        );
        assert!(
            !exported.parent_span_is_remote,
            "the inbound context became a REMOTE PARENT. The default sampler is ParentBased, so \
             this hands any unauthenticated caller control of our sampling decision — \
             decision 019f95ff rule 3. Exported: {exported:#?}"
        );
    }
}
