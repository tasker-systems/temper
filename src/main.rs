mod cli;

use clap::Parser;
use cli::{
    Cli, Commands, MilestoneAction, NoteAction, ProjectAction, ResearchAction, SessionAction,
    SkillAction, TicketAction,
};

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
            project,
            limit,
            format,
        } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let cwd = std::env::current_dir().unwrap_or_default();
            let resolved = temper_cli::project::resolve_from_cwd(&cwd, &config.projects);
            let project = project
                .as_deref()
                .or_else(|| resolved.map(|r| r.name.as_str()));
            temper_cli::commands::events::run(&config, project, limit, &format)
        }
        Commands::Index {
            force,
            paths,
            sources,
        } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::index::run(&config, force, paths.as_deref(), sources.as_deref())
        }
        Commands::Search {
            query,
            format,
            note_type,
            project,
            limit,
        } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::search::run(
                &config,
                &query,
                &format,
                note_type.as_deref(),
                project.as_deref(),
                limit,
            )
        }
        Commands::Context {
            topic,
            format,
            depth,
            limit,
        } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::context::run(&config, &topic, &format, depth, limit)
        }
        Commands::Note { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                NoteAction::Create {
                    note_type,
                    title,
                    project,
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
                        project.as_deref(),
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
                    project,
                    stdin: _,
                    show_template,
                    ticket,
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
                        project.as_deref(),
                        stdin_content.as_deref(),
                        ticket.as_deref(),
                        state.as_deref(),
                        &format,
                    )
                }
                SessionAction::List { project, format } => {
                    temper_cli::commands::session::list(&config, project.as_deref(), &format)
                }
            }
        }
        Commands::Ticket { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let cwd = std::env::current_dir().unwrap_or_default();
            let resolved = temper_cli::project::resolve_from_cwd(&cwd, &config.projects);
            match action {
                TicketAction::Create {
                    title,
                    project,
                    milestone,
                    stdin: _,
                    show_template,
                } => {
                    if show_template {
                        let content = temper_cli::vault::get_template("ticket")?;
                        print!("{content}");
                        return Ok(());
                    }
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()))
                        .ok_or_else(|| {
                            temper_cli::error::TemperError::Project(
                                "no project specified and could not infer from CWD".into(),
                            )
                        })?;
                    let title = title.expect("title required when not using --show-template");
                    temper_cli::commands::ticket::create(
                        &config,
                        project,
                        &title,
                        milestone.as_deref(),
                        None,
                    )?;
                    Ok(())
                }
                TicketAction::Move {
                    slug,
                    stage,
                    milestone,
                    project,
                } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()));
                    temper_cli::commands::ticket::move_ticket(
                        &config,
                        &slug,
                        stage.as_deref(),
                        milestone.as_deref(),
                        project,
                    )
                }
                TicketAction::Done {
                    slug,
                    branch,
                    pr,
                    project,
                } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()));
                    temper_cli::commands::ticket::done(
                        &config,
                        &slug,
                        branch.as_deref(),
                        pr.as_deref(),
                        project,
                    )
                }
                TicketAction::List {
                    project,
                    milestone,
                    format,
                } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()));
                    temper_cli::commands::ticket::list(
                        &config,
                        project,
                        milestone.as_deref(),
                        &format,
                    )
                }
                TicketAction::Show {
                    slug,
                    project,
                    format,
                } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()));
                    temper_cli::commands::ticket::show(&config, &slug, project, &format)
                }
                TicketAction::Board {
                    project,
                    milestone,
                    format,
                } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()))
                        .ok_or_else(|| {
                            temper_cli::error::TemperError::Project(
                                "no project specified and could not infer from CWD".into(),
                            )
                        })?;
                    temper_cli::commands::ticket::board(
                        &config,
                        project,
                        milestone.as_deref(),
                        &format,
                    )
                }
            }
        }
        Commands::Project { action } => match action {
            ProjectAction::Add { name, path, repo } => {
                let vault_root = temper_cli::config::resolve_vault(cli.vault.as_deref())?;
                temper_cli::commands::project::add(&vault_root, &name, &path, repo.as_deref())
            }
            ProjectAction::Remove { name } => {
                let vault_root = temper_cli::config::resolve_vault(cli.vault.as_deref())?;
                temper_cli::commands::project::remove(&vault_root, &name)
            }
            ProjectAction::List => {
                let config = temper_cli::config::load(cli.vault.as_deref())?;
                temper_cli::commands::project::list(&config)
            }
        },
        Commands::Milestone { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let cwd = std::env::current_dir().unwrap_or_default();
            let resolved = temper_cli::project::resolve_from_cwd(&cwd, &config.projects);
            match action {
                MilestoneAction::Create {
                    title,
                    project,
                    slug,
                    format,
                } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()))
                        .ok_or_else(|| {
                            temper_cli::error::TemperError::Project(
                                "no project specified and could not infer from CWD".into(),
                            )
                        })?;
                    temper_cli::commands::milestone::create(
                        &config,
                        project,
                        &title,
                        slug.as_deref(),
                        &format,
                    )?;
                    Ok(())
                }
                MilestoneAction::List { project, format } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()))
                        .ok_or_else(|| {
                            temper_cli::error::TemperError::Project(
                                "no project specified and could not infer from CWD".into(),
                            )
                        })?;
                    temper_cli::commands::milestone::list(&config, project, &format)
                }
                MilestoneAction::Update {
                    slug,
                    status,
                    project,
                } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()));
                    temper_cli::commands::milestone::update(&config, &slug, &status, project)
                }
            }
        }
        Commands::Normalize {
            project,
            dry_run,
            fix_slugs,
        } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::normalize::run(&config, project.as_deref(), dry_run, fix_slugs)?;
            Ok(())
        }
        Commands::Warmup { project, format } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let cwd = std::env::current_dir().unwrap_or_default();
            let resolved = temper_cli::project::resolve_from_cwd(&cwd, &config.projects);
            let project = project
                .as_deref()
                .or_else(|| resolved.map(|r| r.name.as_str()));
            temper_cli::commands::warmup::run(&config, project, &format)
        }
        Commands::Research { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                ResearchAction::Save {
                    title,
                    project,
                    format,
                    show_template,
                    stdin: _,
                } => {
                    if show_template {
                        let content = temper_cli::vault::get_template("research")?;
                        print!("{content}");
                        return Ok(());
                    }
                    let cwd = std::env::current_dir().unwrap_or_default();
                    let resolved = temper_cli::project::resolve_from_cwd(&cwd, &config.projects);
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()));
                    let title = title.expect("title required when not using --show-template");
                    let stdin_content = temper_cli::vault::read_stdin_if_piped();
                    temper_cli::commands::research::save(
                        &config,
                        &title,
                        project,
                        stdin_content.as_deref(),
                        &format,
                    )
                }
            }
        }
        Commands::Skill { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                SkillAction::Generate => {
                    let content = temper_cli::commands::skill::generate(&config)?;
                    print!("{}", content);
                    Ok(())
                }
                SkillAction::Install {
                    global: _,
                    project,
                    path,
                } => {
                    let output_path = if let Some(p) = path {
                        std::path::PathBuf::from(p)
                    } else if let Some(proj) = project {
                        temper_cli::config::expand_tilde(&format!(
                            "{}/.claude/commands/temper.md",
                            proj
                        ))
                    } else {
                        config.skill_output.clone()
                    };
                    temper_cli::commands::skill::install(&config, &output_path)?;
                    temper_cli::output::success(format!(
                        "Skill installed: {}",
                        output_path.display()
                    ));
                    Ok(())
                }
                SkillAction::Check => temper_cli::commands::skill::check(&config),
            }
        }
    }
}
