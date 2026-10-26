use super::*;

// The keep list screen.

#[test]
fn the_keep_screen_removes_marked_entries_on_enter() {
    let fx = Fixture::new();
    let keep = KeepList::empty(&fx.env)
        .with_target("xcode.archives")
        .unwrap()
        .with_path("/Volumes/Work/keep")
        .unwrap();
    let start = App::new(Mode::Trash, &fx.env, Ok(keep), None, now());

    let (on_keep, _) = start.on_key(Key::Char('4'));
    let (saved, effects) = press(&on_keep, &[Key::Down, Key::Char(' '), Key::Enter]);

    let left = saved.keep.clone().unwrap();
    assert_eq!(left.targets().collect::<Vec<_>>(), ["xcode.archives"]);
    assert_eq!(left.paths().count(), 0);
    assert_eq!(effects, [Effect::SaveKeep(left)]);
    assert_eq!(saved.screen, Screen::Menu);
}

#[test]
fn leaving_the_keep_screen_without_marks_changes_nothing() {
    let fx = Fixture::new();
    let (on_keep, _) = app(&fx, Mode::Trash).on_key(Key::Char('4'));

    assert_eq!(press(&on_keep, &[Key::Enter]).1, []);
    assert_eq!(press(&on_keep, &[Key::Char(' '), Key::Esc]).1, []);
}

#[test]
fn a_failed_keep_save_is_shown() {
    let fx = Fixture::new();

    let (app, _) = app(&fx, Mode::Trash).on_event(Event::KeepSaved(Err("disk full".to_string())));

    assert!(app.message.unwrap().contains("disk full"));
}
