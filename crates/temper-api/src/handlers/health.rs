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

/// What the build was told its commit is, read at *run* time.
///
/// Cargo keeps this in step with the value `build.rs` baked in: `build.rs` declares
/// `rerun-if-env-changed=VERCEL_GIT_COMMIT_SHA`, so changing or clearing the variable
/// re-runs it and recompiles this crate. A build and a run in one `cargo nextest`
/// invocation therefore see the same answer, which is what lets the tests below compare
/// the two and call a disagreement a defect.
///
/// Empty and whitespace-only are folded into `None`, matching `build.rs`'s own guard —
/// the two must agree on what counts as "told a commit" or the correlation would fail on
/// a value neither of them considers real.
#[cfg(test)]
fn commit_this_build_was_told() -> Option<String> {
    let raw = std::env::var("VERCEL_GIT_COMMIT_SHA").ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slot is part of the wire contract, and its *presence* is precisely what the
    /// pre-#584 state lacked: `/api/health` answered `{"status":"ok","version":"0.1.0"}`
    /// with no `commit` key at all, so a caller could not tell "this build recorded no
    /// commit" from "this build predates the field". Absence of the value must be
    /// reported as an explicit `null`; absence of the *key* is the state the clause
    /// `the-commit-serving-an-environment-is-knowable-without-inference` forbids.
    ///
    /// The response is built here rather than read from `health_check()` deliberately.
    /// Taking the ambient one would make this assertion vacuous in exactly the
    /// environment we most want it live — a build that *was* told its commit serializes
    /// `Some`, and `#[serde(skip_serializing_if = "Option::is_none")]` drops only `None`.
    /// That one-line attribute reads as a tidy-up, silently restores the ambiguity the
    /// field was added to remove, and is the regression this test exists to red.
    #[test]
    fn an_unrecorded_commit_serializes_as_null_rather_than_vanishing_from_the_body() {
        let wire = serde_json::to_value(HealthResponse {
            status: "ok",
            version: "0.0.0-test",
            commit: None,
        })
        .expect("HealthResponse serializes");

        let object = wire.as_object().expect("the health body is a JSON object");

        assert!(
            object.contains_key("commit"),
            "a build that recorded no commit must still carry the `commit` key — dropping \
             it returns /api/health to the pre-#584 shape, where absence and \
             predates-the-field are indistinguishable to a caller; got {wire}"
        );
        assert!(
            object["commit"].is_null(),
            "an unrecorded commit is `null`, never a placeholder string: `we cannot tell` \
             must not render as `we checked`; got {wire}"
        );
    }

    /// A recorded commit is a commit — never an empty string, never a sentinel that
    /// happens to look like one. `unverifiable` is not `wrong`, and it is also not
    /// something to paper over with a plausible value.
    ///
    /// This one is legitimately conditional — there is no value to shape-check when the
    /// build recorded none — but that is only tolerable because the condition is live
    /// somewhere by construction: the Unit CI job builds with a commit, so the body below
    /// runs on every push. A conditional assertion whose condition holds nowhere is the
    /// defect this module was rewritten to remove, and it is worth saying out loud beside
    /// the one assertion here that is still guarded.
    #[tokio::test]
    async fn a_recorded_commit_is_a_hex_sha_and_never_a_placeholder() {
        let Json(body) = health_check().await.expect("health check succeeds");

        assert_eq!(body.status, "ok");

        if let Some(sha) = body.commit {
            assert!(!sha.is_empty(), "a recorded commit must not be empty");
            assert!(
                sha.chars().all(|c| c.is_ascii_hexdigit()),
                "a recorded commit must be a hex sha, got {sha:?}"
            );
        }
    }

    /// The bite, and it can no longer decline to bite.
    ///
    /// This assertion is **total**: every environment lands on a branch that asserts
    /// something. The previous form returned early when `VERCEL_GIT_COMMIT_SHA` was
    /// unset, which is every CI run — so a well-designed witness fired nowhere, and
    /// "every job runs it" read as coverage while no job could fail it.
    ///
    /// The two branches are each live somewhere, by construction rather than by luck:
    ///
    /// - **told** — the *Unit* CI job supplies a fixed synthetic sha, so this branch runs
    ///   there on every push. It fails if `build.rs` stops running, if the
    ///   `TEMPER_BUILD_COMMIT` name drifts between `build.rs` and the handler, or if the
    ///   handler stops reading the baked value.
    /// - **not told** — every other environment: a developer's machine, the *Integration*
    ///   and *Artifacts* CI jobs, `cargo install`. This branch is an assertion, not a
    ///   shrug: it fails if `build.rs` ever starts emitting a sentinel for a build that
    ///   was told nothing, which is the same lie as a wrong commit wearing a humbler
    ///   costume.
    ///
    /// What it still does not witness: that a *deployed* environment answers. That is
    /// stated in the goal register's coverage row rather than implied by this test's
    /// existence.
    #[tokio::test]
    async fn the_binary_reports_the_commit_its_build_was_told_and_absence_when_it_was_not() {
        let Json(body) = health_check().await.expect("health check succeeds");

        match (commit_this_build_was_told(), body.commit) {
            (Some(told), Some(reported)) => assert_eq!(
                reported, told,
                "this build was told VERCEL_GIT_COMMIT_SHA={told}, so /api/health must \
                 report that commit — a mismatch means build.rs did not run, the \
                 TEMPER_BUILD_COMMIT name drifted between build.rs and this handler, or \
                 the handler stopped reading the baked value"
            ),
            (Some(told), None) => panic!(
                "this build was told VERCEL_GIT_COMMIT_SHA={told} and /api/health reports \
                 no commit at all — the bake is dead: either build.rs did not run, or it \
                 emitted a name this handler does not read"
            ),
            (None, Some(reported)) => panic!(
                "this build was told no commit, yet /api/health reports {reported:?} — a \
                 build that cannot know its commit must report absence, never a \
                 plausible-looking value"
            ),
            (None, None) => {}
        }
    }
}
