#![cfg(feature = "artifact-tests")]
//! Guards the single-source invariant: the committed install migration must equal the generator's
//! output from the shared artifact body. If artifact SQL changes without regenerating, this fails.
mod common;
use std::process::Command;

#[test]
fn install_migration_matches_generated() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let out = Command::new("bash")
        .arg(format!("{root}/tools/gen-install-migration.sh"))
        .arg("--stdout")
        .output()
        .expect("generator runs");
    assert!(
        out.status.success(),
        "generator failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let generated = String::from_utf8(out.stdout).unwrap();
    let committed = common::read_latest_install_migration(root);
    assert_eq!(
        generated, committed,
        "install migration is stale — run `cargo make gen-install-migration`"
    );
}
