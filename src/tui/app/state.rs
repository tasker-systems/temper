use crate::actions::types::{
    IndexStats, MilestoneInfo, NormalizeSummary, SearchHit, TicketInfo, VaultDocument,
};

// ---------------------------------------------------------------------------
// Enums & structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Projects,
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
        /// Slugs used for matching query actor responses (may differ from display names)
        load_project: String,
        load_milestone: String,
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
    Projects(BoardState),
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
