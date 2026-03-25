use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{AppAction, Tab};

/// Map a crossterm key event to an `AppAction`, taking into account whether
/// we are currently in popup mode, command-mode, search-input mode,
/// search-results mode, context-input mode, context-results mode, or the viewer.
#[expect(
    clippy::too_many_arguments,
    reason = "all mode flags are distinct boolean signals needed for routing"
)]
pub fn map_key(
    key: KeyEvent,
    in_popup: bool,
    in_command_mode: bool,
    in_search_input: bool,
    in_search_results: bool,
    in_context_input: bool,
    in_context_results: bool,
    in_viewer: bool,
    show_help: bool,
) -> Option<AppAction> {
    // Ctrl-C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(AppAction::Quit);
    }

    // Help overlay: any key press dismisses it
    if show_help {
        return Some(AppAction::ToggleHelp);
    }

    if in_popup {
        return map_popup(key);
    }

    if in_command_mode {
        return map_command_mode(key);
    }

    if in_search_input {
        return map_search_input(key);
    }

    if in_search_results {
        return map_search_results(key);
    }

    if in_context_input {
        return map_context_input(key);
    }

    if in_context_results {
        return map_context_results(key);
    }

    if in_viewer {
        return map_viewer(key);
    }

    map_normal(key)
}

fn map_popup(key: KeyEvent) -> Option<AppAction> {
    match key.code {
        KeyCode::Esc => Some(AppAction::Escape),
        KeyCode::Char('j') | KeyCode::Down => Some(AppAction::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(AppAction::MoveUp),
        KeyCode::Enter => Some(AppAction::Enter),
        _ => None,
    }
}

fn map_command_mode(key: KeyEvent) -> Option<AppAction> {
    match key.code {
        KeyCode::Enter => Some(AppAction::SubmitCommand(String::new())), // caller fills from App.command_input
        KeyCode::Esc => Some(AppAction::Escape),
        KeyCode::Backspace => Some(AppAction::CommandBackspace),
        KeyCode::Char(ch) => Some(AppAction::CommandInput(ch)),
        _ => None,
    }
}

fn map_search_input(key: KeyEvent) -> Option<AppAction> {
    match key.code {
        KeyCode::Esc => Some(AppAction::Escape),
        KeyCode::Tab | KeyCode::Down => Some(AppAction::SearchFocusResults),
        KeyCode::Backspace => Some(AppAction::SearchBackspace),
        KeyCode::Char(ch) => Some(AppAction::SearchInput(ch)),
        _ => None,
    }
}

fn map_search_results(key: KeyEvent) -> Option<AppAction> {
    match key.code {
        KeyCode::Esc => Some(AppAction::Escape),
        KeyCode::Char('/') => Some(AppAction::SearchRefocusInput),
        KeyCode::Char('j') | KeyCode::Down => Some(AppAction::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(AppAction::MoveUp),
        KeyCode::Enter => Some(AppAction::Enter),
        KeyCode::Char('c') => Some(AppAction::OpenContextForSelected),
        KeyCode::Char('q') => Some(AppAction::Quit),
        KeyCode::Char(':') => Some(AppAction::EnterCommandMode),
        KeyCode::Char('1') => Some(AppAction::SwitchTab(Tab::Projects)),
        KeyCode::Char('2') => Some(AppAction::SwitchTab(Tab::Search)),
        KeyCode::Char('3') => Some(AppAction::SwitchTab(Tab::Context)),
        KeyCode::Char('4') => Some(AppAction::SwitchTab(Tab::Maintain)),
        KeyCode::Char('?') => Some(AppAction::ToggleHelp),
        _ => None,
    }
}

fn map_context_input(key: KeyEvent) -> Option<AppAction> {
    match key.code {
        KeyCode::Esc => Some(AppAction::Escape),
        KeyCode::Enter => Some(AppAction::ContextSubmitInput),
        KeyCode::Backspace => Some(AppAction::ContextBackspace),
        KeyCode::Char(ch) => Some(AppAction::ContextInput(ch)),
        _ => None,
    }
}

fn map_context_results(key: KeyEvent) -> Option<AppAction> {
    match key.code {
        KeyCode::Esc => Some(AppAction::Escape),
        KeyCode::Char('/') => Some(AppAction::ContextActivateInput),
        KeyCode::Char('j') | KeyCode::Down => Some(AppAction::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(AppAction::MoveUp),
        KeyCode::Enter => Some(AppAction::Enter),
        KeyCode::Char('c') => Some(AppAction::ContextRecenter),
        KeyCode::Char('+') | KeyCode::Char('=') => Some(AppAction::ContextDepthUp),
        KeyCode::Char('-') => Some(AppAction::ContextDepthDown),
        KeyCode::Char('q') => Some(AppAction::Quit),
        KeyCode::Char(':') => Some(AppAction::EnterCommandMode),
        KeyCode::Char('1') => Some(AppAction::SwitchTab(Tab::Projects)),
        KeyCode::Char('2') => Some(AppAction::SwitchTab(Tab::Search)),
        KeyCode::Char('3') => Some(AppAction::SwitchTab(Tab::Context)),
        KeyCode::Char('4') => Some(AppAction::SwitchTab(Tab::Maintain)),
        KeyCode::Char('?') => Some(AppAction::ToggleHelp),
        _ => None,
    }
}

fn map_viewer(key: KeyEvent) -> Option<AppAction> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(AppAction::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(AppAction::MoveUp),
        KeyCode::Esc | KeyCode::Char('q') => Some(AppAction::Escape),
        KeyCode::Char('e') => Some(AppAction::OpenEditor),
        KeyCode::Char(':') => Some(AppAction::EnterCommandMode),
        KeyCode::Char('?') => Some(AppAction::ToggleHelp),
        _ => None,
    }
}

fn map_normal(key: KeyEvent) -> Option<AppAction> {
    match key.code {
        // Movement — vim keys
        KeyCode::Char('j') | KeyCode::Down => Some(AppAction::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(AppAction::MoveUp),
        KeyCode::Char('h') | KeyCode::Left => Some(AppAction::MoveLeft),
        KeyCode::Char('l') | KeyCode::Right => Some(AppAction::MoveRight),

        // Enter / Esc
        KeyCode::Enter => Some(AppAction::Enter),
        KeyCode::Esc => Some(AppAction::Escape),

        // Command mode
        KeyCode::Char(':') => Some(AppAction::EnterCommandMode),

        // Quick search
        KeyCode::Char('/') => Some(AppAction::SwitchTab(Tab::Search)),

        // Tab shortcuts (1-based)
        KeyCode::Char('1') => Some(AppAction::SwitchTab(Tab::Projects)),
        KeyCode::Char('2') => Some(AppAction::SwitchTab(Tab::Search)),
        KeyCode::Char('3') => Some(AppAction::SwitchTab(Tab::Context)),
        KeyCode::Char('4') => Some(AppAction::SwitchTab(Tab::Maintain)),

        // Ticket mutation
        KeyCode::Char('s') => Some(AppAction::OpenStagePicker),
        KeyCode::Char('S') => Some(AppAction::OpenScopePicker),

        // Maintain actions
        KeyCode::Char('i') => Some(AppAction::IndexRebuild),
        KeyCode::Char('n') => Some(AppAction::NormalizeRun),

        // Misc
        KeyCode::Char('q') => Some(AppAction::Quit),
        KeyCode::Char('?') => Some(AppAction::ToggleHelp),

        _ => None,
    }
}

/// Parse a colon-command string into an `AppAction`.
/// Supports abbreviations: first-letter matching.
pub fn parse_command(input: &str) -> Option<AppAction> {
    let trimmed = input.trim();
    match trimmed {
        "q" | "quit" => Some(AppAction::Quit),
        "p" | "projects" | "b" | "board" => Some(AppAction::SwitchTab(Tab::Projects)),
        "s" | "search" => Some(AppAction::SwitchTab(Tab::Search)),
        "c" | "context" => Some(AppAction::SwitchTab(Tab::Context)),
        "m" | "maintain" => Some(AppAction::SwitchTab(Tab::Maintain)),
        "?" | "h" | "help" => Some(AppAction::ToggleHelp),
        _ => None,
    }
}
