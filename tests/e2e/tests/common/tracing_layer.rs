//! A test `tracing::Layer` that captures events and span data for assertions.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::span;
use tracing::{Event, Id, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// A captured tracing event with its fields and parent span fields.
#[derive(Debug, Clone)]
pub struct CapturedEvent {
    pub target: String,
    pub level: Level,
    pub fields: HashMap<String, String>,
    pub span_fields: HashMap<String, String>,
}

/// Shared storage for captured events.
pub type CapturedEvents = Arc<Mutex<Vec<CapturedEvent>>>;

/// A `tracing::Layer` that records every event into a shared `Vec`.
pub struct TestTracingLayer {
    events: CapturedEvents,
}

impl TestTracingLayer {
    pub fn new() -> (Self, CapturedEvents) {
        let events: CapturedEvents = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                events: events.clone(),
            },
            events,
        )
    }
}

/// Visitor that collects fields into a `HashMap<String, String>`.
struct FieldCollector(HashMap<String, String>);

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0
            .insert(field.name().to_string(), value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0
            .insert(field.name().to_string(), value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0
            .insert(field.name().to_string(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0
            .insert(field.name().to_string(), value.to_string());
    }
}

impl<S> Layer<S> for TestTracingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &span::Attributes<'_>,
        id: &Id,
        ctx: Context<'_, S>,
    ) {
        let mut fields = FieldCollector(HashMap::new());
        attrs.record(&mut fields);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(fields.0);
        }
    }

    fn on_record(&self, id: &Id, values: &span::Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut exts = span.extensions_mut();
            if let Some(fields) = exts.get_mut::<HashMap<String, String>>() {
                let mut collector = FieldCollector(HashMap::new());
                values.record(&mut collector);
                fields.extend(collector.0);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = FieldCollector(HashMap::new());
        event.record(&mut fields);

        // Walk parent spans to collect span-level fields.
        let mut span_fields = HashMap::new();
        if let Some(scope) = ctx.event_span(event) {
            let span = scope;
            if let Some(exts) = span.extensions().get::<HashMap<String, String>>() {
                span_fields.extend(exts.clone());
            }
            // Walk ancestors
            for ancestor in span.scope().skip(1) {
                if let Some(exts) = ancestor.extensions().get::<HashMap<String, String>>() {
                    for (k, v) in exts {
                        span_fields.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
        }

        let captured = CapturedEvent {
            target: event.metadata().target().to_string(),
            level: *event.metadata().level(),
            fields: fields.0,
            span_fields,
        };

        self.events.lock().unwrap().push(captured);
    }
}
