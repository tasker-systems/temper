//! Shared test helpers for `temper-cli` integration tests.
//!
//! Lives at `tests/common/mod.rs` (rather than `common.rs`) so cargo treats
//! it as a module rather than a stand-alone test binary.

use std::sync::Once;

static AUTH_INIT: Once = Once::new();

/// Isolate the test process from any temper-related environment variables a
/// developer's shell may export.
///
/// Two kinds of isolation:
/// - Points `TEMPER_AUTH_PATH` at a non-existent path so the publish-tail
///   helper finds no token and no-ops without touching
///   `~/.config/temper/auth.json` or making network calls.
/// - Removes `TEMPER_TOKEN` / `TEMPER_PROVIDER` / `TEMPER_DEVICE_ID`. A
///   developer who uses the `temper` CLI in cloud mode has these exported;
///   inherited into a test process they cause auth side-effects. CI has none
///   of these set, which is why the failures only reproduce on configured dev
///   machines.
///
/// Idempotent — runs at most once per test process. The values written are
/// constant (not per-test), so parallel calls converge on the same final
/// state. Tests that need a different auth path can override via
/// `temp_env::with_var` for their scope.
pub fn init_isolated_auth() {
    AUTH_INIT.call_once(|| {
        // SAFETY: Once ensures this closure runs at most once. The values
        // written/removed are constant across the entire test process, so no
        // test observes a torn or shifting value.
        unsafe {
            std::env::set_var(
                "TEMPER_AUTH_PATH",
                "/tmp/temper-cli-tests-no-such-auth-path/auth.json",
            );
            for var in ["TEMPER_TOKEN", "TEMPER_PROVIDER", "TEMPER_DEVICE_ID"] {
                std::env::remove_var(var);
            }
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
        temper_cli::commands::resource::CreateResourceArgs {
            doc_type: "goal",
            title,
            context: Some(context),
            goal: None,
            mode: None,
            effort: None,
            slug: Some(&slug),
            task: None,
            body_flag: None,
            from: None,
            format: temper_cli::format::OutputFormat::Json,
        },
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
        temper_cli::commands::resource::CreateResourceArgs {
            doc_type: "task",
            title,
            context: Some(context),
            goal: goal_slug,
            mode,
            effort,
            slug: Some(&slug),
            task: None,
            body_flag: None,
            from: None,
            format: temper_cli::format::OutputFormat::Json,
        },
    )
    .unwrap();
    slug
}
