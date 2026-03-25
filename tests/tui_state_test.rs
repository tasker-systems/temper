use temper_cli::tui::app::{App, AppAction, Tab};
use temper_cli::tui::event::parse_command;

#[test]
fn test_tab_switch_replaces_stack() {
    let mut app = App::new_for_test();
    assert_eq!(app.active_tab(), Tab::Board);
    assert_eq!(app.stack_depth(), 1);

    app.dispatch(AppAction::SwitchTab(Tab::Search));
    assert_eq!(app.active_tab(), Tab::Search);
    assert_eq!(app.stack_depth(), 1);

    app.dispatch(AppAction::SwitchTab(Tab::Context));
    assert_eq!(app.active_tab(), Tab::Context);
    assert_eq!(app.stack_depth(), 1);

    app.dispatch(AppAction::SwitchTab(Tab::Maintain));
    assert_eq!(app.active_tab(), Tab::Maintain);
    assert_eq!(app.stack_depth(), 1);

    app.dispatch(AppAction::SwitchTab(Tab::Board));
    assert_eq!(app.active_tab(), Tab::Board);
    assert_eq!(app.stack_depth(), 1);
}

#[test]
fn test_esc_pops_stack() {
    let mut app = App::new_for_test();
    // new_for_test starts at Milestones with one milestone
    assert_eq!(app.stack_depth(), 1);

    // Enter drills into Swimlanes
    app.dispatch(AppAction::Enter);
    assert_eq!(app.stack_depth(), 2);

    // Esc pops back
    app.dispatch(AppAction::Escape);
    assert_eq!(app.stack_depth(), 1);
}

#[test]
fn test_esc_on_root_does_nothing() {
    let mut app = App::new_for_test();
    assert_eq!(app.stack_depth(), 1);

    app.dispatch(AppAction::Escape);
    assert_eq!(app.stack_depth(), 1);
}

#[test]
fn test_command_mode_parsing() {
    // Single-letter abbreviations
    assert_eq!(parse_command("q"), Some(AppAction::Quit));
    assert_eq!(parse_command("b"), Some(AppAction::SwitchTab(Tab::Board)));
    assert_eq!(parse_command("s"), Some(AppAction::SwitchTab(Tab::Search)));
    assert_eq!(parse_command("c"), Some(AppAction::SwitchTab(Tab::Context)));
    assert_eq!(
        parse_command("m"),
        Some(AppAction::SwitchTab(Tab::Maintain))
    );

    // Full words
    assert_eq!(parse_command("quit"), Some(AppAction::Quit));
    assert_eq!(
        parse_command("board"),
        Some(AppAction::SwitchTab(Tab::Board))
    );
    assert_eq!(
        parse_command("search"),
        Some(AppAction::SwitchTab(Tab::Search))
    );
    assert_eq!(
        parse_command("context"),
        Some(AppAction::SwitchTab(Tab::Context))
    );
    assert_eq!(
        parse_command("maintain"),
        Some(AppAction::SwitchTab(Tab::Maintain))
    );

    // Help
    assert_eq!(parse_command("?"), Some(AppAction::ToggleHelp));
    assert_eq!(parse_command("h"), Some(AppAction::ToggleHelp));
    assert_eq!(parse_command("help"), Some(AppAction::ToggleHelp));

    // Unknown returns None
    assert_eq!(parse_command("xyz"), None);
    assert_eq!(parse_command(""), None);
}
