use clap::Parser;
use temper_cli::cli::{
    AdminAction, AdminConnectionAction, AdminMachineAction, AdminRequestsAction, AdminSamlAction,
    AdminSlackAction, AuthAction, Cli, CogmapCmd, Commands, ConfigAction, ContextAction,
    InvocationCmd, ResourceAction, SkillAction, SlackAction, StewardCmd, TeamAction,
};
use temper_cli::commands;
use temper_cli::format::OutputFormat;

fn main() {
    // Logs go to STDERR, never stdout: stdout is reserved for machine-readable
    // output (JSON/TOON) so `temper … | jq` stays clean. Without this, library
    // logs — notably ONNX Runtime's `ort` INFO chatter on embed paths — would
    // interleave with the command's JSON on stdout and break parsing.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();

    // The CLI embeds one document for one user, so it should use the machine.
    //
    // It asks for the PERFORMANCE-core count, not `0` ("let ORT decide") and not the
    // total core count — both are measurably worse. On a 12-core M4 Pro, `0` used only
    // ~6 threads (10.77s/segment) and all 12 cores were *also* slower than the 8
    // performance cores (9.73s vs 9.62s), because an intra-op batch advances only as
    // fast as its slowest thread and the efficiency cores drag every barrier. See
    // `temper_ingest::cpu` for the full table (task 019f57d2).
    //
    // On a platform where we cannot honestly detect performance cores, `performance_cores()`
    // returns `None` and we keep today's behavior (`0`) rather than invent a number.
    //
    // The server deliberately does NOT opt in — concurrent ingests could oversubscribe the
    // box, and that case is still unmeasured (task 019f5892).
    //
    // Must run before the first embed: the count is read when the ORT session is lazily built.
    #[cfg(feature = "embed")]
    {
        temper_ingest::embed::set_intra_op_threads(
            temper_ingest::cpu::performance_cores().unwrap_or(0),
        );
        // An explicitly typed flag outranks the ambient environment.
        if let Some(n) = cli.embed_threads {
            temper_ingest::embed::force_intra_op_threads(n);
        }
    }

    // Resolve global output settings once, before dispatch. Color is applied
    // before `run` so all output — including the error path below — obeys it.
    let global_cfg = temper_core::types::config::load_config().unwrap_or_default();
    temper_cli::color::apply_color_choice(cli.color.as_deref(), global_cfg.cli.color.as_deref());
    let output_format =
        OutputFormat::resolve_with(cli.format.as_deref(), global_cfg.cli.format.as_deref());

    if let Err(e) = run(cli, output_format) {
        match &e {
            temper_cli::error::TemperError::SystemAccessRequired(details) => {
                temper_cli::access_gate::render_system_access_required(
                    details.email.as_deref(),
                    details.refusal.as_ref(),
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
                    cogmap,
                    mode,
                    effort,
                    open_meta,
                    goal,
                    task,
                    show_template,
                    stdin: _,
                    body,
                    from,
                    sources,
                    sources_as_edges,
                    no_source,
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
                            cogmap: cogmap.as_deref(),
                            mode: mode.as_deref(),
                            effort: effort.as_deref(),
                            open_meta: open_meta.as_deref(),
                            goal: goal.as_deref(),
                            task: task.as_deref(),
                            body_flag: body,
                            from,
                            sources,
                            sources_as_edges,
                            no_source,
                            format: output_format,
                            act: act.into_act_input()?,
                        },
                    )
                }
                ResourceAction::List {
                    r#type,
                    context,
                    limit,
                    all,
                    offset,
                    sort,
                    title_contains,
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
                        all,
                        offset,
                        sort: sort.as_deref(),
                        title_contains: title_contains.as_deref(),
                        stage: stage.as_deref(),
                        goal: goal.as_deref(),
                        status: status.as_deref(),
                        format: output_format,
                        meta_only,
                        fields: &fields,
                    },
                ),
                ResourceAction::DescribeOpenMeta => {
                    temper_cli::commands::resource::describe_open_meta(output_format)
                }
                ResourceAction::Show {
                    r#ref,
                    edges,
                    lineage,
                    provenance,
                    meta_only,
                    fields,
                } => temper_cli::commands::resource::show(
                    &config,
                    temper_cli::commands::resource::ShowParams {
                        r#ref: &r#ref,
                        format: output_format,
                        edges,
                        lineage,
                        provenance,
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
                    open_meta,
                    stage,
                    mode,
                    effort,
                    seq,
                    branch,
                    pr,
                    goal,
                    clear_goal,
                    status,
                    body,
                    sources,
                    content_block,
                    act,
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
                        open_meta: open_meta.as_deref(),
                        stage: stage.as_deref(),
                        mode: mode.as_deref(),
                        effort: effort.as_deref(),
                        seq,
                        branch: branch.as_deref(),
                        pr: pr.as_deref(),
                        goal: goal.as_deref(),
                        clear_goal,
                        status: status.as_deref(),
                        body,
                        sources: &sources,
                        content_block,
                        format: output_format,
                        act: act.into_act_input()?,
                    };
                    temper_cli::commands::resource::update(&config, &params)
                }
                ResourceAction::Annotate {
                    r#ref,
                    sources,
                    content_block,
                    act,
                } => temper_cli::commands::resource::annotate(
                    &config,
                    temper_cli::commands::resource::AnnotateParams {
                        r#ref: &r#ref,
                        sources: &sources,
                        content_block,
                        format: output_format,
                        act: act.into_act_input()?,
                    },
                ),
                ResourceAction::Delete { r#ref, force, act } => {
                    temper_cli::commands::resource::delete(
                        &config,
                        &r#ref,
                        force,
                        act.into_act_input()?,
                        output_format,
                    )
                }
                ResourceAction::Reassign { r#ref, to } => {
                    temper_cli::commands::resource::reassign(&r#ref, &to, output_format)
                }
                ResourceAction::Grant {
                    r#ref,
                    to_profile,
                    to_team,
                    read,
                    write,
                    grant,
                } => temper_cli::commands::resource::grant(
                    &r#ref,
                    to_profile,
                    to_team,
                    read,
                    write,
                    grant,
                    output_format,
                ),
                ResourceAction::Revoke {
                    r#ref,
                    from_profile,
                    from_team,
                } => temper_cli::commands::resource::revoke(
                    &r#ref,
                    from_profile,
                    from_team,
                    output_format,
                ),
                ResourceAction::Facet {
                    r#ref,
                    values,
                    weight,
                    act,
                } => temper_cli::commands::facet::run(r#ref, values, weight, act, output_format),
            }
        }
        Commands::Context { action } => match action {
            ContextAction::Subscribe { name } => {
                temper_cli::commands::context_cmd::subscribe(&name)
            }
            ContextAction::Unsubscribe { name } => {
                temper_cli::commands::context_cmd::unsubscribe(&name)
            }
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
            ContextAction::List => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::context_cmd::list(client, output_format).await
                })
            }),
            ContextAction::Share { context, team } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::context_cmd::share_remote(
                            client,
                            &context,
                            &team,
                            output_format,
                        )
                        .await
                    })
                })
            }
            ContextAction::Unshare { context, team } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::context_cmd::unshare_remote(
                            client,
                            &context,
                            &team,
                            output_format,
                        )
                        .await
                    })
                })
            }
            ContextAction::Transfer { context, team } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::context_cmd::transfer_remote(
                            client,
                            &context,
                            &team,
                            output_format,
                        )
                        .await
                    })
                })
            }
            ContextAction::Shape { context, lens } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::context_cmd::shape_remote(
                            client,
                            &context,
                            lens.as_deref(),
                            output_format,
                        )
                        .await
                    })
                })
            }
            ContextAction::RegionMetrics { context, lens } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::context_cmd::region_metrics_remote(
                            client,
                            &context,
                            lens.as_deref(),
                            output_format,
                        )
                        .await
                    })
                })
            }
            ContextAction::Materialize { context, threshold } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::context_cmd::materialize_remote(
                            client,
                            &context,
                            threshold,
                            output_format,
                        )
                        .await
                    })
                })
            }
        },
        Commands::Warmup { context } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let context = context.as_deref();
            temper_cli::commands::warmup::run(&config, context, output_format)
        }
        Commands::Invitations => temper_cli::actions::runtime::with_client(|client| {
            Box::pin(async move {
                temper_cli::commands::invitations::list_mine(client, output_format).await
            })
        }),
        Commands::Team { action } => match action {
            TeamAction::Join { token } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::accept_invitation(client, &token, output_format)
                        .await
                })
            }),
            TeamAction::Invite { team, email, role } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::team::invite_remote(
                            client,
                            &team,
                            &email,
                            &role,
                            output_format,
                        )
                        .await
                    })
                })
            }
            TeamAction::Decline { token } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::decline_invitation(client, &token, output_format)
                        .await
                })
            }),
            TeamAction::Invitations { team } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::team::list_invitations_remote(
                            client,
                            &team,
                            output_format,
                        )
                        .await
                    })
                })
            }
            TeamAction::Show { team } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::show_remote(client, &team, output_format).await
                })
            }),
            TeamAction::Leave { team } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::leave_remote(client, &team, output_format).await
                })
            }),
            TeamAction::RemoveMember { team, profile } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::team::remove_member_remote(
                            client,
                            &team,
                            &profile,
                            output_format,
                        )
                        .await
                    })
                })
            }
            TeamAction::SetRole {
                team,
                profile,
                role,
            } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::set_role_remote(
                        client,
                        &team,
                        &profile,
                        &role,
                        output_format,
                    )
                    .await
                })
            }),
            TeamAction::Update {
                team,
                name,
                description,
            } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::update_remote(
                        client,
                        &team,
                        name.as_deref(),
                        description.as_deref(),
                        output_format,
                    )
                    .await
                })
            }),
            TeamAction::Delete { team } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::delete_remote(client, &team, output_format).await
                })
            }),
            TeamAction::Reassign { team, from, to } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::team::reassign_remote(
                            client,
                            &team,
                            &from,
                            &to,
                            output_format,
                        )
                        .await
                    })
                })
            }
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
        Commands::Admin { action } => match action {
            AdminAction::Settings {
                gating_team_slug,
                instance_name,
                terms_version,
                terms_resource_uri,
            } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let req = temper_core::types::admin::UpdateSettingsRequest {
                        gating_team_slug,
                        instance_name,
                        terms_version,
                        terms_resource_uri,
                    };
                    temper_cli::commands::admin::settings_remote(client, req, output_format).await
                })
            }),
            AdminAction::Promote { profile, team } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin::promote_remote(
                            client,
                            &profile,
                            team.as_deref(),
                            output_format,
                        )
                        .await
                    })
                })
            }
            AdminAction::Demote { profile } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin::demote_remote(client, &profile).await
                    })
                })
            }
            AdminAction::Access { action } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::admin::access_remote(client, &action).await
                })
            }),
            AdminAction::Ledger {
                subject,
                actor,
                limit,
                offset,
            } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::admin::ledger_remote(
                        client,
                        subject.as_deref(),
                        actor.as_deref(),
                        limit,
                        offset,
                        output_format,
                    )
                    .await
                })
            }),
            AdminAction::Requests { action } => match action {
                AdminRequestsAction::List => temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin::requests_list_remote(client, output_format)
                            .await
                    })
                }),
                AdminRequestsAction::Review {
                    id,
                    approve,
                    reject,
                    note,
                } => temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin::requests_review_remote(
                            client,
                            &id,
                            approve,
                            reject,
                            note.as_deref(),
                            output_format,
                        )
                        .await
                    })
                }),
            },
            AdminAction::Saml { action } => match action {
                AdminSamlAction::Provision {
                    no_interactive,
                    instance_url,
                    api_origin,
                    idp_key,
                    idp_cert_file,
                    idp_sso_url,
                    idp_entity_id,
                    nameid_format,
                    email_attr,
                    stable_id_attr,
                    groups_attr,
                    kid,
                    clients,
                    env_out,
                    sql_out,
                    apply,
                } => temper_cli::commands::admin_saml::provision(
                    temper_cli::commands::admin_saml::ProvisionArgs {
                        no_interactive,
                        instance_url,
                        api_origin,
                        idp_key,
                        idp_cert_file,
                        idp_sso_url,
                        idp_entity_id,
                        nameid_format,
                        email_attr,
                        stable_id_attr,
                        groups_attr,
                        kid,
                        clients,
                        env_out,
                        sql_out,
                        apply,
                    },
                ),
                AdminSamlAction::MapGroup {
                    idp_key,
                    group,
                    team,
                    role,
                    from_seen,
                    apply,
                } => {
                    if from_seen {
                        temper_cli::commands::admin_saml::from_seen(&idp_key)
                    } else {
                        // clap `required_unless_present = "from_seen"` guarantees both are
                        // Some in this branch; unwrap defensively rather than panic.
                        let (Some(group), Some(team)) = (group, team) else {
                            return Err(temper_cli::error::TemperError::Config(
                                "map-group requires <group> and <team> unless --from-seen is set"
                                    .into(),
                            ));
                        };
                        temper_cli::actions::runtime::with_client(|client| {
                            Box::pin(async move {
                                temper_cli::commands::admin_saml::map_group(
                                    client, &idp_key, &group, &team, &role, apply,
                                )
                                .await
                            })
                        })
                    }
                }
                AdminSamlAction::Verify { instance_url, db } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_saml::verify(client, &instance_url, db)
                                .await
                        })
                    })
                }
            },
            AdminAction::Reembed {
                resource,
                context,
                all,
                limit,
                dry_run,
            } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::admin::reembed_remote(
                        client,
                        resource,
                        context,
                        all,
                        limit,
                        dry_run,
                        output_format,
                    )
                    .await
                })
            }),
            AdminAction::Machine { action } => match action {
                AdminMachineAction::Provision {
                    client_id,
                    label,
                    owner_team,
                    teams,
                    cogmaps,
                } => temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin_machine::provision_remote(
                            client,
                            &client_id,
                            &label,
                            owner_team.as_deref(),
                            &teams,
                            &cogmaps,
                            output_format,
                        )
                        .await
                    })
                }),
                AdminMachineAction::Rebind {
                    from,
                    client_id,
                    label,
                    no_revoke_old,
                } => temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin_machine::rebind_remote(
                            client,
                            &from,
                            &client_id,
                            &label,
                            no_revoke_old,
                            output_format,
                        )
                        .await
                    })
                }),
                AdminMachineAction::Issue {
                    label,
                    owner_team,
                    teams,
                    cogmaps,
                } => temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin_machine::issue_remote(
                            client,
                            &label,
                            owner_team.as_deref(),
                            &teams,
                            &cogmaps,
                            output_format,
                        )
                        .await
                    })
                }),
                AdminMachineAction::RotateSecret { id, grace_seconds } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_machine::rotate_secret_remote(
                                client,
                                &id,
                                grace_seconds,
                                output_format,
                            )
                            .await
                        })
                    })
                }
                AdminMachineAction::List { include_revoked } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_machine::list_remote(
                                client,
                                include_revoked,
                                output_format,
                            )
                            .await
                        })
                    })
                }
                AdminMachineAction::Show { id } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_machine::show_remote(
                                client,
                                &id,
                                output_format,
                            )
                            .await
                        })
                    })
                }
                AdminMachineAction::Revoke { id } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_machine::revoke_remote(
                                client,
                                &id,
                                output_format,
                            )
                            .await
                        })
                    })
                }
            },
            AdminAction::Slack { action } => match action {
                AdminSlackAction::Disconnect { principal } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_slack::disconnect_remote(
                                client,
                                &principal,
                                output_format,
                            )
                            .await
                        })
                    })
                }
            },
            AdminAction::Connection { action } => match action {
                AdminConnectionAction::Provision {
                    provider,
                    name,
                    owner_team,
                    reach,
                    covers,
                } => temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin_connection::provision_remote(
                            client,
                            &provider,
                            &name,
                            owner_team.as_deref(),
                            reach.as_deref(),
                            covers.as_deref(),
                            output_format,
                        )
                        .await
                    })
                }),
                AdminConnectionAction::List { include_revoked } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_connection::list_remote(
                                client,
                                include_revoked,
                                output_format,
                            )
                            .await
                        })
                    })
                }
                AdminConnectionAction::Show { id } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_connection::show_remote(
                                client,
                                &id,
                                output_format,
                            )
                            .await
                        })
                    })
                }
                AdminConnectionAction::AttachCredential {
                    id,
                    broker,
                    connector,
                    installation,
                } => temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin_connection::attach_credential_remote(
                            client,
                            &id,
                            &broker,
                            &connector,
                            installation.as_deref(),
                            output_format,
                        )
                        .await
                    })
                }),
                AdminConnectionAction::SetWebhooks { id, events } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_connection::set_webhook_events_remote(
                                client,
                                &id,
                                events,
                                output_format,
                            )
                            .await
                        })
                    })
                }
                AdminConnectionAction::SetTools { id, tools } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_connection::set_tool_manifest_remote(
                                client,
                                &id,
                                tools,
                                output_format,
                            )
                            .await
                        })
                    })
                }
                AdminConnectionAction::Revoke { id } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_connection::revoke_remote(
                                client,
                                &id,
                                output_format,
                            )
                            .await
                        })
                    })
                }
                AdminConnectionAction::GrantReach {
                    id,
                    team,
                    affirm_reach,
                } => temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin_connection::grant_reach_remote(
                            client,
                            &id,
                            &team,
                            affirm_reach,
                            output_format,
                        )
                        .await
                    })
                }),
                AdminConnectionAction::RevokeReach { id, team } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin_connection::revoke_reach_remote(
                                client,
                                &id,
                                &team,
                                output_format,
                            )
                            .await
                        })
                    })
                }
            },
        },
        Commands::Auth { action } => match action {
            AuthAction::Login => temper_cli::commands::auth::login(output_format),
            AuthAction::Token { provider } => {
                temper_cli::commands::auth::token(&provider, output_format)
            }
            AuthAction::Logout => temper_cli::commands::auth::logout(output_format),
            AuthAction::Status => temper_cli::commands::auth::status(output_format),
            AuthAction::ExportToken => temper_cli::commands::auth::export_token(),
            AuthAction::RequestAccess { message } => {
                temper_cli::commands::auth::request_access(message.as_deref())
            }
            AuthAction::WithdrawRequest => temper_cli::commands::auth::withdraw_request(),
            AuthAction::RequestReview { message } => {
                temper_cli::commands::auth::request_review(message.as_deref())
            }
        },
        Commands::Slack { action } => match action {
            SlackAction::Disconnect => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::slack::disconnect_remote(client, output_format).await
                })
            }),
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
            cogmap,
            wayfind,
            lens,
            regions,
            doc_type,
            limit,
            text_only,
            seed_ids,
            edge_types,
            depth,
            no_graph,
            seed_only,
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
                    cogmap: cogmap.as_deref(),
                    wayfind,
                    lens: lens.as_deref(),
                    regions,
                    doc_type: doc_type.as_deref(),
                    limit,
                    seed_ids,
                    edge_types,
                    depth,
                    no_graph,
                    seed_only,
                },
                output_format,
            )
        }
        Commands::Edge { action } => temper_cli::commands::edge::run(action, output_format),
        Commands::Cogmap { cmd } => match cmd {
            CogmapCmd::Reconcile {
                r#ref,
                manifest,
                act,
            } => {
                commands::cogmap::reconcile(&r#ref, &manifest, act.into_act_input()?, output_format)
            }
            CogmapCmd::Create { manifest, name, id } => {
                commands::cogmap::create(&manifest, name.as_deref(), id.as_deref(), output_format)
            }
            CogmapCmd::Shape { cogmap, lens } => {
                commands::cogmap::shape(&cogmap, lens.as_deref(), output_format)
            }
            CogmapCmd::RegionMetrics { cogmap, lens } => {
                commands::cogmap::region_metrics(&cogmap, lens.as_deref(), output_format)
            }
            CogmapCmd::Analytics { cogmap } => commands::cogmap::analytics(&cogmap, output_format),
            CogmapCmd::Materialize { cogmap, threshold } => {
                commands::cogmap::materialize(&cogmap, threshold, output_format)
            }
            CogmapCmd::Bind { r#ref, team } => commands::cogmap::bind(&r#ref, &team, output_format),
            CogmapCmd::Unbind { r#ref, team } => {
                commands::cogmap::unbind(&r#ref, &team, output_format)
            }
            CogmapCmd::Grant {
                r#ref,
                to_profile,
                to_team,
                read,
                write,
                grant,
            } => commands::cogmap::grant(
                &r#ref,
                to_profile,
                to_team,
                read,
                write,
                grant,
                output_format,
            ),
            CogmapCmd::Revoke {
                r#ref,
                from_profile,
                from_team,
            } => commands::cogmap::revoke(&r#ref, from_profile, from_team, output_format),
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
        Commands::Steward { cmd } => match cmd {
            StewardCmd::Delta { cogmap, threshold } => {
                commands::steward::delta(&cogmap, threshold, output_format)
            }
            StewardCmd::AdvanceWatermark { cogmap, event } => {
                commands::steward::advance_watermark(&cogmap, &event, output_format)
            }
        },
        Commands::Trail { kind, r#ref } => commands::trail::run(kind, &r#ref, output_format),
        Commands::Version { checksum } => {
            temper_cli::commands::version::run(checksum, output_format)
        }
        Commands::Update {
            check,
            version,
            force,
        } => {
            // `update` raises a CLI-local `CliError` (install failures never
            // belong in the shared `TemperError`), so render + exit here rather
            // than laundering it back through the `TemperError` return path.
            if let Err(e) = temper_cli::commands::update::run(check, version, force, output_format)
            {
                temper_cli::output::error(format!("temper: {e}"));
                std::process::exit(1);
            }
            Ok(())
        }
    }
}
