use clap::Parser;
use temper_cli::cli::{
    AuthAction, Cli, CogmapCmd, Commands, ConfigAction, ContextAction, InvocationCmd,
    ResourceAction, SkillAction, TeamAction,
};
use temper_cli::commands;
use temper_cli::format::OutputFormat;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();

    // Resolve global output settings once, before dispatch. Color is applied
    // before `run` so all output — including the error path below — obeys it.
    let global_cfg = temper_core::types::config::load_config().unwrap_or_default();
    temper_cli::color::apply_color_choice(cli.color.as_deref(), global_cfg.cli.color.as_deref());
    let output_format =
        OutputFormat::resolve_with(cli.format.as_deref(), global_cfg.cli.format.as_deref());

    if let Err(e) = run(cli, output_format) {
        match &e {
            temper_cli::error::TemperError::SystemAccessRequired(details) => {
                render_system_access_required(
                    details.email.as_deref(),
                    details.join_request_status.as_deref(),
                    details.request_url.as_deref(),
                    details.cli_command.as_deref(),
                );
            }
            _ => {
                temper_cli::output::error(format!("temper: {e}"));
            }
        }
        std::process::exit(1);
    }
}

fn render_system_access_required(
    email: Option<&str>,
    join_request_status: Option<&str>,
    request_url: Option<&str>,
    cli_command: Option<&str>,
) {
    use temper_cli::output;

    let identity = email.unwrap_or("your account");
    output::error(format!(
        "You're signed in as {identity}, but this temper instance\n  requires approved access."
    ));
    output::blank();

    match join_request_status {
        Some("pending") => {
            output::plain("  Your access request is pending review.");
            output::hint("  Run `temper team status` to check for updates.");
        }
        Some("rejected") => {
            output::plain("  Your previous request was not approved. You can submit a new one:");
            if let Some(cmd) = cli_command {
                output::hint(format!("    {cmd}"));
            }
        }
        Some("withdrawn") => {
            output::plain("  You withdrew your previous request. Submit a new one:");
            if let Some(cmd) = cli_command {
                output::hint(format!("    {cmd}"));
            }
        }
        _ => {
            output::plain("  To request access, run:");
            if let Some(cmd) = cli_command {
                output::hint(format!("    {cmd}"));
            }
            if let Some(url) = request_url {
                output::blank();
                output::plain(format!("  Or visit: {url}"));
            }
        }
    }
}

fn run(cli: Cli, output_format: OutputFormat) -> temper_cli::error::Result<()> {
    match cli.command {
        Commands::Init {
            path,
            no_interactive,
            instance_url,
            auth_domain,
            auth_client_id,
            auth_audience,
            idp,
            auth_server_id,
        } => {
            let vault_path = path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
            let self_host = temper_cli::commands::init::self_host_from_flags(
                instance_url,
                auth_domain,
                auth_client_id,
                auth_audience,
                Some(idp),
                auth_server_id,
            )?;
            temper_cli::commands::init::run(
                &vault_path,
                no_interactive,
                true,
                output_format,
                self_host,
            )
        }
        Commands::Check { quiet } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::check::run(&config, quiet, output_format)
        }
        Commands::Status { verbose } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::status::run(&config, verbose, output_format)
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
                    task,
                    show_template,
                    stdin: _,
                    body,
                    from,
                    act,
                } => {
                    if show_template {
                        let doc_type = temper_workflow::frontmatter::DocType::from_str(&r#type)?;
                        let content = temper_cli::vault::get_template(doc_type)?;
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
                        temper_cli::commands::resource::CreateResourceArgs {
                            doc_type: &r#type,
                            title: &title,
                            context: context.as_deref(),
                            goal: goal.as_deref(),
                            mode: mode.as_deref(),
                            effort: effort.as_deref(),
                            slug: slug.as_deref(),
                            task: task.as_deref(),
                            body_flag: body,
                            from,
                            format: output_format,
                            act: act.into_act_input()?,
                        },
                    )
                }
                ResourceAction::List {
                    r#type,
                    context,
                    limit,
                    stage,
                    goal,
                    status,
                    meta_only,
                    fields,
                } => temper_cli::commands::resource::list(
                    &config,
                    temper_cli::commands::resource::ListParams {
                        doc_type: &r#type,
                        context: context.as_deref(),
                        limit,
                        stage: stage.as_deref(),
                        goal: goal.as_deref(),
                        status: status.as_deref(),
                        format: output_format,
                        meta_only,
                        fields: &fields,
                    },
                ),
                ResourceAction::Show {
                    r#ref,
                    edges,
                    meta_only,
                    fields,
                } => temper_cli::commands::resource::show(
                    &config,
                    temper_cli::commands::resource::ShowParams {
                        r#ref: &r#ref,
                        format: output_format,
                        edges,
                        meta_only,
                        fields: &fields,
                    },
                ),
                ResourceAction::Update {
                    r#ref,
                    type_to,
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
                    body,
                } => {
                    let params = temper_cli::commands::resource::UpdateParams {
                        r#ref: &r#ref,
                        type_to: type_to.as_deref(),
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
                        body,
                        format: output_format,
                    };
                    temper_cli::commands::resource::update(&config, &params)
                }
                ResourceAction::Delete { r#ref, force } => {
                    temper_cli::commands::resource::delete(&config, &r#ref, force, output_format)
                }
            }
        }
        Commands::Context { action } => match action {
            ContextAction::Add { name } => temper_cli::commands::context_cmd::add(&name),
            ContextAction::Remove { name } => temper_cli::commands::context_cmd::remove(&name),
            ContextAction::Create { name, owner } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::context_cmd::create_remote(
                            client,
                            &name,
                            owner.as_deref(),
                            output_format,
                        )
                        .await
                    })
                })
            }
            ContextAction::List => {
                let config = temper_cli::config::load(cli.vault.as_deref())?;
                temper_cli::commands::context_cmd::list(&config, output_format)
            }
        },
        Commands::Warmup { context } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let context = context.as_deref();
            temper_cli::commands::warmup::run(&config, context, output_format)
        }
        Commands::Team { action } => match action {
            TeamAction::Join { team: _, message } => {
                temper_cli::commands::team::join(message.as_deref())
            }
            TeamAction::Status { team: _ } => temper_cli::commands::team::status(),
            TeamAction::Leave { team: _ } => temper_cli::commands::team::leave(),
            TeamAction::Create {
                slug,
                name,
                parent,
                auto_join_role,
            } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::create_remote(
                        client,
                        &slug,
                        name.as_deref(),
                        parent.as_deref(),
                        auto_join_role.as_deref(),
                        output_format,
                    )
                    .await
                })
            }),
            TeamAction::AddMember {
                team,
                profile,
                role,
            } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::add_member_remote(
                        client,
                        &team,
                        &profile,
                        &role,
                        output_format,
                    )
                    .await
                })
            }),
            TeamAction::List => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::list_remote(client, output_format).await
                })
            }),
        },
        Commands::Auth { action } => match action {
            AuthAction::Login => temper_cli::commands::auth::login(output_format),
            AuthAction::Token { provider } => {
                temper_cli::commands::auth::token(&provider, output_format)
            }
            AuthAction::Logout => temper_cli::commands::auth::logout(output_format),
            AuthAction::Status => temper_cli::commands::auth::status(output_format),
            AuthAction::ExportToken => temper_cli::commands::auth::export_token(),
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
                    let report = temper_cli::commands::skill::install(&config, &skill_dir)?;
                    if report.is_no_op() {
                        temper_cli::output::success(format!(
                            "Skill already up to date ({} files): {}",
                            report.total,
                            skill_dir.display()
                        ));
                    } else {
                        temper_cli::output::success(format!(
                            "Skill installed: {} ({} of {} files updated)",
                            skill_dir.display(),
                            report.changed.len(),
                            report.total
                        ));
                        for path in &report.changed {
                            temper_cli::output::item(path);
                        }
                    }
                    Ok(())
                }
                SkillAction::Check => temper_cli::commands::skill::check(&config),
            }
        }
        Commands::Pull { context } => commands::pull::run(&context),
        Commands::Config { action } => match action {
            ConfigAction::Edit => commands::config::edit(),
        },
        Commands::Search {
            query,
            context,
            doc_type,
            limit,
            text_only,
            seed_ids,
            edge_types,
            depth,
            no_graph,
        } => {
            use temper_cli::actions::search as search_actions;
            // Resolve the query embedding at the call site, then bundle every
            // CLI-derived search field into `CliSearchArgs` for `run`.
            let embedding = if text_only {
                None
            } else {
                Some(search_actions::embed_query(&query)?)
            };
            commands::search_cmd::run(
                search_actions::CliSearchArgs {
                    query: &query,
                    embedding,
                    context: context.as_deref(),
                    doc_type: doc_type.as_deref(),
                    limit,
                    seed_ids,
                    edge_types,
                    depth,
                    no_graph,
                },
                output_format,
            )
        }
        Commands::Edge { action } => temper_cli::commands::edge::run(action),
        Commands::Cogmap { cmd } => match cmd {
            CogmapCmd::Reconcile { r#ref, manifest } => {
                commands::cogmap::reconcile(&r#ref, &manifest, output_format)
            }
            CogmapCmd::Shape { cogmap, lens } => {
                commands::cogmap::shape(&cogmap, lens.as_deref(), output_format)
            }
            CogmapCmd::RegionMetrics { cogmap, lens } => {
                commands::cogmap::region_metrics(&cogmap, lens.as_deref(), output_format)
            }
            CogmapCmd::Analytics { cogmap } => commands::cogmap::analytics(&cogmap, output_format),
        },
        Commands::Invocation { cmd } => match cmd {
            InvocationCmd::Open {
                cogmap,
                parent,
                trigger_kind,
            } => {
                commands::invocation::open(&cogmap, parent.as_deref(), &trigger_kind, output_format)
            }
            InvocationCmd::Close {
                invocation,
                disposition,
                outcome,
            } => commands::invocation::close(
                &invocation,
                disposition,
                outcome.as_deref(),
                output_format,
            ),
            InvocationCmd::Show { invocation } => {
                commands::invocation::show(&invocation, output_format)
            }
            InvocationCmd::List { cogmap, status } => {
                commands::invocation::list(cogmap.as_deref(), status.as_deref(), output_format)
            }
        },
    }
}
