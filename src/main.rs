mod cli;

use clap::Parser;
use cli::{
    Cli, Commands, MilestoneAction, NoteAction, ProjectAction, SessionAction, SkillAction,
    TicketAction,
};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        eprintln!("temper: {e}");
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
                    stdin,
                } => temper_cli::commands::note::create(
                    &config,
                    &note_type,
                    &title,
                    project.as_deref(),
                    stdin,
                ),
            }
        }
        Commands::Session { action } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            match action {
                SessionAction::Save {
                    title,
                    project,
                    stdin,
                } => {
                    let stdin_content = if stdin {
                        use std::io::Read;
                        let mut buf = String::new();
                        std::io::stdin().read_to_string(&mut buf).ok();
                        Some(buf)
                    } else {
                        None
                    };
                    temper_cli::commands::session::save(
                        &config,
                        title.as_deref(),
                        project.as_deref(),
                        stdin_content.as_deref(),
                    )
                }
                SessionAction::List { project } => {
                    temper_cli::commands::session::list(&config, project.as_deref())
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
                    stdin,
                } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()))
                        .ok_or_else(|| {
                            temper_cli::error::TemperError::Project(
                                "no project specified and could not infer from CWD".into(),
                            )
                        })?;
                    temper_cli::commands::ticket::create(
                        &config,
                        project,
                        &title,
                        milestone.as_deref(),
                        stdin,
                    )?;
                    Ok(())
                }
                TicketAction::Move {
                    slug,
                    stage,
                    milestone,
                } => temper_cli::commands::ticket::move_ticket(
                    &config,
                    &slug,
                    stage.as_deref(),
                    milestone.as_deref(),
                ),
                TicketAction::Done { slug, branch, pr } => temper_cli::commands::ticket::done(
                    &config,
                    &slug,
                    branch.as_deref(),
                    pr.as_deref(),
                ),
                TicketAction::List { project, milestone } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()));
                    temper_cli::commands::ticket::list(&config, project, milestone.as_deref())
                }
                TicketAction::Show { slug } => temper_cli::commands::ticket::show(&config, &slug),
                TicketAction::Board { project, milestone } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()))
                        .ok_or_else(|| {
                            temper_cli::error::TemperError::Project(
                                "no project specified and could not infer from CWD".into(),
                            )
                        })?;
                    temper_cli::commands::ticket::board(&config, project, milestone.as_deref())
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
                    )?;
                    Ok(())
                }
                MilestoneAction::List { project } => {
                    let project = project
                        .as_deref()
                        .or_else(|| resolved.map(|r| r.name.as_str()))
                        .ok_or_else(|| {
                            temper_cli::error::TemperError::Project(
                                "no project specified and could not infer from CWD".into(),
                            )
                        })?;
                    temper_cli::commands::milestone::list(&config, project)
                }
                MilestoneAction::Update { slug, status } => {
                    temper_cli::commands::milestone::update(&config, &slug, &status)
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
                    println!("Skill installed: {}", output_path.display());
                    Ok(())
                }
                SkillAction::Check => temper_cli::commands::skill::check(&config),
            }
        }
    }
}
