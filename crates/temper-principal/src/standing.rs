use serde::{Deserialize, Serialize};

/// The one authoritative standing state for a principal (spec D2).
///
/// Five states plus absence. **Absence is not a variant** — a principal with no standing row is
/// denied structurally (spec §7 obligation 1), which is what makes D7's connection-profile safety
/// hold by construction rather than by a check someone can forget.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admission.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Provisioned, never granted. Where every door lands (D11).
    Denied,
    /// Has asked for system access. Still denied, but the refusal can say so. Human-only (D14).
    Requested,
    /// May use the instance.
    Approved,
    /// Was granted and lost it. Only an admin act leaves this state (D15).
    Revoked,
    /// The principal itself is disabled. Prior standing is recoverable from the log.
    Deactivated,
}

impl Standing {
    /// The database literal for this state. Paired with [`Standing::parse`]; the two are one
    /// contract and the round-trip is tested.
    pub fn as_str(&self) -> &'static str {
        match self {
            Standing::Denied => "denied",
            Standing::Requested => "requested",
            Standing::Approved => "approved",
            Standing::Revoked => "revoked",
            Standing::Deactivated => "deactivated",
        }
    }

    /// Total parse. **`None` refuses; it never defaults** (spec §7 obligation 2).
    ///
    /// The column can hold a value this binary does not know during a rolling deploy or after a
    /// rollback. Returning a default here would admit an unknown state, and it would only bite
    /// inside a deploy window — which is why this is the obligation most likely to be got wrong.
    ///
    /// The `_ => None` below is the **one** permitted catchall in this crate: it is on the *input*
    /// side (`&str`, an unbounded set) and its result is a refusal. Spec §7 obligation 3's
    /// prohibition is on catchalls in matches over [`Standing`] itself, where a new variant must
    /// force a compile error at every decision site.
    pub fn parse(raw: &str) -> Option<Standing> {
        match raw {
            "denied" => Some(Standing::Denied),
            "requested" => Some(Standing::Requested),
            "approved" => Some(Standing::Approved),
            "revoked" => Some(Standing::Revoked),
            "deactivated" => Some(Standing::Deactivated),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_every_known_state() {
        for s in [
            Standing::Denied,
            Standing::Requested,
            Standing::Approved,
            Standing::Revoked,
            Standing::Deactivated,
        ] {
            assert_eq!(
                Standing::parse(s.as_str()),
                Some(s),
                "as_str/parse must round-trip; the column literal and the enum are one contract"
            );
        }
    }

    #[test]
    fn an_unrecognized_state_is_none_never_a_default() {
        // Spec §7 obligation 2: the column can hold a value this binary does not know
        // during a rolling deploy or after a rollback. `None` refuses; it never defaults.
        for unknown in ["", "APPROVED", "approved ", "admin", "pending", "🙂"] {
            assert_eq!(
                Standing::parse(unknown),
                None,
                "{unknown:?} must not parse — a default here is a silent grant inside a deploy window"
            );
        }
    }
}
