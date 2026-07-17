use tempfile::TempDir;

/// Build a `Config` plus the environment overrides skill tests need, without
/// touching the process environment directly. Callers wrap their test body in
/// `temp_env::with_vars(env, || { ... })` so the overrides are scoped, restored
/// on exit (even on panic), and serialized against other env-mutating tests.
///
/// Two overrides matter:
/// - `TEMPER_GLOBAL_CONFIG` — `skill::generate` / `compute_config_hash` read the
///   global config from here.
/// - `HOME` — `skill::install` writes the command wrapper to
///   `~/.claude/commands/temper.md` via `dirs::home_dir()`. Pointing `HOME` at
///   the tempdir keeps that write inside the test's isolated space instead of
///   the shared real home, which otherwise causes parallel skill tests to race
///   (each test's config hash differs, so concurrent installs clobber each
///   other) and pollutes the developer's actual command wrapper.
fn test_config_with_global(
    dir: &TempDir,
) -> (temper_cli::config::Config, Vec<(String, Option<String>)>) {
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();

    // skill::generate reads global_config_path(), so we need a real config file
    let config_path = dir.path().join("global-config.toml");
    let vault_path = dir.path().to_string_lossy();
    std::fs::write(
        &config_path,
        format!(
            r#"[vault]
path = "{vault_path}"

[sync.subscriptions]
contexts = ["myapp"]

[skill]
output = "~/.claude/skills/temper"
framework = "superpowers"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "test"
audience = "https://temperkb.io/api"
scopes = ["openid"]
"#
        ),
    )
    .unwrap();

    let env = vec![
        (
            "TEMPER_GLOBAL_CONFIG".to_string(),
            Some(config_path.to_string_lossy().into_owned()),
        ),
        (
            "HOME".to_string(),
            Some(dir.path().to_string_lossy().into_owned()),
        ),
    ];

    let config = temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["myapp".to_string()],
        subscriptions: Vec::new(),
        skill_output: dir.path().join("skill-output"),
        profile_slug: None,
    };

    (config, env)
}

#[test]
fn test_skill_generate_produces_valid_content() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        let content = temper_cli::commands::skill::generate(&config).unwrap();
        // generate() now returns reference.md content (generated from clap)
        assert!(content.contains("temper"));
        assert!(content.contains("# CLI Reference"));
        assert!(content.contains("## Commands"));
    });
}

#[test]
fn test_skill_install_writes_directory() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        let skill_dir = dir.path().join("skill-output");
        let report = temper_cli::commands::skill::install(&config, &skill_dir).unwrap();
        assert!(
            !report.is_no_op(),
            "first install into empty dir should report changed files"
        );
        assert_eq!(report.changed.len(), report.total);

        assert!(skill_dir.join("SKILL.md").exists());
        assert!(skill_dir.join("reference.md").exists());
        assert!(skill_dir.join("subagent-guidance.md").exists());
        assert!(skill_dir.join("plan-verification.md").exists());
        assert!(skill_dir.join("implementation-grounding.md").exists());
        assert!(skill_dir.join("session-lifecycle.md").exists());
        assert!(skill_dir.join("cognitive-maps.md").exists());
        assert!(skill_dir.join("workflows/build-small.md").exists());
        assert!(skill_dir.join("workflows/build-medium.md").exists());
        assert!(skill_dir.join("workflows/build-large.md").exists());
        assert!(skill_dir.join("workflows/plan-small.md").exists());
        assert!(skill_dir.join("workflows/plan-medium.md").exists());
        assert!(skill_dir.join("workflows/plan-large.md").exists());
        assert!(skill_dir.join("guidance").is_dir());
    });
}

#[test]
fn test_skill_install_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        let skill_dir = dir.path().join("skill-output");
        temper_cli::commands::skill::install(&config, &skill_dir).unwrap();
        let second = temper_cli::commands::skill::install(&config, &skill_dir).unwrap();
        assert!(
            second.is_no_op(),
            "second install with no changes should be a no-op, got: {:?}",
            second.changed
        );

        // Mutating one file should make the next install report exactly that file.
        let mutated = skill_dir.join("subagent-guidance.md");
        std::fs::write(&mutated, "stale\n").unwrap();
        let third = temper_cli::commands::skill::install(&config, &skill_dir).unwrap();
        assert_eq!(third.changed, vec!["subagent-guidance.md".to_string()]);
    });
}

#[test]
fn test_skill_generate_includes_reference_sections() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        // generate() now returns reference.md with generated commands and footer
        let content = temper_cli::commands::skill::generate(&config).unwrap();
        assert!(
            content.contains("## Invocation"),
            "should contain invocation section"
        );
        assert!(
            content.contains("## Task Stages"),
            "should contain task stages footer"
        );
    });
}

#[test]
fn test_skill_generate_includes_command_table() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        // generate() now returns reference.md with the generated command table
        let content = temper_cli::commands::skill::generate(&config).unwrap();
        assert!(content.contains("| Command | Syntax |"));
        assert!(content.contains("| init |"));
        assert!(content.contains("| search |"));
    });
}

#[test]
fn test_skill_generate_includes_task_commands() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        // generate() now returns reference.md with resource subcommands
        let content = temper_cli::commands::skill::generate(&config).unwrap();
        assert!(content.contains("| resource create |"));
        assert!(content.contains("| resource list |"));
        assert!(content.contains("--mode"));
        assert!(content.contains("--effort"));
    });
}

#[test]
fn test_skill_generate_documents_list_truncation_and_sort_filter() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        // The reference must teach the truncation footgun + escape hatches so
        // agents narrow/enumerate before asserting absence or completeness.
        let content = temper_cli::commands::skill::generate(&config).unwrap();
        assert!(
            content.contains("## Listing: truncation, sort, and filters"),
            "reference.md should document the listing truncation/sort/filter mechanics"
        );
        assert!(
            content.contains("truncated"),
            "should explain the `truncated` signal"
        );
        assert!(
            content.contains("Never conclude a resource is absent"),
            "should instruct agents not to assert absence from a default list"
        );
        // The new flags should be named in the reference.
        for flag in ["--all", "--sort", "--title-contains", "--offset"] {
            assert!(content.contains(flag), "reference should mention {flag}");
        }
    });
}

#[test]
fn test_skill_generate_documents_body_source_precedence() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        // The reference must pin the body-source precedence and the loop-stdin
        // footgun so agents don't silently clobber a body via inherited stdin.
        let content = temper_cli::commands::skill::generate(&config).unwrap();
        assert!(
            content.contains("## Body Source"),
            "reference.md should document the body-source precedence"
        );
        assert!(
            content.contains("while read"),
            "should call out the redirected-loop stdin footgun"
        );
        assert!(
            content.contains("implicit stdin is a body rewrite"),
            "should state that implicit non-TTY stdin is a full-body rewrite"
        );
    });
}

#[test]
fn test_skill_generate_documents_block_grain_ingest() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        // The reference must route agents to the block-grain / attribution
        // surface so they don't default to whole-body writes for citation-graded
        // or large documents.
        let content = temper_cli::commands::skill::generate(&config).unwrap();
        assert!(
            content.contains("## Block-Grain Ingest & Attribution"),
            "reference.md should document the block-grain ingest/attribution surface"
        );
        for tool in [
            "ingest_begin",
            "ingest_append",
            "ingest_blocks",
            "ingest_finalize",
        ] {
            assert!(
                content.contains(tool),
                "reference should name the {tool} lifecycle tool"
            );
        }
        assert!(
            content.contains("annotate"),
            "should document the annotate-only provenance backfill"
        );
        assert!(
            content.contains("--provenance"),
            "should point at `resource show --provenance` for reading provenance"
        );
    });
}

#[test]
fn test_skill_md_routes_to_block_grain_and_guards_stdin() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        let skill_dir = dir.path().join("skill-output");
        temper_cli::commands::skill::install(&config, &skill_dir).unwrap();
        let skill_md = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        // Router must point at the block-grain reference section...
        assert!(
            skill_md.contains("Block-Grain Ingest & Attribution"),
            "SKILL.md router should route block-level writes to the reference section"
        );
        // ...and guard the resource-update stdin footgun.
        assert!(
            skill_md.contains("while read"),
            "SKILL.md should warn against running update inside a redirected loop"
        );
    });
}

#[test]
fn test_skill_generate_includes_skill_only_commands() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        // generate() now returns reference.md which includes skill-only commands in footer
        let content = temper_cli::commands::skill::generate(&config).unwrap();
        assert!(
            content.contains("## Skill-Only Commands"),
            "should contain skill-only commands section"
        );
        assert!(
            content.contains("task start"),
            "should contain task start skill command"
        );
        assert!(
            content.contains("task resume"),
            "should contain task resume skill command"
        );
        assert!(
            content.contains("session start"),
            "should contain session start skill command"
        );
    });
}

#[test]
fn test_skill_generate_uses_decorated_context_ref_form() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        let content = temper_cli::commands::skill::generate(&config).unwrap();
        // reference.md should document the decorated ref form, not bare name
        assert!(
            content.contains("@me/"),
            "reference.md should show @me/<ctx> ref form examples"
        );
        // Old bare-name error copy must not appear
        assert!(
            !content.contains("use --context <name>"),
            "old bare-name error copy must not appear in reference.md"
        );
    });
}

#[test]
fn test_skill_md_contexts_section_addresses_by_ref() {
    let dir = TempDir::new().unwrap();
    let (config, env) = test_config_with_global(&dir);

    temp_env::with_vars(env, || {
        let skill_dir = dir.path().join("skill-output");
        temper_cli::commands::skill::install(&config, &skill_dir).unwrap();
        let skill_md = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        // SKILL.md should explain that contexts are addressed by @me/<slug>
        assert!(
            skill_md.contains("@me/"),
            "SKILL.md should reference @me/<slug> context addressing"
        );
        // Examples in SKILL.md should show the decorated form
        assert!(
            skill_md.contains("@me/<ctx>"),
            "SKILL.md workflow examples should use @me/<ctx> form"
        );
    });
}
