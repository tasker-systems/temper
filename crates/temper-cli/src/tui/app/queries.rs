use super::state::*;
use super::App;

impl App {
    pub fn handle_query_result(&mut self, result: super::QueryResult) {
        match result {
            super::QueryResult::SearchResults(sr) => {
                if let Screen::Search(s) = self.current_screen_mut() {
                    s.results = sr.hits;
                    s.loading = false;
                }
            }
            super::QueryResult::ContextResults(cr) => {
                if let Screen::Context(s) = self.current_screen_mut() {
                    // Flatten hops into neighbor entries for display, tracking depth by hop index
                    s.neighbors = cr
                        .hops
                        .into_iter()
                        .enumerate()
                        .flat_map(|(hop_idx, hop)| {
                            hop.related_chunks.into_iter().map(move |group| {
                                let best_score =
                                    group.chunks.iter().map(|c| c.score).fold(0.0f32, f32::max);
                                ContextNeighbor {
                                    label: group.title,
                                    file_path: group.file_path,
                                    note_type: group.note_type,
                                    score: best_score,
                                    depth: hop_idx,
                                }
                            })
                        })
                        .collect();
                    s.loading = false;
                }
            }
            super::QueryResult::IndexComplete(stats) => {
                if let Screen::Maintain(s) = self.current_screen_mut() {
                    s.index_stats = Some(stats);
                    s.running = false;
                    s.progress_message = None;
                }
            }
            super::QueryResult::NormalizeComplete(summary) => {
                if let Screen::Maintain(s) = self.current_screen_mut() {
                    s.last_normalize = Some(summary);
                    s.running = false;
                    s.progress_message = None;
                }
            }
            super::QueryResult::Progress { message } => {
                if let Screen::Maintain(s) = self.current_screen_mut() {
                    s.progress_message = Some(message);
                }
            }
            super::QueryResult::Error(msg) => {
                tracing::warn!("query actor error: {}", msg);
            }
            super::QueryResult::TicketsLoaded {
                project,
                milestone,
                backlog,
                in_progress,
                done,
            }
            | super::QueryResult::TicketMoved {
                project,
                milestone,
                backlog,
                in_progress,
                done,
            } => {
                self.apply_swimlane_columns(&project, &milestone, backlog, in_progress, done);
            }
        }
    }

    /// Apply loaded ticket columns to the matching Swimlanes screen in the stack.
    fn apply_swimlane_columns(
        &mut self,
        project: &str,
        milestone: &str,
        backlog: Vec<crate::actions::types::TicketInfo>,
        in_progress: Vec<crate::actions::types::TicketInfo>,
        done: Vec<crate::actions::types::TicketInfo>,
    ) {
        for screen in &mut self.stack {
            if let Screen::Projects(board) = screen {
                if let BoardLevel::Swimlanes {
                    load_project: ref lp,
                    load_milestone: ref lm,
                    columns,
                    row,
                    ..
                } = &mut board.level
                {
                    if lp == project && lm == milestone {
                        columns[0] = backlog;
                        columns[1] = in_progress;
                        columns[2] = done;
                        // Clamp row so it doesn't point past end
                        // (column clamping happens in move_selection)
                        *row = 0;
                        return;
                    }
                }
            }
        }
    }

    /// Send the current search query to the query actor (non-blocking).
    /// Only sends if the query is non-empty; clears loading flag if empty.
    pub(super) fn send_search_query(&mut self) {
        let query = if let Screen::Search(s) = self.current_screen() {
            s.query.clone()
        } else {
            return;
        };

        if query.is_empty() {
            if let Screen::Search(s) = self.current_screen_mut() {
                s.loading = false;
                s.results.clear();
            }
            return;
        }

        if let Some(tx) = &self.req_tx {
            let _ = tx.try_send(super::super::query_actor::QueryRequest::Search { query });
        }
    }
}
