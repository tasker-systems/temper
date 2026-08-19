//! Bakes the git commit this binary was built from into the binary itself.
//!
//! Vercel sets `VERCEL_GIT_COMMIT_SHA` in the build environment (verified against a real
//! build log, 2026-07-30). Outside a Vercel deploy the variable is absent, and that case
//! is reported as absence — never as a placeholder — so `/api/health` can distinguish
//! "this build did not record a commit" from "this build is at commit X".
//!
//! Design: internal/superpowers/specs/2026-07-30-schema-binary-pairing-design.md § 5.

fn main() {
    // Rebuild when the variable appears, changes, or disappears.
    println!("cargo:rerun-if-env-changed=VERCEL_GIT_COMMIT_SHA");
    println!("cargo:rerun-if-changed=build.rs");

    match std::env::var("VERCEL_GIT_COMMIT_SHA") {
        Ok(sha) if !sha.trim().is_empty() => {
            println!("cargo:rustc-env=TEMPER_BUILD_COMMIT={}", sha.trim());
        }
        _ => {
            // Emit nothing. `option_env!` then resolves to `None`, which is the honest
            // answer; emitting a sentinel string here would make absence indistinguishable
            // from a commit literally named that.
        }
    }
}
