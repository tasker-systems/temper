//! CLI diagnostic channel types — structured diagnostics and errors on stdout.
//!
//! The CLI separates stdout (payload) from stderr (human-facing prose). Agents
//! commonly merge both streams (`2>&1 | jq`), so a single stderr line corrupts the
//! JSON payload. These types let diagnostics and errors ride the stdout payload
//! in a structured form an agent can parse, while stderr retains the
//! human-readable rendering for TTY/TOON mode.
//!
//! See task `019fdc7d` and design spec `01a010ec` for the decision record.

use serde::{Deserialize, Serialize};

use crate::types::error_details::ErrorDetails;
use crate::types::precondition::PreconditionFailure;

/// What kind of diagnostic this is. Never `error` — errors are the
/// [`ErrorPayload`] tier, not diagnostics on a
/// success.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "diagnostics.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    /// Something went wrong but the result is still usable — e.g. a truncated
    /// list, a degraded search arm, a cloud-unreachable status fallback.
    Warning,
    /// Noteworthy but not a problem — e.g. a scope that resolved to zero
    /// candidates.
    Info,
}

/// A diagnostic emitted alongside a successful result.
///
/// Additive on the result struct: `#[serde(default,
/// skip_serializing_if = "Vec::is_empty")]` so it vanishes from the wire when
/// empty and existing `jq` queries are unaffected. Present when there is
/// something to say; absent means "nothing to report."
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "diagnostics.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Diagnostic {
    /// `warning` | `info` — what kind of signal this is. Never `error` (errors
    /// are the [`ErrorPayload`] tier, not diagnostics on a success).
    pub level: DiagnosticLevel,
    /// A stable, greppable code (e.g. `"truncated"`, `"wide-degraded"`,
    /// `"scope-empty"`). A caller branches on this, not on message text.
    pub code: &'static str,
    /// Human-readable explanation. May wrap.
    pub message: String,
    /// Optional next-step hint, the same field the API carries per-arm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Structured error payload on stdout when a command fails.
///
/// Replaces the stderr-prose-then-exit-1 path in `main.rs`. The exit code stays
/// non-zero; the explanation is now parseable. An agent branches on `code` or on
/// the exit code — either works.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "diagnostics.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ErrorPayload {
    /// A stable, greppable code derived from the `TemperError` variant — e.g.
    /// `"not-found"`, `"bad-request"`, `"forbidden"`, `"network"`. A caller
    /// branches on this.
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
    /// Optional next-step hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Structured detail, present only for a refused plan and a failed conditional write; skipped
    /// when `None`, so other errors' payloads carry no `details` key. See [`PayloadDetails`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    // `ErrorDetails` derives no `JsonSchema` and `ErrorPayload`'s schema is published nowhere; a
    // consumer of that schema must derive it for `ErrorDetails` before dropping this skip.
    #[cfg_attr(feature = "mcp", schemars(skip))]
    pub details: Option<PayloadDetails>,
}

/// What an [`ErrorPayload`]'s `details` carries.
///
/// Untagged, and unambiguous by required field: a refused plan's list requires `refusals`, a
/// failed conditional write requires `failed`.
///
/// - **`Refusal`**: what the HTTP door puts in `error.details`, the refused plan's list, each
///   entry with its `reason` and `teaches`.
/// - **`Precondition`**: what the HTTP door puts in `error.precondition`, every failed expectation
///   with the value there now (`{"failed": [{tier, key, expected, current}]}`). The CLI caller is
///   the caller the server answered, and the server answers it only to a reader, so the values
///   belong here; the `message` beside it names keys only.
///
/// Not a third [`ErrorDetails`] arm: that union is published in `openapi.json` and pinned, and
/// this payload is the CLI's own.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "diagnostics.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum PayloadDetails {
    Refusal(ErrorDetails),
    Precondition(PreconditionFailure),
}
