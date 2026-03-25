mod actions;
mod focus;
mod queries;
pub mod state;
pub use state::*;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tokio::sync::mpsc;

use super::query_actor::{QueryRequest, QueryResult};
use super::tabs::board;
use super::tabs::context;
use super::tabs::maintain;
use super::tabs::search;
use super::views::popup::{render_popup, scope_options, stage_options};
use super::views::viewer;
use super::widgets::command_line::render_command_line;
use super::widgets::keyhints::render_keyhints;
use crate::actions::types::MilestoneInfo;
use crate::config::Config;

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    stack: Vec<Screen>,
    pub focus: FocusRegion,
    pub command_mode: bool,
    pub command_input: String,
    pub should_quit: bool,
    pub popup: PopupState,
    pub show_help: bool,
    pub editor_request: Option<String>,
    req_tx: Option<mpsc::Sender<QueryRequest>>,
    /// Project names from config, used when creating fresh Board screens.
    project_names: Vec<String>,
    /// The CWD-resolved project name, if any.
    inferred_project: Option<String>,
    /// Hold a clone of config for loading data.
    config: Option<Config>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("stack", &self.stack)
            .field("focus", &self.focus)
            .field("command_mode", &self.command_mode)
            .field("command_input", &self.command_input)
            .field("should_quit", &self.should_quit)
            .field("popup", &self.popup)
            .field("show_help", &self.show_help)
            .field("editor_request", &self.editor_request)
            .finish()
    }
}

impl App {
    /// Create an App suitable for tests — starts on Board / Milestones with one
    /// dummy milestone so that `Enter` can push a Swimlanes screen.
    pub fn new_for_test() -> Self {
        let milestone = MilestoneWithCounts {
            info: MilestoneInfo {
                title: "v0.1".into(),
                slug: "v0-1".into(),
                project: "demo".into(),
                seq: 1,
                status: "active".into(),
            },
            backlog: 0,
            in_progress: 0,
            done: 0,
        };
        let board = BoardState {
            level: BoardLevel::Milestones {
                project: "demo".into(),
                selected: 0,
                milestones: vec![milestone],
            },
        };
        Self {
            stack: vec![Screen::Projects(board)],
            focus: FocusRegion::Primary,
            command_mode: false,
            command_input: String::new(),
            should_quit: false,
            popup: PopupState::None,
            show_help: false,
            editor_request: None,
            req_tx: None,
            project_names: vec!["demo".into()],
            inferred_project: Some("demo".into()),
            config: None,
        }
    }

    /// Create with a specific root screen.
    pub fn new(root: Screen) -> Self {
        Self {
            stack: vec![root],
            focus: FocusRegion::Primary,
            command_mode: false,
            command_input: String::new(),
            should_quit: false,
            popup: PopupState::None,
            show_help: false,
            editor_request: None,
            req_tx: None,
            project_names: vec![],
            inferred_project: None,
            config: None,
        }
    }

    /// Create from config and a query request sender. Resolves the current
    /// project from CWD and starts on the Board tab at the appropriate level.
    pub fn from_config(
        config: &Config,
        req_tx: mpsc::Sender<QueryRequest>,
    ) -> crate::error::Result<Self> {
        let cwd = std::env::current_dir()?;
        let project = crate::project::resolve_from_cwd(&cwd, &config.projects);
        let project_names: Vec<String> = config.projects.keys().cloned().collect();
        let inferred_project = project.as_ref().map(|p| p.name.clone());

        let root_screen = if let Some(proj) = project {
            // Load milestones synchronously for the resolved project
            let milestones = load_milestones_with_counts(config, &proj.name);
            Screen::Projects(BoardState {
                level: BoardLevel::Milestones {
                    project: proj.name.clone(),
                    selected: 0,
                    milestones,
                },
            })
        } else {
            Screen::Projects(BoardState {
                level: BoardLevel::Projects {
                    selected: 0,
                    projects: project_names.clone(),
                },
            })
        };

        Ok(Self {
            stack: vec![root_screen],
            focus: FocusRegion::Primary,
            command_mode: false,
            command_input: String::new(),
            should_quit: false,
            popup: PopupState::None,
            show_help: false,
            editor_request: None,
            req_tx: Some(req_tx),
            project_names,
            inferred_project,
            config: Some(config.clone()),
        })
    }

    // -- Accessors ----------------------------------------------------------

    pub fn active_tab(&self) -> Tab {
        match &self.stack[0] {
            Screen::Projects(_) => Tab::Projects,
            Screen::Search(_) => Tab::Search,
            Screen::Context(_) => Tab::Context,
            Screen::Maintain(_) => Tab::Maintain,
            Screen::Viewer(_) => Tab::Projects, // viewer is always pushed atop something
        }
    }

    pub fn stack_depth(&self) -> usize {
        self.stack.len()
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn in_command_mode(&self) -> bool {
        self.command_mode
    }

    pub fn in_search_input(&self) -> bool {
        if let Screen::Search(s) = self.current_screen() {
            s.input_focused
        } else {
            false
        }
    }

    /// Returns true when on the Search screen with focus on the results list
    /// (not the input). Used by the event mapper to bind `/` to refocus input.
    pub fn in_search_results(&self) -> bool {
        if let Screen::Search(s) = self.current_screen() {
            !s.input_focused
        } else {
            false
        }
    }

    /// Returns true when on the Context screen with topic input active.
    pub fn in_context_input(&self) -> bool {
        if let Screen::Context(s) = self.current_screen() {
            s.input_active
        } else {
            false
        }
    }

    /// Returns true when on the Context screen (input not active).
    pub fn in_context_results(&self) -> bool {
        if let Screen::Context(s) = self.current_screen() {
            !s.input_active
        } else {
            false
        }
    }

    /// Returns true when on the Viewer screen.
    pub fn in_viewer(&self) -> bool {
        matches!(self.current_screen(), Screen::Viewer(_))
    }

    /// Returns true when a popup is active (stage or scope picker).
    pub fn in_popup(&self) -> bool {
        self.popup != PopupState::None
    }

    pub fn current_screen(&self) -> &Screen {
        self.stack.last().expect("stack must never be empty")
    }

    pub fn current_screen_mut(&mut self) -> &mut Screen {
        self.stack.last_mut().expect("stack must never be empty")
    }

    // -- Dispatch -----------------------------------------------------------

    pub fn dispatch(&mut self, action: AppAction) {
        // Handle popup first
        if self.popup != PopupState::None {
            match action {
                AppAction::Escape => {
                    self.popup = PopupState::None;
                    return;
                }
                AppAction::MoveUp => {
                    match &mut self.popup {
                        PopupState::StagePicker { selected, .. } => {
                            *selected = selected.saturating_sub(1);
                        }
                        PopupState::ScopePicker { selected, .. } => {
                            *selected = selected.saturating_sub(1);
                        }
                        PopupState::None => {}
                    }
                    return;
                }
                AppAction::MoveDown => {
                    let max = match &self.popup {
                        PopupState::StagePicker { .. } => 3,
                        PopupState::ScopePicker { .. } => 2,
                        PopupState::None => 0,
                    };
                    match &mut self.popup {
                        PopupState::StagePicker { selected, .. }
                        | PopupState::ScopePicker { selected, .. } => {
                            *selected = (*selected + 1).min(max);
                        }
                        PopupState::None => {}
                    }
                    return;
                }
                AppAction::Enter => {
                    self.confirm_popup();
                    return;
                }
                _ => return,
            }
        }

        // Handle command mode
        if self.command_mode {
            match action {
                AppAction::Escape => {
                    self.command_mode = false;
                    self.command_input.clear();
                    return;
                }
                AppAction::SubmitCommand(_) => {
                    let cmd = std::mem::take(&mut self.command_input);
                    self.command_mode = false;
                    if let Some(resolved) = super::event::parse_command(&cmd) {
                        self.dispatch(resolved);
                    }
                    return;
                }
                AppAction::CommandInput(ch) => {
                    self.command_input.push(ch);
                    return;
                }
                AppAction::CommandBackspace => {
                    self.command_input.pop();
                    return;
                }
                _ => return,
            }
        }

        match action {
            AppAction::Quit => {
                self.should_quit = true;
            }

            AppAction::SwitchTab(tab) => {
                let root = self.make_root_screen(tab);
                self.stack = vec![root];
                self.reset_focus();
            }

            AppAction::Enter => {
                self.handle_enter();
            }

            AppAction::Escape => {
                if self.show_help {
                    self.show_help = false;
                } else if matches!(self.current_screen(), Screen::Search(s) if s.input_focused) {
                    // Unfocus search input — move to results or allow tab switch
                    if let Screen::Search(s) = self.current_screen_mut() {
                        s.input_focused = false;
                    }
                } else if matches!(self.current_screen(), Screen::Search(_)) {
                    // On search results — pop back if stacked
                    if self.stack.len() > 1 {
                        self.stack.pop();
                    }
                } else if let Screen::Context(s) = self.current_screen_mut() {
                    if s.input_active {
                        // Cancel input without changing center
                        s.input_active = false;
                        s.input_text.clear();
                    } else if !s.center_stack.is_empty() {
                        // Pop to the previous center and re-query
                        let prev = s.center_stack.pop().expect("just checked non-empty");
                        s.current_center = prev.clone();
                        s.neighbors.clear();
                        s.selected = 0;
                        s.loading = true;
                        let depth = s.depth;
                        if let Some(tx) = &self.req_tx {
                            let _ = tx.try_send(super::query_actor::QueryRequest::Context {
                                topic: prev,
                                depth,
                                limit: 20,
                            });
                        }
                    } else if self.stack.len() > 1 {
                        self.stack.pop();
                    }
                } else if self.stack.len() > 1 {
                    self.stack.pop();
                }
            }

            AppAction::MoveUp => self.move_selection(Direction::Up),
            AppAction::MoveDown => self.move_selection(Direction::Down),
            AppAction::MoveLeft => self.move_selection(Direction::Left),
            AppAction::MoveRight => self.move_selection(Direction::Right),

            AppAction::EnterCommandMode => {
                self.command_mode = true;
                self.command_input.clear();
            }

            AppAction::SubmitCommand(cmd) => {
                if let Some(resolved) = super::event::parse_command(&cmd) {
                    self.dispatch(resolved);
                }
            }

            AppAction::ToggleHelp => {
                self.show_help = !self.show_help;
            }

            AppAction::SearchInput(ch) => {
                if let Screen::Search(s) = self.current_screen_mut() {
                    s.query.insert(s.cursor_pos, ch);
                    s.cursor_pos += 1;
                    s.loading = true;
                }
                self.send_search_query();
            }
            AppAction::SearchBackspace => {
                if let Screen::Search(s) = self.current_screen_mut() {
                    if s.cursor_pos > 0 {
                        s.cursor_pos -= 1;
                        s.query.remove(s.cursor_pos);
                        s.loading = !s.query.is_empty();
                    }
                }
                self.send_search_query();
            }
            AppAction::SearchFocusResults => {
                if let Screen::Search(s) = self.current_screen_mut() {
                    s.input_focused = false;
                }
            }
            AppAction::SearchRefocusInput => {
                if let Screen::Search(s) = self.current_screen_mut() {
                    s.input_focused = true;
                }
            }

            AppAction::ContextActivateInput => {
                if let Screen::Context(s) = self.current_screen_mut() {
                    s.input_active = true;
                    s.input_text.clear();
                }
            }
            AppAction::ContextInput(ch) => {
                if let Screen::Context(s) = self.current_screen_mut() {
                    s.input_text.push(ch);
                }
            }
            AppAction::ContextBackspace => {
                if let Screen::Context(s) = self.current_screen_mut() {
                    s.input_text.pop();
                }
            }
            AppAction::ContextSubmitInput => {
                let (topic, depth) = if let Screen::Context(s) = self.current_screen_mut() {
                    let topic = std::mem::take(&mut s.input_text);
                    s.input_active = false;
                    if topic.is_empty() {
                        return;
                    }
                    s.current_center = topic.clone();
                    s.center_stack.clear();
                    s.neighbors.clear();
                    s.selected = 0;
                    s.loading = true;
                    (topic, s.depth)
                } else {
                    return;
                };
                if let Some(tx) = &self.req_tx {
                    let _ = tx.try_send(super::query_actor::QueryRequest::Context {
                        topic,
                        depth,
                        limit: 20,
                    });
                }
            }
            AppAction::ContextRecenter => {
                // Re-center on the currently selected neighbor: push current center, set new
                let (topic, depth) = if let Screen::Context(s) = self.current_screen_mut() {
                    if let Some(neighbor) = s.neighbors.get(s.selected) {
                        let new_center = neighbor.label.clone();
                        let old_center = s.current_center.clone();
                        s.center_stack.push(old_center);
                        s.current_center = new_center.clone();
                        s.neighbors.clear();
                        s.selected = 0;
                        s.loading = true;
                        (new_center, s.depth)
                    } else {
                        return;
                    }
                } else {
                    return;
                };
                if let Some(tx) = &self.req_tx {
                    let _ = tx.try_send(super::query_actor::QueryRequest::Context {
                        topic,
                        depth,
                        limit: 20,
                    });
                }
            }
            AppAction::ContextDepthUp => {
                let (topic, depth) = if let Screen::Context(s) = self.current_screen_mut() {
                    if s.current_center.is_empty() {
                        return;
                    }
                    s.depth = (s.depth + 1).min(3);
                    s.loading = true;
                    (s.current_center.clone(), s.depth)
                } else {
                    return;
                };
                if let Some(tx) = &self.req_tx {
                    let _ = tx.try_send(super::query_actor::QueryRequest::Context {
                        topic,
                        depth,
                        limit: 20,
                    });
                }
            }
            AppAction::ContextDepthDown => {
                let (topic, depth) = if let Screen::Context(s) = self.current_screen_mut() {
                    if s.current_center.is_empty() {
                        return;
                    }
                    s.depth = s.depth.saturating_sub(1).max(1);
                    s.loading = true;
                    (s.current_center.clone(), s.depth)
                } else {
                    return;
                };
                if let Some(tx) = &self.req_tx {
                    let _ = tx.try_send(super::query_actor::QueryRequest::Context {
                        topic,
                        depth,
                        limit: 20,
                    });
                }
            }
            AppAction::OpenEditor => {
                if let Screen::Viewer(v) = self.current_screen() {
                    self.editor_request = Some(v.document.path.clone());
                }
            }

            AppAction::OpenContextForSelected => {
                // Wire 'c' from Search to open Context tab centered on selected result
                let topic = if let Screen::Search(s) = self.current_screen() {
                    if s.input_focused {
                        return;
                    }
                    s.results.get(s.selected).map(|hit| hit.file_path.clone())
                } else {
                    return;
                };
                if let Some(topic) = topic {
                    let context_screen = Screen::Context(ContextState {
                        center_stack: vec![],
                        current_center: topic.clone(),
                        depth: 1,
                        neighbors: vec![],
                        selected: 0,
                        loading: true,
                        input_active: false,
                        input_text: String::new(),
                    });
                    self.stack = vec![context_screen];
                    if let Some(tx) = &self.req_tx {
                        let _ = tx.try_send(super::query_actor::QueryRequest::Context {
                            topic,
                            depth: 1,
                            limit: 20,
                        });
                    }
                }
            }

            AppAction::OpenStagePicker => {
                if let Some((slug, project)) = self.selected_ticket_identity() {
                    self.popup = PopupState::StagePicker {
                        slug,
                        project,
                        selected: 0,
                    };
                }
            }

            AppAction::OpenScopePicker => {
                if let Some((slug, project)) = self.selected_ticket_identity() {
                    self.popup = PopupState::ScopePicker {
                        slug,
                        project,
                        selected: 0,
                    };
                }
            }

            AppAction::IndexRebuild => {
                if let Screen::Maintain(s) = self.current_screen_mut() {
                    s.running = true;
                    s.progress_message = None;
                }
                if let Some(tx) = &self.req_tx {
                    let _ = tx.try_send(super::query_actor::QueryRequest::Index { force: true });
                }
            }

            AppAction::NormalizeRun => {
                if let Screen::Maintain(s) = self.current_screen_mut() {
                    s.running = true;
                    s.progress_message = None;
                }
                if let Some(tx) = &self.req_tx {
                    let _ = tx.try_send(super::query_actor::QueryRequest::Normalize {
                        project: None,
                        dry_run: false,
                        fix_slugs: false,
                    });
                }
            }

            AppAction::FocusNext => self.focus_next(),
            AppAction::FocusPrev => self.focus_prev(),

            AppAction::CommandInput(_) | AppAction::CommandBackspace => {
                // only meaningful in command mode, handled above
            }
        }
    }

    // -- Render -------------------------------------------------------------

    pub fn render(&self, frame: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(ratatui::prelude::Direction::Vertical)
            .constraints([
                Constraint::Length(1), // tab bar
                Constraint::Min(1),    // content area
                Constraint::Length(1), // key hints / command line
            ])
            .split(frame.area());

        // Tab bar — custom Line with Spans for active/inactive styling
        let active_tab = self.active_tab();
        let tab_defs: &[(&str, Tab)] = &[
            ("Projects", Tab::Projects),
            ("Search", Tab::Search),
            ("Context", Tab::Context),
            ("Maintain", Tab::Maintain),
        ];
        let divider_style = Style::default().fg(Color::DarkGray);
        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::raw("  "));
        for (i, (name, tab)) in tab_defs.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", divider_style));
            }
            if *tab == active_tab {
                spans.push(Span::styled(
                    *name,
                    Style::default()
                        .fg(Color::White)
                        .bold()
                        .add_modifier(Modifier::UNDERLINED),
                ));
            } else {
                spans.push(Span::styled(*name, Style::default().fg(Color::DarkGray)));
            }
        }
        let tab_bar = Paragraph::new(Line::from(spans));
        frame.render_widget(tab_bar, chunks[0]);

        // Content area
        match self.current_screen() {
            Screen::Projects(board_state) => {
                board::render_board(frame, chunks[1], board_state);
            }
            Screen::Search(search_state) => {
                search::render_search(frame, chunks[1], search_state);
            }
            Screen::Context(context_state) => {
                context::render_context(frame, chunks[1], context_state);
            }
            Screen::Viewer(viewer_state) => {
                viewer::render_viewer(frame, chunks[1], viewer_state);
            }
            Screen::Maintain(s) => {
                maintain::render_maintain(frame, chunks[1], s);
            }
        }

        // Bottom bar: key hints or command line
        if self.command_mode {
            render_command_line(frame, chunks[2], &self.command_input);
        } else {
            render_keyhints(frame, chunks[2], self.current_screen());
        }

        // Popup overlay (rendered last so it appears on top)
        match &self.popup {
            PopupState::None => {}
            PopupState::StagePicker { selected, .. } => {
                let opts = stage_options();
                render_popup(frame, chunks[1], " Move Stage ", &opts, *selected);
            }
            PopupState::ScopePicker { selected, .. } => {
                let opts = scope_options();
                render_popup(frame, chunks[1], " Set Scope ", &opts, *selected);
            }
        }

        // Help overlay (rendered on top of everything)
        if self.show_help {
            render_help_overlay(frame, frame.area());
        }
    }

    fn make_root_screen(&self, tab: Tab) -> Screen {
        match tab {
            Tab::Projects => {
                // If we have an inferred project, load its milestones
                if let (Some(proj), Some(config)) = (&self.inferred_project, &self.config) {
                    let milestones = load_milestones_with_counts(config, proj);
                    Screen::Projects(BoardState {
                        level: BoardLevel::Milestones {
                            project: proj.clone(),
                            selected: 0,
                            milestones,
                        },
                    })
                } else {
                    Screen::Projects(BoardState {
                        level: BoardLevel::Projects {
                            selected: 0,
                            projects: self.project_names.clone(),
                        },
                    })
                }
            }
            Tab::Search => Screen::Search(SearchState {
                query: String::new(),
                cursor_pos: 0,
                results: vec![],
                selected: 0,
                input_focused: true,
                loading: false,
            }),
            Tab::Context => Screen::Context(ContextState {
                center_stack: vec![],
                current_center: String::new(),
                depth: 1,
                neighbors: vec![],
                selected: 0,
                loading: false,
                input_active: true,
                input_text: String::new(),
            }),
            Tab::Maintain => Screen::Maintain(MaintainState {
                index_stats: None,
                last_normalize: None,
                progress_message: None,
                running: false,
            }),
        }
    }
}

enum Direction {
    Up,
    Down,
    Left,
    Right,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_milestones_with_counts(config: &Config, project: &str) -> Vec<MilestoneWithCounts> {
    let milestones = match crate::actions::milestone::load_milestones(config, Some(project)) {
        Ok(ms) => ms,
        Err(_) => return vec![],
    };
    let counts =
        crate::actions::milestone::count_tickets_by_stage(config, project).unwrap_or_default();

    // "All Tickets" synthetic entry — every ticket across the entire vault
    let all_tickets = {
        let all = crate::actions::ticket::load_tickets(config, None, None).unwrap_or_default();
        let mut backlog = 0usize;
        let mut in_progress = 0usize;
        let mut done = 0usize;
        for t in &all {
            match t.stage.as_str() {
                "in-progress" => in_progress += 1,
                "done" | "cancelled" => done += 1,
                _ => backlog += 1,
            }
        }
        MilestoneWithCounts {
            info: MilestoneInfo {
                title: "(All Tickets)".into(),
                slug: "__all__".into(),
                project: "__all__".into(),
                seq: 0,
                status: "active".into(),
            },
            backlog,
            in_progress,
            done,
        }
    };

    let mut result = vec![all_tickets];
    result.extend(milestones.into_iter().map(|info| {
        let slug_counts = counts.get(&info.slug);
        MilestoneWithCounts {
            backlog: slug_counts
                .and_then(|c| c.get("backlog"))
                .copied()
                .unwrap_or(0),
            in_progress: slug_counts
                .and_then(|c| c.get("in-progress"))
                .copied()
                .unwrap_or(0),
            done: slug_counts
                .and_then(|c| c.get("done"))
                .copied()
                .unwrap_or(0),
            info,
        }
    }));
    result
}

fn render_help_overlay(frame: &mut ratatui::Frame, area: Rect) {
    // Overlay dimensions: ~44 chars wide, 27 lines tall
    const OVERLAY_W: u16 = 44;
    const OVERLAY_H: u16 = 27;

    let x = area.x + area.width.saturating_sub(OVERLAY_W) / 2;
    let y = area.y + area.height.saturating_sub(OVERLAY_H) / 2;
    let w = OVERLAY_W.min(area.width);
    let h = OVERLAY_H.min(area.height);
    let overlay_area = Rect::new(x, y, w, h);

    let help_text = vec![
        Line::from(Span::styled(
            "Navigation",
            Style::default().fg(Color::Yellow).bold(),
        )),
        Line::from("  1-4         Switch tab"),
        Line::from("  j/k ↑↓      Move selection"),
        Line::from("  h/l ←→      Columns / projects"),
        Line::from("  Enter       Open / drill in"),
        Line::from("  Esc         Back / up"),
        Line::from(""),
        Line::from(Span::styled(
            "Mutation",
            Style::default().fg(Color::Yellow).bold(),
        )),
        Line::from("  s           Change stage"),
        Line::from("  S           Change scope"),
        Line::from("  e           Open in $EDITOR"),
        Line::from(""),
        Line::from(Span::styled(
            "Search / Context",
            Style::default().fg(Color::Yellow).bold(),
        )),
        Line::from("  /           Focus search input"),
        Line::from("  c           Context from item"),
        Line::from("  +/-         Context depth"),
        Line::from("  Tab         Input → results"),
        Line::from(""),
        Line::from(Span::styled(
            "Maintenance",
            Style::default().fg(Color::Yellow).bold(),
        )),
        Line::from("  i           Rebuild index"),
        Line::from("  n           Run normalize"),
        Line::from(""),
        Line::from(Span::styled(
            "Command Mode",
            Style::default().fg(Color::Yellow).bold(),
        )),
        Line::from("  :q          Quit"),
        Line::from("  :b :s :c :m Switch tabs"),
        Line::from("  :? :h       This help"),
        Line::from(""),
        Line::from(Span::styled(
            "Press any key to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .title(Span::styled(
            " Help ",
            Style::default().fg(Color::White).bold(),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .style(Style::default().fg(Color::White));

    frame.render_widget(Clear, overlay_area);
    frame.render_widget(paragraph, overlay_area);
}

/// Launch the TUI event loop.
pub fn run(config: &Config) -> crate::error::Result<()> {
    // Terminal setup
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    // Ensure cleanup on panic
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        default_hook(info);
    }));

    // Create channels
    let (req_tx, req_rx) = tokio::sync::mpsc::channel::<super::query_actor::QueryRequest>(16);
    let (res_tx, mut res_rx) = tokio::sync::mpsc::channel::<QueryResult>(16);

    // Spawn query actor
    let _actor_handle = super::query_actor::spawn_query_actor(config.clone(), req_rx, res_tx);

    // Create app
    let mut app = App::from_config(config, req_tx)?;

    // Main loop
    let result: crate::error::Result<()> = loop {
        if let Err(e) = terminal.draw(|frame| app.render(frame)) {
            break Err(e.into());
        }

        // Check for query results (non-blocking)
        while let Ok(result) = res_rx.try_recv() {
            app.handle_query_result(result);
        }

        // Handle editor handoff if requested
        if let Some(path) = app.editor_request.take() {
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            crossterm::terminal::disable_raw_mode()?;
            crossterm::execute!(
                terminal.backend_mut(),
                crossterm::terminal::LeaveAlternateScreen
            )?;
            let _ = std::process::Command::new(&editor).arg(&path).status();
            crossterm::execute!(
                terminal.backend_mut(),
                crossterm::terminal::EnterAlternateScreen
            )?;
            crossterm::terminal::enable_raw_mode()?;
            terminal.clear()?;
            // Reload the document content into the viewer state
            if let Screen::Viewer(v) = app.current_screen_mut() {
                if let Ok(doc) = crate::actions::vault::read_document(std::path::Path::new(&path)) {
                    v.document = doc;
                    v.scroll_offset = 0;
                }
            }
        }

        // Poll for crossterm events with short timeout
        match crossterm::event::poll(std::time::Duration::from_millis(50)) {
            Ok(true) => match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(key)) => {
                    if key.kind == crossterm::event::KeyEventKind::Press {
                        if let Some(action) = super::event::map_key(
                            key,
                            app.in_popup(),
                            app.in_command_mode(),
                            app.in_search_input(),
                            app.in_search_results(),
                            app.in_context_input(),
                            app.in_context_results(),
                            app.in_viewer(),
                            app.show_help,
                        ) {
                            app.dispatch(action);
                        }
                    }
                }
                Ok(_) => {} // ignore mouse, resize, etc. for now
                Err(e) => {
                    break Err(e.into());
                }
            },
            Ok(false) => {} // no event, continue
            Err(e) => {
                break Err(e.into());
            }
        }

        if app.should_quit() {
            break Ok(());
        }
    };

    // Teardown
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result
}
