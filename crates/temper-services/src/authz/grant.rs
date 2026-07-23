//! Grant administration — may this caller administer grants on this subject?
//!
//! The arms and the probe sequence are unchanged from the `grant_authority` this replaces; only
//! the shape moved. In particular the **short-circuit order is load-bearing**: an admin costs one
//! query, a denied delegate costs three, and reordering the branches would quietly make every
//! admin request pay for probes it does not need.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{CogmapId, ProfileId};
use temper_substrate::payloads::{AnchorTable, RefTarget};

use super::{Authorized, ConnectionAuthority, ScopedAuthority};
use crate::error::{ApiError, ApiResult};
use crate::services::access_service::{
    cogmap_write_requires_admin, is_system_admin, profile_can_grant, GrantAuthority,
};
use crate::services::machine_authz::AuthorizedGrant;

#[async_trait]
impl ScopedAuthority for GrantAuthority {
    type Subject = RefTarget;

    async fn resolve(pool: &PgPool, caller: ProfileId, subject: RefTarget) -> ApiResult<Self> {
        if is_system_admin(pool, caller).await? {
            return Ok(GrantAuthority::SystemAdmin);
        }

        // Structural escalation guard (plan Task 5b.4). `require_cogmap_write_admin` keeps the
        // reserved L0 kernel and gating-team-joined maps admin-only, but the grant path never
        // consulted it — so a non-admin `can_grant` holder could mint `can_write` on the kernel,
        // reaching by the grant axis exactly what the write axis forbids. `machine_authz`'s own
        // tests seed such a row, so the state is reachable, not hypothetical. Admins already
        // returned above, so a map in the admin-only regime denies here regardless of any
        // `can_grant` the caller holds.
        if subject.kind == AnchorTable::Cogmaps
            && cogmap_write_requires_admin(pool, CogmapId(subject.id)).await?
        {
            return Ok(GrantAuthority::None);
        }

        Ok(
            if profile_can_grant(pool, caller, subject.kind.as_str(), subject.id).await? {
                GrantAuthority::Delegated
            } else {
                GrantAuthority::None
            },
        )
    }

    fn is_denial(&self) -> bool {
        matches!(self, GrantAuthority::None)
    }

    fn denial() -> ApiError {
        ApiError::Forbidden
    }
}

/// Proof that a subject is being **born in this transaction** — that no prior authority over it can
/// exist, because it did not exist.
///
/// This is the genesis exception, and it is a real exception: the cogmap creator seed
/// (`db_backend.rs`, `cogmap_genesis`) legitimately has nothing to gate against. The obvious
/// accommodation would be a forge on `Authorized` — `Authorized::at_genesis(..)` — and it is refused
/// on purpose. `Authorized<A>` is generic, so a hatch on it hands **every** domain a bypass in order
/// to solve **one** domain's problem. This type is that bypass confined to its own name.
///
/// **Honest limit, stated rather than implied: this cannot *prove* freshness.** Nothing stops a
/// caller minting one for an id that has existed for a year. What it buys is confinement and
/// visibility — one narrow `pub(crate)` type, a constructor whose name reads as the claim being
/// made, and the call-site-count test below, so a second construction site fails a test instead of
/// passing review unnoticed. Bootstrapping is hard to model without exceptions; the risk is owned
/// somewhere, and this is the smallest blast radius available.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BornSubject<S: Copy> {
    subject: S,
}

impl<S: Copy> BornSubject<S> {
    /// Claim that `subject` is being created in the current transaction.
    ///
    /// Named as an assertion because calling it *is* one: you are stating that no prior authority
    /// over this id can exist. If that is not true at your call site, this is the wrong type.
    pub(crate) fn minted_in_this_transaction(subject: S) -> Self {
        Self { subject }
    }

    pub(crate) fn subject(&self) -> S {
        self.subject
    }
}

/// The **COMPLETE set of ways a `kb_access_grants` row may be born.** There is no ungated path;
/// adding a fifth way means adding an arm, in a diff, under review.
///
/// The four arms are four *different* gates, and that is correct rather than sloppy. The connection
/// path documents why it must not route through grant authority — *"the `can_grant` seam has no
/// bootstrap holder for a connection subject"* — so this design does not force one gate onto every
/// caller. It enumerates the legitimate warrants and makes the primitive unreachable without one.
///
/// Its counterpart on the removal axis is `RevokeWarrant`; the two are deliberately separate types
/// (spec §2.5) because revocation is weaker-gated on purpose.
#[derive(Debug)]
pub(crate) enum GrantWarrant<'a> {
    /// Human/API grant administration — minted by `authorize_capability_grant`, which establishes
    /// the authority arm **and** applies attenuation (a delegate may not confer more than they
    /// hold).
    Administered(&'a Authorized<GrantAuthority>),
    /// Machine-registration reach, contained against the registrar's own — minted by
    /// `machine_authz::authorize_registration`. One arm per ROW, not per reach: see
    /// `AuthorizedGrant`.
    MachineReach(&'a AuthorizedGrant),
    /// Connection read-reach — minted by `ConnectionAuthority`: authority over the connection, plus
    /// a manage-capable role on the team receiving the reach.
    ConnectionReach(&'a Authorized<ConnectionAuthority>),
    /// Creator seed at cogmap genesis. The only arm backed by no authority check at all, because at
    /// genesis there is no prior subject to hold authority over — see `BornSubject`, including its
    /// honest limit.
    Birth(&'a BornSubject<RefTarget>),
}

impl GrantWarrant<'_> {
    /// **The subject of the row this warrant authorizes.** `insert_grant` reads it from here and
    /// takes no subject argument at all — not "must match the gate", but nothing to match, because
    /// there is one spelling.
    ///
    /// No `_ =>`: a fifth way to mint a grant must be a compile error here, which is the property
    /// the enum exists to buy.
    pub(crate) fn subject(&self) -> RefTarget {
        match self {
            GrantWarrant::Administered(proof) => proof.subject(),
            // A machine reach grant is always over a cogmap — `apply_reach` writes
            // `subject_table = 'kb_cogmaps'`, and the proof carries the id.
            GrantWarrant::MachineReach(grant) => RefTarget {
                kind: AnchorTable::Cogmaps,
                id: grant.cogmap_id(),
            },
            // The proof's subject is the (connection, team) pair; the grant's SUBJECT is the
            // connection — the team is its principal, which `insert_grant` still takes separately
            // because a principal is not a scope.
            GrantWarrant::ConnectionReach(proof) => RefTarget {
                kind: AnchorTable::Connections,
                id: proof.subject().connection_id,
            },
            GrantWarrant::Birth(born) => born.subject(),
        }
    }
}

/// Type a wire-supplied `subject_table` string, denying rather than erroring on an unknown value.
///
/// **`None` means denied, not malformed.** That is deliberate parity: before this existed, an
/// unrecognized table string was passed straight to the `can(...)` SQL predicate, which matched no
/// row and returned false — a 403. Raising a 400 here instead would be a behavior change on a path
/// that has no live caller to change it for.
///
/// It has no live caller because every surface *injects* the table as a literal
/// (`handlers/resources.rs:430` and `:466`, `handlers/cognitive_maps.rs:365` and `:400`, and the
/// MCP equivalents) and a DB CHECK bounds the column to those same four values
/// (`migrations/20260714000020_connection_reach_grants.sql:44`). The `String` on the wire type is a
/// stringly-typed spelling of a set the callers already know statically; this function is the one
/// place that has to cope with that, and it copes by denying.
pub(crate) fn wire_subject(subject_table: &str, subject_id: Uuid) -> Option<RefTarget> {
    // No `_ =>` over the admitted set: adding a table to the CHECK constraint should surface here
    // as a decision, not resolve silently to a denial.
    let kind = match subject_table {
        "kb_resources" => AnchorTable::Resources,
        "kb_contexts" => AnchorTable::Contexts,
        "kb_cogmaps" => AnchorTable::Cogmaps,
        "kb_connections" => AnchorTable::Connections,
        _ => return None,
    };
    Some(RefTarget {
        kind,
        id: subject_id,
    })
}

#[cfg(test)]
mod tests {
    /// **`BornSubject` has exactly ONE construction site, and that number is the point.**
    ///
    /// The type cannot verify its own claim (see its doc comment) — what it offers instead is that
    /// every place claiming "this subject is brand new" is countable, and counted here. Modelled on
    /// `temper_principal::admission`'s `admit_reads_standing_and_nothing_else`, and it fails for the
    /// same reason that one does: because the shape of the code changed, not because the number
    /// drifted.
    ///
    /// **If this fails, the fix is never "bump the number."** A second construction site means a
    /// second path asserting the genesis exception, and that assertion is exactly what nobody is
    /// checking. Go read the new site and satisfy yourself that its subject really is minted in the
    /// same transaction — then, and only then, change the count in the same diff that added it, so a
    /// reviewer sees both halves together.
    ///
    /// The one legitimate site is the cogmap creator seed in
    /// `backend/db_backend.rs`'s `cogmap_genesis`, whose own comment already argues the claim:
    /// *"`born_cogmap` is always a freshly-minted id that cannot already carry a grant."*
    #[test]
    fn born_subject_has_exactly_one_construction_site() {
        let src_root = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let count = count_constructions(std::path::Path::new(src_root));
        assert_eq!(
            count, 1,
            "BornSubject::minted_in_this_transaction is constructed at {count} sites; expected 1 \
             (the cogmap creator seed). Read this test's doc comment before changing the number."
        );
    }

    /// Walk `src/` counting occurrences of the constructor, skipping this file — its own doc
    /// comments name the constructor and would otherwise count themselves.
    fn count_constructions(dir: &std::path::Path) -> usize {
        let mut total = 0;
        for entry in std::fs::read_dir(dir).expect("read src dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                total += count_constructions(&path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.file_name() != Some(std::ffi::OsStr::new("grant.rs"))
            {
                let body = std::fs::read_to_string(&path).expect("read source file");
                total += body
                    .matches("BornSubject::minted_in_this_transaction")
                    .count();
            }
        }
        total
    }
}
