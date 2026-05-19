//! Shared test helpers for `temper-cli` integration tests.
//!
//! Lives at `tests/common/mod.rs` (rather than `common.rs`) so cargo treats
//! it as a module rather than a stand-alone test binary.

use std::sync::Once;

static AUTH_INIT: Once = Once::new();

/// Point `TEMPER_AUTH_PATH` at a non-existent path so the publish-tail helper
/// finds no token and no-ops without touching `~/.config/temper/auth.json`
/// or making any network calls.
///
/// Idempotent — runs at most once per test process. The value is a constant
/// (not per-test) so parallel set_var calls converge on the same final state
/// even if the OS sees the writes in any order. Tests that need a different
/// auth path can override via `temp_env::with_var` for their scope.
///
/// Without this, integration tests on a developer machine with a real
/// `~/.config/temper/auth.json` would silently publish test fixtures to the
/// production server.
pub fn init_isolated_auth() {
    AUTH_INIT.call_once(|| {
        // SAFETY: Once ensures this closure runs at most once. The value
        // written is constant across the entire test process, so no test
        // observes a torn or shifting value.
        unsafe {
            std::env::set_var(
                "TEMPER_AUTH_PATH",
                "/tmp/temper-cli-tests-no-such-auth-path/auth.json",
            );
        }
    });
}

/// Create a goal via the canonical `commands::resource::create` path and return
/// its slug. Goals use a plain slugified title (no date prefix).
///
/// Pre-creates the `@me/{context}` directory so `resolve_context_with_fallback`
/// does not redirect to "default" when the context hasn't been used yet.
///
/// This replaces the deleted `commands::goal::create` test-setup helper.
#[allow(dead_code)]
pub fn create_goal(config: &temper_cli::config::Config, context: &str, title: &str) -> String {
    use temper_cli::vault::slugify;
    // Pre-create context dir so `resolve_context_with_fallback` doesn't
    // redirect to "default" before the first write creates it.
    std::fs::create_dir_all(config.vault_root.join("@me").join(context)).unwrap();
    let slug = slugify(title);
    temper_cli::commands::resource::create(
        config,
        "goal",
        title,
        Some(context),
        None,
        None,
        None,
        Some(&slug),
        None,
        "text",
    )
    .unwrap();
    slug
}

/// Create a task via the canonical `commands::resource::create` path and return
/// its slug. Tasks use a date-prefixed slug (`YYYY-MM-DD-{slugified-title}`).
///
/// This replaces the deleted `commands::task::create` / `actions::task::create`
/// test-setup helpers.
#[allow(dead_code)]
pub fn create_task(
    config: &temper_cli::config::Config,
    context: &str,
    title: &str,
    goal_slug: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
) -> String {
    use chrono::Local;
    use temper_cli::vault::slugify;
    let today = Local::now().format("%Y-%m-%d").to_string();
    let slug = format!("{today}-{}", slugify(title));
    temper_cli::commands::resource::create(
        config,
        "task",
        title,
        Some(context),
        goal_slug,
        mode,
        effort,
        Some(&slug),
        None,
        "text",
    )
    .unwrap();
    slug
}

/// Move a task to a new stage via the canonical `commands::resource::update` path.
///
/// This replaces the deleted `commands::task::move_task` test-setup helper.
#[allow(dead_code)]
pub fn move_task_to_stage(
    config: &temper_cli::config::Config,
    slug: &str,
    context: &str,
    stage: &str,
) {
    let params = temper_cli::commands::resource::UpdateParams {
        slug,
        doc_type: Some("task"),
        type_from: None,
        type_to: None,
        context: Some(context),
        context_to: None,
        title: None,
        tags: &[],
        aliases: &[],
        relates_to: &[],
        references: &[],
        depends_on: &[],
        extends: &[],
        preceded_by: &[],
        derived_from: &[],
        stage: Some(stage),
        mode: None,
        effort: None,
        goal: None,
        seq: None,
        branch: None,
        pr: None,
        status: None,
        body: None,
    };
    temper_cli::commands::resource::update(config, &params).unwrap();
}
