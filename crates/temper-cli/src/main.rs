mod cli;

use clap::Parser;
use cli::{
    AuthAction, Cli, Commands, ContextAction, GoalAction, NoteAction, ResearchAction,
    SessionAction, SkillAction, SyncAction, TaskAction,
};
use temper_cli::commands;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        temper_cli::output::error(format!("temper: {e}"));
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> temper_cli::error::Result<()> {
    match cli.command {
        Commands::Init {
            path,
            no_interactive,
        } => {
            let vault_path = path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
            temper_cli::commands::init::run(&vault_path, no_interactive, true)
        }
        Commands::Check { quiet } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::check::run(&config, quiet)
        }
        Commands::Status { verbose } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::status::run(&config, verbose)
        }
        Commands::Events {
            context,
            limit,
            format,
        } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let context = context.as_deref();
            temper_cli::commands::events::run(&config, context, limit, &format)
        }
        Commands::Note { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                NoteAction::Create {
                    note_type,
                    title,
                    context,
                    stdin: _,
                    show_template,
                    format,
                } => {
                    if show_template {
                        let nt = note_type.as_deref().unwrap_or("session");
                        let content = temper_cli::vault::get_template(nt)?;
                        print!("{content}");
                        return Ok(());
                    }
                    let note_type =
                        note_type.expect("note_type required when not using --show-template");
                    let title = title.expect("title required when not using --show-template");
                    temper_cli::commands::note::create(
                        &config,
                        &note_type,
                        &title,
                        context.as_deref(),
                        &format,
                    )
                }
            }
        }
        Commands::Session { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                SessionAction::Save {
                    title,
                    context,
                    stdin: _,
                    show_template,
                    task,
                    state,
                    format,
                } => {
                    if show_template {
                        let content = temper_cli::vault::get_template("session")?;
                        print!("{content}");
                        return Ok(());
                    }
                    let stdin_content = temper_cli::vault::read_stdin_if_piped();
                    temper_cli::commands::session::save(
                        &config,
                        title.as_deref(),
                        context.as_deref(),
                        stdin_content.as_deref(),
                        task.as_deref(),
                        state.as_deref(),
                        &format,
                    )
                }
                SessionAction::List { context, format } => {
                    temper_cli::commands::session::list(&config, context.as_deref(), &format)
                }
            }
        }
        Commands::Task { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                TaskAction::Create {
                    title,
                    context,
                    goal,
                    mode,
                    effort,
                    stdin: _,
                    show_template,
                } => {
                    if show_template {
                        let content = temper_cli::vault::get_template("task")?;
                        print!("{content}");
                        return Ok(());
                    }
                    let context = context.as_deref().ok_or_else(|| {
                        temper_cli::error::TemperError::Project(
                            "no context specified — use --context <name>".into(),
                        )
                    })?;
                    let context =
                        temper_cli::commands::resolve_context_with_fallback(&config, context);
                    let title = title.expect("title required when not using --show-template");
                    temper_cli::commands::task::create(
                        &config,
                        &context,
                        &title,
                        goal.as_deref(),
                        mode.as_deref(),
                        effort.as_deref(),
                    )?;
                    Ok(())
                }
                TaskAction::Move {
                    slug,
                    stage,
                    goal,
                    context,
                    mode,
                    effort,
                } => {
                    let context = context.as_deref();
                    temper_cli::commands::task::move_task(
                        &config,
                        &slug,
                        stage.as_deref(),
                        goal.as_deref(),
                        context,
                        mode.as_deref(),
                        effort.as_deref(),
                    )
                }
                TaskAction::Done {
                    slug,
                    branch,
                    pr,
                    context,
                } => {
                    let context = context.as_deref();
                    temper_cli::commands::task::done(
                        &config,
                        &slug,
                        branch.as_deref(),
                        pr.as_deref(),
                        context,
                    )
                }
                TaskAction::List {
                    context,
                    goal,
                    format,
                } => {
                    let context = context.as_deref();
                    temper_cli::commands::task::list(&config, context, goal.as_deref(), &format)
                }
                TaskAction::Show {
                    slug,
                    context,
                    format,
                } => {
                    let context = context.as_deref();
                    temper_cli::commands::task::show(&config, &slug, context, &format)
                }
            }
        }
        Commands::Context { action } => match action {
            ContextAction::Add { name } => temper_cli::commands::context_cmd::add(&name),
            ContextAction::Remove { name } => temper_cli::commands::context_cmd::remove(&name),
            ContextAction::Create { name } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::context_cmd::create_remote(client, &name).await
                })
            }),
            ContextAction::List => {
                let config = temper_cli::config::load(cli.vault.as_deref())?;
                temper_cli::commands::context_cmd::list(&config)
            }
        },
        Commands::Goal { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                GoalAction::Create {
                    title,
                    context,
                    slug,
                    format,
                } => {
                    let context = context.as_deref().ok_or_else(|| {
                        temper_cli::error::TemperError::Project(
                            "no context specified — use --context <name>".into(),
                        )
                    })?;
                    let context =
                        temper_cli::commands::resolve_context_with_fallback(&config, context);
                    temper_cli::commands::goal::create(
                        &config,
                        &context,
                        &title,
                        slug.as_deref(),
                        &format,
                    )?;
                    Ok(())
                }
                GoalAction::List { context, format } => {
                    let context = context.as_deref().ok_or_else(|| {
                        temper_cli::error::TemperError::Project(
                            "no context specified — use --context <name>".into(),
                        )
                    })?;
                    let context =
                        temper_cli::commands::resolve_context_with_fallback(&config, context);
                    temper_cli::commands::goal::list(&config, &context, &format)
                }
                GoalAction::Update {
                    slug,
                    status,
                    context,
                } => {
                    let context = context.as_deref();
                    temper_cli::commands::goal::update(&config, &slug, &status, context)
                }
            }
        }
        Commands::Normalize {
            context,
            dry_run,
            fix_slugs,
        } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::normalize::run(&config, context.as_deref(), dry_run, fix_slugs)?;
            Ok(())
        }
        Commands::Warmup { context, format } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let context = context.as_deref();
            temper_cli::commands::warmup::run(&config, context, &format)
        }
        Commands::Research { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                ResearchAction::Save {
                    title,
                    context,
                    format,
                    show_template,
                    stdin: _,
                } => {
                    if show_template {
                        let content = temper_cli::vault::get_template("research")?;
                        print!("{content}");
                        return Ok(());
                    }
                    let context = context.as_deref();
                    let title = title.expect("title required when not using --show-template");
                    let stdin_content = temper_cli::vault::read_stdin_if_piped();
                    temper_cli::commands::research::save(
                        &config,
                        &title,
                        context,
                        stdin_content.as_deref(),
                        &format,
                    )
                }
            }
        }
        Commands::Auth { action } => match action {
            AuthAction::Login => temper_cli::commands::auth::login(),
            AuthAction::Token { jwt, provider } => {
                temper_cli::commands::auth::token(&jwt, &provider)
            }
            AuthAction::Logout => temper_cli::commands::auth::logout(),
            AuthAction::Status => temper_cli::commands::auth::status(),
        },
        Commands::Skill { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                SkillAction::Generate => {
                    let content = temper_cli::commands::skill::generate(&config)?;
                    print!("{}", content);
                    Ok(())
                }
                SkillAction::Install { path } => {
                    let skill_dir = if let Some(p) = path {
                        std::path::PathBuf::from(p)
                    } else {
                        config.skill_output.clone()
                    };
                    temper_cli::commands::skill::install(&config, &skill_dir)?;
                    temper_cli::output::success(format!(
                        "Skill installed: {}",
                        skill_dir.display()
                    ));
                    Ok(())
                }
                SkillAction::Check => temper_cli::commands::skill::check(&config),
            }
        }
        Commands::Add {
            path,
            dir,
            context,
            doc_type,
            format,
            force,
            dry_run,
            ignore,
        } => commands::add::run(
            &path,
            dir,
            context.as_deref(),
            &doc_type,
            &format,
            force,
            dry_run,
            ignore.as_deref(),
        ),
        Commands::Pull { resource_id } => commands::pull::run(&resource_id),
        Commands::Remove { resource_id, force } => commands::remove::run(&resource_id, force),
        Commands::Sync { action } => match action {
            SyncAction::Run { context, format } => commands::sync_cmd::run(&context, &format),
            SyncAction::Status { context, format } => commands::sync_cmd::status(&context, &format),
            SyncAction::Refresh { format } => commands::sync_cmd::refresh(&format),
            SyncAction::Reset { format } => commands::sync_cmd::reset(&format),
        },
        Commands::Search {
            query,
            context,
            doc_type,
            limit,
            format,
        } => commands::search_cmd::run(
            &query,
            context.as_deref(),
            doc_type.as_deref(),
            limit,
            &format,
        ),
    }
}
