//! Proof of the enclosure (spec §3.1, Testing): the sealed `SystemAdmin` proof — these fixtures MUST
//! NOT compile. This PR's whole thesis is "we were technically doing it right but it took discipline →
//! now it is typed"; a compile-fail proof is that guarantee *demonstrated*, not asserted.
//!
//! The `.stderr` snapshots are committed because trybuild requires them (a missing one is a test
//! failure, not a pass — verified). CI runs `dtolnay/rust-toolchain@stable`, which floats, and
//! trybuild matches stderr exactly, so a rustc reword could red this gate. The two fixtures were
//! chosen to lean on rustc's *most stable* diagnostics — E0423 (private tuple-struct field) and E0308
//! (mismatched types) — precisely to minimise that. If a toolchain bump ever does reword one, the fix
//! is a one-line regen, not a real regression: `TRYBUILD=overwrite cargo test -p temper-services
//! --test compile_fail`, then review the diff is only wording before committing.

#[test]
fn system_admin_is_unforgeable_and_gates_admin_fns() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
