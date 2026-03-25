use super::state::*;
use super::{load_milestones_with_counts, App, Direction};
use crate::actions::types::VaultDocument;

impl App {
    pub(super) fn handle_enter(&mut self) {
        let screen = self.current_screen().clone();
        match screen {
            Screen::Projects(ref board) => match &board.level {
                BoardLevel::ProjectList { detail, .. } => {
                    if self.focus == FocusRegion::Secondary {
                        // Enter on milestone — push swimlanes
                        if let Some(d) = detail {
                            if let Some(ms) = d.milestones.get(d.selected) {
                                let ms_slug = ms.info.slug.clone();
                                let ms_title = ms.info.title.clone();
                                let load_project = ms.info.project.clone();
                                let display_project = if load_project == "__all__" {
                                    "All".to_string()
                                } else {
                                    d.project.clone()
                                };
                                let next = Screen::Projects(BoardState {
                                    level: BoardLevel::Swimlanes {
                                        project: display_project,
                                        milestone: ms_title,
                                        load_project: load_project.clone(),
                                        load_milestone: ms_slug.clone(),
                                        column: 0,
                                        row: 0,
                                        columns: [vec![], vec![], vec![]],
                                    },
                                });
                                self.stack.push(next);
                                self.reset_focus();
                                if let Some(tx) = &self.req_tx {
                                    let _ = tx.try_send(
                                        super::super::query_actor::QueryRequest::LoadTickets {
                                            project: load_project,
                                            milestone: ms_slug,
                                        },
                                    );
                                }
                            }
                        }
                    } else {
                        // Enter on project — focus the detail panel
                        self.focus = FocusRegion::Secondary;
                        self.sync_focus_to_state();
                    }
                }
                BoardLevel::Swimlanes {
                    columns,
                    column,
                    row,
                    project,
                    milestone,
                    ..
                } => {
                    if let Some(tickets) = columns.get(*column) {
                        if let Some(ticket) = tickets.get(*row) {
                            if let Some(config) = &self.config {
                                let ticket_path = config
                                    .vault_root
                                    .join("tickets")
                                    .join(&ticket.project)
                                    .join(format!("{}.md", ticket.slug));
                                if let Ok(doc) = crate::actions::vault::read_document(&ticket_path)
                                {
                                    self.stack.push(Screen::Viewer(ViewerState {
                                        document: doc.clone(),
                                        scroll_offset: 0,
                                        source_label: format!(
                                            "Board > {} > {}",
                                            project, ticket.title
                                        ),
                                        breadcrumb_segments: vec![
                                            "All".into(),
                                            project.clone(),
                                            milestone.clone(),
                                            doc.title.clone(),
                                        ],
                                    }));
                                }
                            }
                        }
                    }
                }
            },
            Screen::Search(ref s) => {
                if !s.input_focused {
                    if let Some(hit) = s.results.get(s.selected) {
                        let doc = VaultDocument {
                            path: hit.file_path.clone(),
                            note_type: hit.note_type.clone(),
                            title: hit.file_path.clone(),
                            frontmatter: serde_yaml::Value::Null,
                            body: hit.content.clone(),
                        };
                        self.stack.push(Screen::Viewer(ViewerState {
                            document: doc.clone(),
                            scroll_offset: 0,
                            source_label: "Search".into(),
                            breadcrumb_segments: vec!["Search".into(), doc.title.clone()],
                        }));
                    }
                }
            }
            Screen::Context(ref s) => {
                if !s.input_active {
                    if let Some(neighbor) = s.neighbors.get(s.selected) {
                        let doc = VaultDocument {
                            path: neighbor.file_path.clone(),
                            note_type: neighbor.note_type.clone(),
                            title: neighbor.label.clone(),
                            frontmatter: serde_yaml::Value::Null,
                            body: String::new(),
                        };
                        let center_topic = s.current_center.clone();
                        self.stack.push(Screen::Viewer(ViewerState {
                            document: doc.clone(),
                            scroll_offset: 0,
                            source_label: "Context".into(),
                            breadcrumb_segments: vec![
                                "Context".into(),
                                center_topic,
                                doc.title.clone(),
                            ],
                        }));
                    }
                }
            }
            Screen::Maintain(_) | Screen::Viewer(_) => {}
        }
    }

    pub(super) fn move_selection(&mut self, dir: Direction) {
        let mut reload_project_detail = false;
        let current_focus = self.focus;

        match self.current_screen_mut() {
            Screen::Projects(board) => match &mut board.level {
                BoardLevel::ProjectList {
                    selected,
                    projects,
                    detail,
                } => {
                    if current_focus == FocusRegion::Secondary {
                        if let Some(d) = detail {
                            match dir {
                                Direction::Up => d.selected = d.selected.saturating_sub(1),
                                Direction::Down => {
                                    if !d.milestones.is_empty() {
                                        d.selected = (d.selected + 1).min(d.milestones.len() - 1);
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else {
                        match dir {
                            Direction::Up => {
                                *selected = selected.saturating_sub(1);
                                reload_project_detail = true;
                            }
                            Direction::Down => {
                                if !projects.is_empty() {
                                    *selected = (*selected + 1).min(projects.len() - 1);
                                    reload_project_detail = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                BoardLevel::Swimlanes {
                    column,
                    row,
                    columns,
                    ..
                } => match dir {
                    Direction::Left => *column = column.saturating_sub(1),
                    Direction::Right => *column = (*column + 1).min(2),
                    Direction::Up => *row = row.saturating_sub(1),
                    Direction::Down => {
                        let col = &columns[*column];
                        if !col.is_empty() {
                            *row = (*row + 1).min(col.len() - 1);
                        }
                    }
                },
            },
            Screen::Search(s) => {
                if !s.input_focused {
                    match dir {
                        Direction::Up => s.selected = s.selected.saturating_sub(1),
                        Direction::Down => {
                            if !s.results.is_empty() {
                                s.selected = (s.selected + 1).min(s.results.len() - 1);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Screen::Context(s) => {
                if !s.input_active {
                    match dir {
                        Direction::Up => s.selected = s.selected.saturating_sub(1),
                        Direction::Down => {
                            if !s.neighbors.is_empty() {
                                s.selected = (s.selected + 1).min(s.neighbors.len() - 1);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Screen::Maintain(_) => {}
            Screen::Viewer(v) => match dir {
                Direction::Up => v.scroll_offset = v.scroll_offset.saturating_sub(1),
                Direction::Down => v.scroll_offset += 1,
                _ => {}
            },
        }

        if reload_project_detail {
            self.load_detail_for_selected_project();
        }
    }

    /// Load milestones for the currently selected project in ProjectList and
    /// write them into the detail panel. Avoids borrow issues by extracting
    /// the project name first, loading data, then writing back.
    pub(super) fn load_detail_for_selected_project(&mut self) {
        let proj_name = if let Screen::Projects(board) = self.current_screen() {
            if let BoardLevel::ProjectList {
                selected, projects, ..
            } = &board.level
            {
                projects.get(*selected).cloned()
            } else {
                None
            }
        } else {
            None
        };

        let Some(proj_name) = proj_name else { return };

        let milestones = self
            .config
            .as_ref()
            .map(|c| load_milestones_with_counts(c, &proj_name))
            .unwrap_or_default();

        if let Screen::Projects(board) = self.current_screen_mut() {
            if let BoardLevel::ProjectList { detail, .. } = &mut board.level {
                *detail = Some(DetailPanel {
                    project: proj_name,
                    milestones,
                    selected: 0,
                });
            }
        }
    }

    /// Confirm the active popup: send the mutation request and close the popup.
    pub(super) fn confirm_popup(&mut self) {
        let popup = std::mem::replace(&mut self.popup, PopupState::None);
        let milestone_info = self.current_swimlane_milestone();

        match popup {
            PopupState::StagePicker {
                slug,
                project,
                selected,
            } => {
                let stages = ["backlog", "in-progress", "done", "cancelled"];
                if let Some(&stage) = stages.get(selected) {
                    if let Some((_, milestone)) = milestone_info {
                        if let Some(tx) = &self.req_tx {
                            let _ =
                                tx.try_send(super::super::query_actor::QueryRequest::MoveTicket {
                                    slug,
                                    project,
                                    milestone,
                                    stage: Some(stage.to_string()),
                                    scope: None,
                                });
                        }
                    }
                }
            }
            PopupState::ScopePicker {
                slug,
                project,
                selected,
            } => {
                let scopes = ["patch", "feature", "epic"];
                if let Some(&scope) = scopes.get(selected) {
                    if let Some((_, milestone)) = milestone_info {
                        if let Some(tx) = &self.req_tx {
                            let _ =
                                tx.try_send(super::super::query_actor::QueryRequest::MoveTicket {
                                    slug,
                                    project,
                                    milestone,
                                    stage: None,
                                    scope: Some(scope.to_string()),
                                });
                        }
                    }
                }
            }
            PopupState::None => {}
        }
    }

    /// Return the (slug, project) of the currently selected ticket, if any.
    /// Works for both Swimlanes and Viewer screens.
    pub(super) fn selected_ticket_identity(&self) -> Option<(String, String)> {
        match self.current_screen() {
            Screen::Projects(board) => {
                if let BoardLevel::Swimlanes {
                    project,
                    columns,
                    column,
                    row,
                    ..
                } = &board.level
                {
                    let ticket = columns.get(*column)?.get(*row)?;
                    Some((ticket.slug.clone(), project.clone()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Return the milestone slug for the current swimlane view, if any.
    pub(super) fn current_swimlane_milestone(&self) -> Option<(String, String)> {
        match self.current_screen() {
            Screen::Projects(board) => {
                if let BoardLevel::Swimlanes {
                    project, milestone, ..
                } = &board.level
                {
                    Some((project.clone(), milestone.clone()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
