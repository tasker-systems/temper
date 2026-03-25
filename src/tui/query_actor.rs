use crate::actions;
use crate::config::Config;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum QueryRequest {
    Search {
        query: String,
    },
    Context {
        topic: String,
        depth: usize,
        limit: usize,
    },
    Index {
        force: bool,
    },
    Normalize {
        project: Option<String>,
        dry_run: bool,
        fix_slugs: bool,
    },
    /// Move a ticket to a new stage and/or scope, then reload swimlane data.
    MoveTicket {
        slug: String,
        project: String,
        milestone: String,
        stage: Option<String>,
        scope: Option<String>,
    },
    /// Load tickets for a swimlane view (backlog / in-progress / done columns).
    LoadTickets {
        project: String,
        milestone: String,
    },
}

#[derive(Debug)]
pub enum QueryResult {
    SearchResults(crate::actions::types::SearchResults),
    ContextResults(crate::actions::types::ContextResults),
    IndexComplete(crate::actions::types::IndexStats),
    NormalizeComplete(crate::actions::types::NormalizeSummary),
    Progress {
        message: String,
    },
    Error(String),
    /// Tickets loaded for the swimlane view.
    TicketsLoaded {
        project: String,
        milestone: String,
        backlog: Vec<crate::actions::types::TicketInfo>,
        in_progress: Vec<crate::actions::types::TicketInfo>,
        done: Vec<crate::actions::types::TicketInfo>,
    },
    /// A ticket was moved; the updated swimlane data is included.
    TicketMoved {
        project: String,
        milestone: String,
        backlog: Vec<crate::actions::types::TicketInfo>,
        in_progress: Vec<crate::actions::types::TicketInfo>,
        done: Vec<crate::actions::types::TicketInfo>,
    },
}

pub fn spawn_query_actor(
    config: Config,
    mut rx: mpsc::Receiver<QueryRequest>,
    tx: mpsc::Sender<QueryResult>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Some(req) = rx.blocking_recv() {
            // Drain channel for debounce
            let req = drain_to_latest(req, &mut rx);

            let result = match req {
                QueryRequest::Search { query } => {
                    match actions::search::run(&config, &query, None, None, 20) {
                        Ok(results) => QueryResult::SearchResults(results),
                        Err(e) => QueryResult::Error(e.to_string()),
                    }
                }
                QueryRequest::Context {
                    topic,
                    depth,
                    limit,
                } => match actions::context::run(&config, &topic, depth, limit) {
                    Ok(results) => QueryResult::ContextResults(results),
                    Err(e) => QueryResult::Error(e.to_string()),
                },
                QueryRequest::Index { force } => {
                    match actions::index::run(&config, force, None, None, |msg| {
                        let _ = tx.blocking_send(QueryResult::Progress {
                            message: msg.to_string(),
                        });
                    }) {
                        Ok(stats) => QueryResult::IndexComplete(stats),
                        Err(e) => QueryResult::Error(e.to_string()),
                    }
                }
                QueryRequest::Normalize {
                    project,
                    dry_run,
                    fix_slugs,
                } => {
                    match actions::normalize::run(&config, project.as_deref(), dry_run, fix_slugs) {
                        Ok(summary) => QueryResult::NormalizeComplete(summary),
                        Err(e) => QueryResult::Error(e.to_string()),
                    }
                }
                QueryRequest::LoadTickets { project, milestone } => {
                    match load_swimlane_tickets(&config, &project, &milestone) {
                        Ok((backlog, in_progress, done)) => QueryResult::TicketsLoaded {
                            project,
                            milestone,
                            backlog,
                            in_progress,
                            done,
                        },
                        Err(e) => QueryResult::Error(e.to_string()),
                    }
                }
                QueryRequest::MoveTicket {
                    slug,
                    project,
                    milestone,
                    stage,
                    scope,
                } => {
                    let move_result = actions::ticket::move_ticket(
                        &config,
                        &slug,
                        stage.as_deref(),
                        None,
                        Some(&project),
                        scope.as_deref(),
                    );
                    match move_result {
                        Err(e) => QueryResult::Error(e.to_string()),
                        Ok(()) => match load_swimlane_tickets(&config, &project, &milestone) {
                            Ok((backlog, in_progress, done)) => QueryResult::TicketMoved {
                                project,
                                milestone,
                                backlog,
                                in_progress,
                                done,
                            },
                            Err(e) => QueryResult::Error(e.to_string()),
                        },
                    }
                }
            };

            if tx.blocking_send(result).is_err() {
                break; // TUI closed
            }
        }
    })
}

/// Load tickets for a project/milestone and split into swimlane columns.
/// Returns (backlog, in_progress, done).
fn load_swimlane_tickets(
    config: &Config,
    project: &str,
    milestone: &str,
) -> crate::error::Result<(
    Vec<crate::actions::types::TicketInfo>,
    Vec<crate::actions::types::TicketInfo>,
    Vec<crate::actions::types::TicketInfo>,
)> {
    let ms_filter = if milestone == "__all__" {
        None
    } else {
        Some(milestone)
    };
    let all = actions::ticket::load_tickets(config, Some(project), ms_filter)?;
    let mut backlog = Vec::new();
    let mut in_progress = Vec::new();
    let mut done = Vec::new();
    for ticket in all {
        match ticket.stage.as_str() {
            "in-progress" => in_progress.push(ticket),
            "done" | "cancelled" => done.push(ticket),
            _ => backlog.push(ticket),
        }
    }
    Ok((backlog, in_progress, done))
}

fn drain_to_latest(current: QueryRequest, rx: &mut mpsc::Receiver<QueryRequest>) -> QueryRequest {
    let mut latest = current;
    while let Ok(newer) = rx.try_recv() {
        latest = newer;
    }
    latest
}
