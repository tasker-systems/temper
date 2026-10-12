//! The erasure policy in force: the `policy/erasure/` manifest, embedded and parsed once.
//!
//! The parser and the model live in temper-core (`temper_core::erasure_policy`), so admin tooling
//! and the CLI can express the shape. The text lives here, because the policy in force is the
//! server's; a client never compiles its own copy.

use std::sync::OnceLock;

use temper_core::erasure_policy::{parse, ErasurePolicy, ManifestFiles};

/// The five manifest files, as shipped in `policy/erasure/`.
pub const FILES: ManifestFiles<'static> = ManifestFiles {
    root: include_str!("../../../policy/erasure/manifest.toml"),
    estate: include_str!("../../../policy/erasure/estate.toml"),
    personal_data: include_str!("../../../policy/erasure/personal-data.toml"),
    sensitivity_scan: include_str!("../../../policy/erasure/sensitivity-scan.toml"),
    resource_erasure: include_str!("../../../policy/erasure/resource-erasure.toml"),
};

/// The erasure policy this build carries, parsed from [`FILES`] on first use.
///
/// # Panics
///
/// If the shipped manifest does not parse. That is a build defect, not a runtime condition: the
/// text is compiled in, so every call in every process would fail the same way.
pub fn in_force() -> &'static ErasurePolicy {
    static POLICY: OnceLock<ErasurePolicy> = OnceLock::new();
    POLICY.get_or_init(|| {
        parse(&FILES).unwrap_or_else(|err| panic!("the shipped erasure-policy manifest: {err}"))
    })
}
