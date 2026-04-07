use clap::Parser;
use temper_cli::cli::{
    AuthAction, Cli, Commands, ContextAction, DoctorAction, ResourceAction, SkillAction, SyncAction,
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
        Commands::Resource { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                ResourceAction::Create {
                    r#type,
                    title,
                    context,
                    goal,
                    mode,
                    effort,
                    slug,
                    show_template,
                    stdin: _,
                    format,
                } => {
                    if show_template {
                        let content = temper_cli::vault::get_template(&r#type)?;
                        print!("{content}");
                        return Ok(());
                    }
                    let title = title.ok_or_else(|| {
                        temper_cli::error::TemperError::Project(
                            "--title is required for resource create".into(),
                        )
                    })?;
                    temper_cli::commands::resource::create(
                        &config,
                        &r#type,
                        &title,
                        context.as_deref(),
                        goal.as_deref(),
                        mode.as_deref(),
                        effort.as_deref(),
                        slug.as_deref(),
                        &format,
                    )
                }
                ResourceAction::List {
                    r#type,
                    context,
                    limit,
                    stage,
                    goal,
                    status,
                    format,
                } => temper_cli::commands::resource::list(
                    &config,
                    &r#type,
                    context.as_deref(),
                    limit,
                    stage.as_deref(),
                    goal.as_deref(),
                    status.as_deref(),
                    &format,
                ),
                ResourceAction::Show {
                    slug,
                    r#type,
                    context,
                    format,
                } => temper_cli::commands::resource::show(
                    &config,
                    &r#type,
                    &slug,
                    context.as_deref(),
                    &format,
                ),
                ResourceAction::Update {
                    slug,
                    r#type,
                    type_from,
                    type_to,
                    context,
                    context_to,
                    title,
                    tags,
                    aliases,
                    relates_to,
                    references,
                    depends_on,
                    extends,
                    preceded_by,
                    derived_from,
                    stage,
                    mode,
                    effort,
                    goal,
                    seq,
                    branch,
                    pr,
                    status,
                } => {
                    let params = temper_cli::commands::resource::UpdateParams {
                        slug: &slug,
                        doc_type: r#type.as_deref(),
                        type_from: type_from.as_deref(),
                        type_to: type_to.as_deref(),
                        context: context.as_deref(),
                        context_to: context_to.as_deref(),
                        title: title.as_deref(),
                        tags: &tags,
                        aliases: &aliases,
                        relates_to: &relates_to,
                        references: &references,
                        depends_on: &depends_on,
                        extends: &extends,
                        preceded_by: &preceded_by,
                        derived_from: &derived_from,
                        stage: stage.as_deref(),
                        mode: mode.as_deref(),
                        effort: effort.as_deref(),
                        goal: goal.as_deref(),
                        seq,
                        branch: branch.as_deref(),
                        pr: pr.as_deref(),
                        status: status.as_deref(),
                    };
                    temper_cli::commands::resource::update(&config, &params)
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
        Commands::Doctor {
            action,
            context,
            format,
        } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                Some(DoctorAction::Fix { dry_run }) => {
                    temper_cli::commands::doctor::run_fix(&config, context.as_deref(), dry_run)?;
                }
                None => {
                    temper_cli::commands::doctor::run(&config, context.as_deref(), &format)?;
                }
            }
            Ok(())
        }
        Commands::Warmup { context, format } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let context = context.as_deref();
            temper_cli::commands::warmup::run(&config, context, &format)
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
            text_only,
        } => commands::search_cmd::run(
            &query,
            context.as_deref(),
            doc_type.as_deref(),
            limit,
            &format,
            text_only,
        ),
    }
}
