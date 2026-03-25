use temper_cli::tui::app::state::FocusRegion;
use temper_cli::tui::app::{App, AppAction, Tab};

#[test]
fn initial_focus_is_content() {
    let app = App::new_for_test();
    assert_ne!(app.focus_region(), FocusRegion::TabBar);
}

#[test]
fn tab_cycles_focus_forward() {
    let mut app = App::new_for_test();
    let initial = app.focus_region();
    app.dispatch(AppAction::FocusNext);
    let after = app.focus_region();
    assert_ne!(initial, after);
}

#[test]
fn shift_tab_cycles_focus_backward() {
    let mut app = App::new_for_test();
    let initial = app.focus_region();
    app.dispatch(AppAction::FocusNext);
    app.dispatch(AppAction::FocusPrev);
    assert_eq!(app.focus_region(), initial);
}

#[test]
fn focus_wraps_around() {
    let mut app = App::new_for_test();
    let initial = app.focus_region();
    for _ in 0..20 {
        app.dispatch(AppAction::FocusNext);
        if app.focus_region() == initial {
            return;
        }
    }
    panic!("focus did not wrap after 20 tabs");
}

#[test]
fn tab_switch_resets_focus_to_first_content() {
    let mut app = App::new_for_test();
    app.dispatch(AppAction::FocusNext);
    app.dispatch(AppAction::SwitchTab(Tab::Search));
    assert_ne!(app.focus_region(), FocusRegion::TabBar);
}
