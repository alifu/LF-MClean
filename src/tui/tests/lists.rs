use super::*;

// Ticking.

#[test]
fn space_on_a_group_ticks_all_its_items_and_again_unticks_them() {
    let fx = Fixture::new();
    let clean = on_clean(&fx);

    let (ticked, _) = press(&clean, &[Key::Char(' ')]);
    let (unticked, _) = press(&ticked, &[Key::Char(' ')]);

    assert_eq!(ticked_names(&ticked), ["App-abc", "Lib-def"]);
    assert_eq!(ticked_names(&unticked), Vec::<String>::new());
}

#[test]
fn an_open_group_ticks_one_item_at_a_time() {
    let fx = Fixture::new();
    let clean = on_clean(&fx);

    let (app, _) = press(&clean, &[Key::Right, Key::Down, Key::Down, Key::Char(' ')]);

    assert_eq!(ticked_names(&app), ["Lib-def"]);
    let group = app.clean_list.group(0, 0);
    assert_eq!(app.clean_list.tick_of(group), Tick::Some);
    let (closed, _) = press(&app, &[Key::Left]);
    assert_eq!(
        closed.clean_list.current(),
        Some(crate::tui::list::Row::Group(0, 0))
    );
}

#[test]
fn all_safe_never_ticks_caution_or_risky_items() {
    let fx = Fixture::new();

    let (app, _) = press(&on_clean(&fx), &[Key::Char('a')]);

    assert_eq!(ticked_names(&app), ["App-abc", "Lib-def", "modules-2"]);
    let (none, _) = press(&app, &[Key::Char('n')]);
    assert_eq!(ticked_names(&none), Vec::<String>::new());
}

#[test]
fn the_cursor_skips_headings_and_stops_at_the_ends() {
    let fx = Fixture::new();
    let clean = on_clean(&fx);

    let (top, _) = press(&clean, &[Key::Up, Key::Up]);
    let (bottom, _) = press(&clean, &[Key::Down; 20]);

    assert_eq!(top.clean_list.cursor, 1, "row 0 is the Xcode heading");
    let rows = bottom.clean_list.rows();
    assert_eq!(bottom.clean_list.cursor, rows.len() - 1);
}

// Keeping.

#[test]
fn k_keeps_a_target_saves_it_and_unticks_its_items() {
    let fx = Fixture::new();
    let (ticked, _) = press(&on_clean(&fx), &[Key::Char(' ')]);

    let (kept, effects) = press(&ticked, &[Key::Char('k')]);

    let keep = kept.keep.clone().unwrap();
    assert_eq!(keep.targets().collect::<Vec<_>>(), ["xcode.derived-data"]);
    assert_eq!(effects, [Effect::SaveKeep(keep)]);
    assert_eq!(ticked_names(&kept), Vec::<String>::new());
    assert!(kept.clean_list.group(0, 0).all_kept());
}

#[test]
fn a_kept_group_cannot_be_ticked_and_says_why() {
    let fx = Fixture::new();
    let (kept, _) = press(&on_clean(&fx), &[Key::Char('k')]);

    let (after, _) = press(&kept, &[Key::Char(' ')]);
    let (all_safe, _) = press(&kept, &[Key::Char('a')]);

    assert_eq!(ticked_names(&after), Vec::<String>::new());
    assert!(after.message.as_deref().unwrap().contains("Kept"));
    assert_eq!(ticked_names(&all_safe), ["modules-2"]);
}

#[test]
fn k_again_stops_keeping() {
    let fx = Fixture::new();

    let (app, effects) = press(&on_clean(&fx), &[Key::Char('k'), Key::Char('k')]);

    assert!(app.keep.unwrap().is_empty());
    assert_eq!(effects.len(), 2);
}

#[test]
fn a_keep_list_loaded_at_start_marks_items() {
    let fx = Fixture::new();
    let keep = KeepList::empty(&fx.env).with_target("android.avd").unwrap();
    let start = App::new(Mode::Trash, &fx.env, Ok(keep), None, now());

    let (app, _) = start.on_key(Key::Char('1'));
    let app = app
        .on_event(Event::Scanned(0, crate::tui::testkit::report()))
        .0;

    let android = app
        .clean_list
        .sections
        .iter()
        .find(|s| s.title == "Android")
        .unwrap();
    assert!(android.groups[0].all_kept());
}

#[test]
fn a_broken_keep_list_blocks_keeping_and_removing() {
    let fx = Fixture::new();
    let start = App::new(
        Mode::Trash,
        &fx.env,
        Err("bad line 3".to_string()),
        None,
        now(),
    );
    let (app, _) = start.on_key(Key::Char('1'));
    let app = app
        .on_event(Event::Scanned(0, crate::tui::testkit::report()))
        .0;

    let (kept, keep_effects) = press(&app, &[Key::Char('k')]);
    let (review, review_effects) = press(&app, &[Key::Char(' '), Key::Enter]);

    assert_eq!(keep_effects, []);
    assert!(kept.message.unwrap().contains("bad line 3"));
    assert_eq!(review_effects, []);
    assert_eq!(review.screen, Screen::Clean);
    assert!(review.message.unwrap().contains("keep list"));
}

#[test]
fn r_scans_the_screen_again_and_drops_the_ticks() {
    let fx = Fixture::new();
    let (ticked, _) = press(&on_clean(&fx), &[Key::Char('a')]);
    let (packages, _) = press(&on_packages(&fx, Usage::default()), &[Key::Char('r')]);
    let (purge, purge_effects) = press(&on_purge(&fx), &[Key::Char('r')]);

    let (again, effects) = ticked.on_key(Key::Char('r'));

    assert_eq!(effects, [Effect::Scan(1)]);
    assert!(matches!(again.scan, Loading::Running(_)));
    assert_eq!(ticked_names(&again), Vec::<String>::new());
    assert_eq!(packages.packages_generation, 1);
    assert!(matches!(purge.purge.found, Loading::Running(_)));
    assert!(matches!(purge_effects[..], [Effect::ScanPurge(1, _)]));
}

#[test]
fn r_does_nothing_while_a_scan_is_running() {
    let fx = Fixture::new();
    let (scanning, _) = app(&fx, Mode::Trash).on_key(Key::Char('1'));

    let (after, effects) = scanning.on_key(Key::Char('r'));

    assert_eq!(effects, []);
    assert_eq!(after.scan_generation, 0);
    assert_eq!(after.message.as_deref(), Some("Still scanning."));
}
