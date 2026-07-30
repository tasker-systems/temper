use axum::Json;

use temper_core::types::api::HealthResponse;

use temper_services::error::ApiResult;

#[utoipa::path(
    get,
    path = "/api/health",
    tag = "Health",
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse),
    )
)]
pub async fn health_check() -> ApiResult<Json<HealthResponse>> {
    Ok(Json(HealthResponse {
        status: "ok",
        // Sourced from Cargo at compile time. NOTE: this is the temper-api crate's own
        // version (0.1.0, unchanged since the crate was created), not a deploy identity —
        // `commit` below is the field that answers "what is running here".
        version: env!("CARGO_PKG_VERSION"),
        // `option_env!` resolves at compile time to the value build.rs emitted, or `None`
        // when the build did not know its commit.
        commit: option_env!("TEMPER_BUILD_COMMIT"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_reports_a_commit_slot_that_never_lies() {
        let Json(body) = health_check().await.expect("health check succeeds");

        assert_eq!(body.status, "ok");

        // The commit is Option, not a placeholder string. A build that cannot know its
        // commit — a local `cargo build`, a `cargo install` — must report *absence*,
        // never a plausible-looking value. `unverifiable` is not `wrong`.
        match body.commit {
            None => {}
            Some(sha) => {
                assert!(!sha.is_empty(), "a recorded commit must not be empty");
                assert!(
                    sha.chars().all(|c| c.is_ascii_hexdigit()),
                    "a recorded commit must be a hex sha, got {sha:?}"
                );
            }
        }
    }

    /// The bite. The test above accepts `None`, so it would pass just as well if the
    /// whole build.rs plumbing were dead — `option_env!` on a name nothing emits is a
    /// silent `None`, not a compile error.
    ///
    /// `build.rs` reads `VERCEL_GIT_COMMIT_SHA` at compile time and emits
    /// `TEMPER_BUILD_COMMIT`. Whenever this binary is *compiled and run* in one
    /// environment — what a Vercel build does, and what
    /// `VERCEL_GIT_COMMIT_SHA=$(git rev-parse HEAD) cargo nextest run …` does locally —
    /// the two must agree. This fails if build.rs stops running, if the two env names
    /// drift apart, or if the handler stops reading the baked value.
    ///
    /// Outside such an environment the variable is absent and this is vacuous, which is
    /// why it supplements the test above rather than replacing it.
    #[tokio::test]
    async fn a_build_told_its_commit_reports_that_commit() {
        let Json(body) = health_check().await.expect("health check succeeds");

        let Ok(sha) = std::env::var("VERCEL_GIT_COMMIT_SHA") else {
            return; // Not a commit-carrying build; nothing to correlate against.
        };
        let sha = sha.trim();
        if sha.is_empty() {
            return;
        }

        assert_eq!(
            body.commit.unwrap_or("<none recorded>"),
            sha,
            "this build was told VERCEL_GIT_COMMIT_SHA={sha}, so /api/health must report \
             that commit — a mismatch means build.rs did not run, the TEMPER_BUILD_COMMIT \
             name drifted between build.rs and this handler, or the handler stopped \
             reading the baked value"
        );
    }
}
