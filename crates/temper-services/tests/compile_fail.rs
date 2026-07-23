#![cfg(feature = "trybuild")]
//! Proof of the enclosure (spec §3.1, Testing): the sealed auth-ladder proofs — these fixtures MUST
//! NOT compile. The thesis is "we were technically doing it right but it took discipline → now it is
//! typed"; a compile-fail proof is that guarantee *demonstrated*, not asserted.
//!
//! Both rungs are covered, and the lower one matters most: `SystemAuthorized` and `SystemAdmin` are
//! each minted from an `AuthenticatedProfile`, so while Level 1 stayed forgeable (public fields, in
//! temper-core, until 2026-07-22) sealing the two above it was decorative — you could forge the
//! input and walk the real gates.
//!
//! Gated behind the `trybuild` feature (OFF by default): trybuild recompiles temper-services + ort in
//! an isolated target dir (~157s), so it must not ride the fast `--workspace` runs. It runs in exactly
//! one place — the dedicated "Seal Proof (trybuild)" CI job — and locally via `cargo make test-trybuild`.
//!
//! The `.stderr` snapshots are committed because trybuild requires them (a missing one is a test
//! failure, not a pass — verified). CI runs `dtolnay/rust-toolchain@stable`, which floats, and
//! trybuild matches stderr exactly, so a rustc reword could red this gate. The fixtures were chosen
//! to lean on rustc's *most stable* diagnostics — E0423 (private tuple-struct field), E0308
//! (mismatched types) and E0451 (private struct fields) — precisely to minimise that. If a toolchain
//! bump ever does reword one, the fix is a one-line regen, not a real regression:
//! `TRYBUILD=overwrite cargo test -p temper-services --test compile_fail`, then review the diff is
//! only wording before committing.

#[test]
fn the_auth_ladder_proofs_are_unforgeable_and_gate_admin_fns() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
