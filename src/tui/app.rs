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
use crate::actions::types::{
    IndexStats, MilestoneInfo, NormalizeSummary, SearchHit, TicketInfo, VaultDocument,
};
use crate::config::Config;

// ---------------------------------------------------------------------------
// Enums & structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Board,
    Search,
    Context,
    Maintain,
}

#[derive(Debug, Clone)]
pub enum BoardLevel {
    Projects {
        selected: usize,
        projects: Vec<String>,
    },
    Milestones {
        project: String,
        selected: usize,
        milestones: Vec<MilestoneWithCounts>,
    },
    Swimlanes {
        project: String,
        milestone: String,
        column: usize,
        row: usize,
        columns: [Vec<TicketInfo>; 3],
    },
}

#[derive(Debug, Clone)]
pub struct MilestoneWithCounts {
    pub info: MilestoneInfo,
    pub backlog: usize,
    pub in_progress: usize,
    pub done: usize,
}

#[derive(Debug, Clone)]
pub struct BoardState {
    pub level: BoardLevel,
}

#[derive(Debug, Clone)]
pub struct SearchState {
    pub query: String,
    pub cursor_pos: usize,
    pub results: Vec<SearchHit>,
    pub selected: usize,
    pub input_focused: bool,
    pub loading: bool,
}

#[derive(Debug, Clone)]
pub struct ContextNeighbor {
    pub label: String,
    pub file_path: String,
    pub note_type: String,
    pub score: f32,
    pub depth: usize,
}

#[derive(Debug, Clone)]
pub struct ContextState {
    pub center_stack: Vec<String>,
    pub current_center: String,
    pub depth: usize,
    pub neighbors: Vec<ContextNeighbor>,
    pub selected: usize,
    pub loading: bool,
    pub input_active: bool,
    pub input_text: String,
}

#[derive(Debug, Clone)]
pub struct MaintainState {
    pub index_stats: Option<IndexStats>,
    pub last_normalize: Option<NormalizeSummary>,
    pub progress_message: Option<String>,
    pub running: bool,
}

#[derive(Debug, Clone)]
pub struct ViewerState {
    pub document: VaultDocument,
    pub scroll_offset: usize,
    pub source_label: String,
}

#[derive(Debug, Clone)]
pub enum Screen {
    Board(BoardState),
    Search(SearchState),
    Context(ContextState),
    Maintain(MaintainState),
    Viewer(ViewerState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupState {
    None,
    StagePicker {
        slug: String,
        project: String,
        selected: usize,
    },
    ScopePicker {
        slug: String,
        project: String,
        selected: usize,
    },
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    Quit,
    SwitchTab(Tab),
    Enter,
    Escape,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    EnterCommandMode,
    SubmitCommand(String),
    CommandInput(char),
    CommandBackspace,
    ToggleHelp,
    // Search-specific
    SearchInput(char),
    SearchBackspace,
    SearchFocusResults,
    SearchRefocusInput,
    // Context-specific
    ContextActivateInput,
    ContextInput(char),
    ContextBackspace,
    ContextSubmitInput,
    ContextRecenter,
    ContextDepthUp,
    ContextDepthDown,
    // Search-to-context
    OpenContextForSelected,
    // Viewer
    OpenEditor,
    // Ticket mutation popups
    OpenStagePicker,
    OpenScopePicker,
    // Maintain actions
    IndexRebuild,
    NormalizeRun,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    stack: Vec<Screen>,
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
            stack: vec![Screen::Board(board)],
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
            Screen::Board(BoardState {
                level: BoardLevel::Milestones {
                    project: proj.name.clone(),
                    selected: 0,
                    milestones,
                },
            })
        } else {
            Screen::Board(BoardState {
                level: BoardLevel::Projects {
                    selected: 0,
                    projects: project_names.clone(),
                },
            })
        };

        Ok(Self {
            stack: vec![root_screen],
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
            Screen::Board(_) => Tab::Board,
            Screen::Search(_) => Tab::Search,
            Screen::Context(_) => Tab::Context,
            Screen::Maintain(_) => Tab::Maintain,
            Screen::Viewer(_) => Tab::Board, // viewer is always pushed atop something
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
            ("Board", Tab::Board),
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
            Screen::Board(board_state) => {
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

    // -- Query results ------------------------------------------------------

    pub fn handle_query_result(&mut self, result: QueryResult) {
        match result {
            QueryResult::SearchResults(sr) => {
                if let Screen::Search(s) = self.current_screen_mut() {
                    s.results = sr.hits;
                    s.loading = false;
                }
            }
            QueryResult::ContextResults(cr) => {
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
            QueryResult::IndexComplete(stats) => {
                if let Screen::Maintain(s) = self.current_screen_mut() {
                    s.index_stats = Some(stats);
                    s.running = false;
                    s.progress_message = None;
                }
            }
            QueryResult::NormalizeComplete(summary) => {
                if let Screen::Maintain(s) = self.current_screen_mut() {
                    s.last_normalize = Some(summary);
                    s.running = false;
                    s.progress_message = None;
                }
            }
            QueryResult::Progress { message } => {
                if let Screen::Maintain(s) = self.current_screen_mut() {
                    s.progress_message = Some(message);
                }
            }
            QueryResult::Error(msg) => {
                tracing::warn!("query actor error: {}", msg);
            }
            QueryResult::TicketsLoaded {
                project,
                milestone,
                backlog,
                in_progress,
                done,
            }
            | QueryResult::TicketMoved {
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
            if let Screen::Board(board) = screen {
                if let BoardLevel::Swimlanes {
                    project: ref p,
                    milestone: ref m,
                    columns,
                    row,
                    ..
                } = &mut board.level
                {
                    if p == project && m == milestone {
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

    // -- Helpers ------------------------------------------------------------

    /// Return the (slug, project) of the currently selected ticket, if any.
    /// Works for both Swimlanes and Viewer screens.
    fn selected_ticket_identity(&self) -> Option<(String, String)> {
        match self.current_screen() {
            Screen::Board(board) => {
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
    fn current_swimlane_milestone(&self) -> Option<(String, String)> {
        match self.current_screen() {
            Screen::Board(board) => {
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

    /// Confirm the active popup: send the mutation request and close the popup.
    fn confirm_popup(&mut self) {
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
                            let _ = tx.try_send(super::query_actor::QueryRequest::MoveTicket {
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
                            let _ = tx.try_send(super::query_actor::QueryRequest::MoveTicket {
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

    /// Send the current search query to the query actor (non-blocking).
    /// Only sends if the query is non-empty; clears loading flag if empty.
    fn send_search_query(&mut self) {
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
            let _ = tx.try_send(super::query_actor::QueryRequest::Search { query });
        }
    }

    fn handle_enter(&mut self) {
        let screen = self.current_screen().clone();
        match screen {
            Screen::Board(ref board) => match &board.level {
                BoardLevel::Projects { selected, projects } => {
                    if let Some(proj) = projects.get(*selected) {
                        let milestones = self
                            .config
                            .as_ref()
                            .map(|c| load_milestones_with_counts(c, proj))
                            .unwrap_or_default();
                        let next = Screen::Board(BoardState {
                            level: BoardLevel::Milestones {
                                project: proj.clone(),
                                selected: 0,
                                milestones,
                            },
                        });
                        self.stack.push(next);
                    }
                }
                BoardLevel::Milestones {
                    project,
                    selected,
                    milestones,
                } => {
                    if let Some(ms) = milestones.get(*selected) {
                        let proj = project.clone();
                        let ms_slug = ms.info.slug.clone();
                        let next = Screen::Board(BoardState {
                            level: BoardLevel::Swimlanes {
                                project: proj.clone(),
                                milestone: ms_slug.clone(),
                                column: 0,
                                row: 0,
                                columns: [vec![], vec![], vec![]],
                            },
                        });
                        self.stack.push(next);
                        // Kick off ticket loading for the swimlane
                        if let Some(tx) = &self.req_tx {
                            let _ = tx.try_send(super::query_actor::QueryRequest::LoadTickets {
                                project: proj,
                                milestone: ms_slug,
                            });
                        }
                    }
                }
                BoardLevel::Swimlanes {
                    columns,
                    column,
                    row,
                    project,
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
                                        document: doc,
                                        scroll_offset: 0,
                                        source_label: format!(
                                            "Board > {} > {}",
                                            project, ticket.title
                                        ),
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
                            document: doc,
                            scroll_offset: 0,
                            source_label: "Search".into(),
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
                        self.stack.push(Screen::Viewer(ViewerState {
                            document: doc,
                            scroll_offset: 0,
                            source_label: "Context".into(),
                        }));
                    }
                }
            }
            Screen::Maintain(_) | Screen::Viewer(_) => {}
        }
    }

    fn move_selection(&mut self, dir: Direction) {
        match self.current_screen_mut() {
            Screen::Board(board) => match &mut board.level {
                BoardLevel::Projects { selected, projects } => match dir {
                    Direction::Up => *selected = selected.saturating_sub(1),
                    Direction::Down => {
                        if !projects.is_empty() {
                            *selected = (*selected + 1).min(projects.len() - 1);
                        }
                    }
                    _ => {}
                },
                BoardLevel::Milestones {
                    selected,
                    milestones,
                    ..
                } => match dir {
                    Direction::Up => *selected = selected.saturating_sub(1),
                    Direction::Down => {
                        if !milestones.is_empty() {
                            *selected = (*selected + 1).min(milestones.len() - 1);
                        }
                    }
                    _ => {}
                },
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
    }

    fn make_root_screen(&self, tab: Tab) -> Screen {
        match tab {
            Tab::Board => {
                // If we have an inferred project, load its milestones
                if let (Some(proj), Some(config)) = (&self.inferred_project, &self.config) {
                    let milestones = load_milestones_with_counts(config, proj);
                    Screen::Board(BoardState {
                        level: BoardLevel::Milestones {
                            project: proj.clone(),
                            selected: 0,
                            milestones,
                        },
                    })
                } else {
                    Screen::Board(BoardState {
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

    milestones
        .into_iter()
        .map(|info| {
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
        })
        .collect()
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
