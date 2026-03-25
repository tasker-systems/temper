use super::state::{BoardLevel, FocusRegion, Screen};
use super::App;

impl App {
    /// Returns the current focus region.
    pub fn focus_region(&self) -> FocusRegion {
        self.focus
    }

    /// Returns the ordered list of focusable regions for the current screen.
    pub fn focus_regions(&self) -> Vec<FocusRegion> {
        match self.current_screen() {
            Screen::Projects(board) => match &board.level {
                BoardLevel::ProjectList { detail, .. } => {
                    if detail.is_some() {
                        vec![
                            FocusRegion::TabBar,
                            FocusRegion::Primary,
                            FocusRegion::Secondary,
                        ]
                    } else {
                        vec![FocusRegion::TabBar, FocusRegion::Primary]
                    }
                }
                BoardLevel::Swimlanes { .. } => {
                    vec![
                        FocusRegion::TabBar,
                        FocusRegion::Tertiary(0),
                        FocusRegion::Tertiary(1),
                        FocusRegion::Tertiary(2),
                    ]
                }
            },
            Screen::Search(_) => {
                vec![
                    FocusRegion::TabBar,
                    FocusRegion::Primary,
                    FocusRegion::Secondary,
                ]
            }
            Screen::Context(_) => {
                vec![
                    FocusRegion::TabBar,
                    FocusRegion::Primary,
                    FocusRegion::Secondary,
                ]
            }
            Screen::Maintain(_) | Screen::Viewer(_) => {
                vec![FocusRegion::TabBar, FocusRegion::Primary]
            }
        }
    }

    /// Cycle focus forward, wrapping around.
    pub fn focus_next(&mut self) {
        let regions = self.focus_regions();
        if regions.is_empty() {
            return;
        }
        let current_idx = regions.iter().position(|r| *r == self.focus).unwrap_or(0);
        let next_idx = (current_idx + 1) % regions.len();
        self.focus = regions[next_idx];
        self.sync_focus_to_state();
    }

    /// Cycle focus backward, wrapping around.
    pub fn focus_prev(&mut self) {
        let regions = self.focus_regions();
        if regions.is_empty() {
            return;
        }
        let current_idx = regions.iter().position(|r| *r == self.focus).unwrap_or(0);
        let next_idx = if current_idx == 0 {
            regions.len() - 1
        } else {
            current_idx - 1
        };
        self.focus = regions[next_idx];
        self.sync_focus_to_state();
    }

    /// After focus changes, sync legacy state flags so existing rendering
    /// and event mapping still works correctly.
    pub(super) fn sync_focus_to_state(&mut self) {
        let focus = self.focus;
        match self.current_screen_mut() {
            Screen::Search(s) => {
                s.input_focused = focus == FocusRegion::Primary;
            }
            Screen::Context(s) => {
                s.input_active = focus == FocusRegion::Primary;
            }
            Screen::Projects(board) => {
                if let BoardLevel::Swimlanes { column, .. } = &mut board.level {
                    if let FocusRegion::Tertiary(idx) = focus {
                        *column = idx;
                    }
                }
            }
            _ => {}
        }
    }

    /// Reset focus to the first content region (skip TabBar), and sync state.
    pub fn reset_focus(&mut self) {
        let regions = self.focus_regions();
        // Pick the first non-TabBar region, or fall back to first region
        self.focus = regions
            .into_iter()
            .find(|r| *r != FocusRegion::TabBar)
            .unwrap_or(FocusRegion::Primary);
        self.sync_focus_to_state();
    }
}
