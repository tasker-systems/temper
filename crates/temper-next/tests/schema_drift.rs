#![cfg(feature = "artifact-tests")]
//! Semantic drift guard: applying the committed temper_next migration lineage (the frozen install
//! migration + every append-only forward delta) reconstructs the same schema as a fresh load of the
//! artifact body (`00_namespace_reset` + `01_schema` + `02_functions`).
//!
//! This replaces the former byte-for-byte guard that compared a single generator-emitted install
//! migration against the artifact. That contract broke in WS6 chunk 4c: the install migration is
//! frozen once merged+applied (sqlx checksum-tracks applied migrations, so editing its bytes breaks
//! `sqlx::migrate!` at API boot on any persistent DB), yet the artifact kept evolving. The honest
//! invariant is now "the migrations, applied in order, rebuild the artifact" — proven semantically by
//! comparing normalized schema fingerprints, not file text.
mod common;

#[test]
fn migrations_reconstruct_artifact_schema() {
    // Build A — the artifact as the design-master defines it (fresh load of 00+01+02).
    common::reset_artifact();
    let from_artifact = common::namespace_fingerprint();

    // Build B — the migration lineage as a real deploy applies it (drop, then install + deltas).
    common::apply_temper_next_migrations();
    let from_migrations = common::namespace_fingerprint();

    assert_eq!(
        from_artifact, from_migrations,
        "temper_next migrations have drifted from the artifact. A schema-artifact change must land as \
         an append-only forward migration (migrations/<ts>_temper_next_*.sql) that rebuilds the new \
         shape — NEVER by editing the frozen install migration. Add or adjust a forward migration so \
         the lineage reconstructs schema-artifact/01_schema.sql + 02_functions.sql."
    );
}
