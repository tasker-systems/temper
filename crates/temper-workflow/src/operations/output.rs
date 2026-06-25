//! CommandOutput — the value-plus-events return shape for every Backend method.

use serde::{Deserialize, Serialize};

use super::events::DomainEvent;

/// What a backend returns from a command: the typed value plus any events
/// emitted during execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandOutput<T> {
    pub value: T,
    pub events: Vec<DomainEvent>,
}

impl<T> CommandOutput<T> {
    /// Build a `CommandOutput` with no events. Useful for trivial returns.
    pub fn new(value: T) -> Self {
        Self {
            value,
            events: Vec::new(),
        }
    }

    /// Build a `CommandOutput` with an explicit events vector.
    pub fn with_events(value: T, events: Vec<DomainEvent>) -> Self {
        Self { value, events }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_output_has_empty_events() {
        let out = CommandOutput::new(42_u32);
        assert_eq!(out.value, 42);
        assert!(out.events.is_empty());
    }

    #[test]
    fn with_events_keeps_events() {
        use crate::operations::events::PushDeferReason;
        let events = vec![DomainEvent::PushDeferred {
            reason: PushDeferReason::Offline,
        }];
        let out = CommandOutput::with_events("hello", events);
        assert_eq!(out.value, "hello");
        assert_eq!(out.events.len(), 1);
    }
}
