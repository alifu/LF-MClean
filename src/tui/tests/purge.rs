use super::effects::run_clean;
use super::*;

// Purge.

#[test]
fn opening_purge_searches_the_project_folders_once() {
    let fx = Fixture::new();
    let folders = app(&fx, Mode::Trash).purge.folders;

    let (first, effects) = app(&fx, Mode::Trash).on_key(Key::Char('3'));
    let (again, again_effects) = press(&first, &[Key::Esc, Key::Char('3')]);

    assert_eq!(effects, [Effect::ScanPurge(0, folders)]);
    assert_eq!(again_effects, []);
    assert!(matches!(again.purge.found, Loading::Running(_)));
}

#[test]
fn purge_lists_one_group_per_project_and_a_folder_can_be_ticked() {
    let fx = Fixture::new();
    let start = on_purge(&fx);

    let (app, _) = press(&start, &[Key::Down, Key::Right, Key::Down, Key::Char(' ')]);

    let groups: Vec<_> = start.purge.list.sections[0]
        .groups
        .iter()
        .map(|g| g.label.as_str())
        .collect();
    assert_eq!(groups, ["Library (1)", "ShopApp (3)"]);
    assert_eq!(ticked_names(&app), ["composeApp/build"]);
}

#[test]
fn all_safe_in_purge_skips_only_the_caution_folder_of_a_project() {
    let fx = Fixture::new();

    let (app, _) = press(&on_purge(&fx), &[Key::Char('a')]);

    assert_eq!(ticked_names(&app), ["build", "composeApp/build", ".gradle"]);
}

#[test]
fn the_review_labels_each_purge_folder_with_its_own_safety() {
    let fx = Fixture::new();
    let (app, _) = press(
        &on_purge(&fx),
        &[
            Key::Down,
            Key::Right,
            Key::Down,
            Key::Down,
            Key::Down,
            Key::Char(' '),
            Key::Enter,
        ],
    );

    let safeties: Vec<_> = app.review.unwrap().items.iter().map(|(_, s)| *s).collect();

    assert_eq!(safeties, [crate::catalog::Safety::Caution]);
}

#[test]
fn k_on_a_project_keeps_it_and_its_folders_cannot_be_ticked() {
    let fx = Fixture::new();
    let (app, effects) = press(&on_purge(&fx), &[Key::Char('k')]);

    assert!(matches!(&effects[..], [Effect::SaveKeep(_)]));
    assert!(
        app.purge.list.sections[0].groups[0]
            .entries
            .iter()
            .all(|e| !e.kept)
    );
    assert!(app.keep.unwrap().paths().next().is_some());
}

#[test]
fn a_stale_project_search_is_dropped_while_a_new_one_runs() {
    let fx = Fixture::new();
    let (summary, _) = press(
        &on_purge(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );
    let summary = summary.on_event(Event::CleanFinished(None)).0;
    let (running, effects) = press(&summary, &[Key::Enter, Key::Char('3')]);

    let after = running
        .on_event(Event::PurgeScanned(
            0,
            crate::tui::testkit::found_projects(),
        ))
        .0;

    assert!(matches!(&effects[..], [Effect::ScanPurge(1, _)]));
    assert!(matches!(after.purge.found, Loading::Running(_)));
}

#[test]
fn purge_review_leads_to_a_clean_of_the_ticked_folders() {
    let fx = Fixture::new();
    let (review, _) = press(&on_purge(&fx), &[Key::Char('a'), Key::Enter]);

    let (cleaning, effects) = press(&review, &[Key::Char('y')]);

    assert_eq!(cleaning.screen, Screen::Cleaning);
    assert!(matches!(&effects[..], [Effect::Clean(items)] if items.len() == 3));
}

#[test]
fn project_results_from_before_a_reset_are_dropped() {
    let fx = Fixture::new();
    // Back at the menu with a finished clean: the summary screen resets the search.
    let (summary, _) = press(
        &on_purge(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );
    let summary = summary.on_event(Event::CleanFinished(None)).0;
    let (reset, _) = press(&summary, &[Key::Enter]);

    let after = reset
        .on_event(Event::PurgeScanned(
            0,
            crate::tui::testkit::found_projects(),
        ))
        .0;

    assert!(matches!(after.purge.found, Loading::Idle));
}

#[test]
fn a_crashed_project_search_can_be_started_again() {
    let fx = Fixture::new();
    let (searching, _) = app(&fx, Mode::Trash).on_key(Key::Char('3'));

    let (crashed, _) = searching.on_event(Event::Crashed("The project search".to_string()));
    let (_, effects) = press(&crashed, &[Key::Esc, Key::Char('3')]);

    assert!(matches!(crashed.purge.found, Loading::Idle));
    assert!(matches!(&effects[..], [Effect::ScanPurge(0, _)]));
}

#[test]
fn problems_while_searching_are_mentioned() {
    let fx = Fixture::new();
    let (searching, _) = app(&fx, Mode::Trash).on_key(Key::Char('3'));
    let mut found = crate::tui::testkit::found_projects();
    found.problems = vec!["/x could not be searched: denied".to_string()];

    let (after, _) = searching.on_event(Event::PurgeScanned(0, found));

    assert!(after.message.unwrap().contains("1 folder"));
}

#[test]
fn purge_folders_can_be_replaced() {
    let fx = Fixture::new();
    let folders = vec![fx.home().join("Code")];

    let app = app(&fx, Mode::Trash).with_purge_folders(folders.clone());

    assert_eq!(
        app.on_key(Key::Char('3')).1,
        [Effect::ScanPurge(0, folders)]
    );
}

fn project_with_build(fx: &Fixture) -> std::path::PathBuf {
    fx.file("Projects/App/settings.gradle.kts");
    fx.dir("Projects/App/build");
    fx.dir("Projects/App/src");
    fx.home().join("Projects/App")
}

fn found_on_disk(fx: &Fixture) -> Vec<crate::scan::Item> {
    crate::purge::find_projects(&fx.env, &[fx.home().join("Projects")])
        .projects
        .into_iter()
        .flat_map(|p| p.items)
        .collect()
}

#[test]
fn purging_removes_only_the_build_folder_of_the_project() {
    let fx = Fixture::new();
    let root = project_with_build(&fx);
    let items = found_on_disk(&fx);

    let events = run_clean(&fx, &items);

    assert!(matches!(&events[0], Event::Cleaned(d) if matches!(d.outcome, Outcome::Removed(_))));
    assert!(!root.join("build").exists());
    assert!(root.join("src/data.bin").exists());
    assert!(root.join("settings.gradle.kts").exists());
    assert!(fx.outside_is_intact());
}

#[test]
fn a_kept_project_is_not_purged() {
    let fx = Fixture::new();
    let root = project_with_build(&fx);
    let items = found_on_disk(&fx);
    KeepList::empty(&fx.env)
        .with_path("~/Projects/App")
        .unwrap()
        .save(&fx.env)
        .unwrap();

    let events = run_clean(&fx, &items);

    assert!(matches!(&events[0], Event::Cleaned(d) if d.outcome == Outcome::Kept));
    assert!(root.join("build").exists());
}

#[test]
fn a_build_folder_that_stopped_being_one_is_refused_at_removal_time() {
    let fx = Fixture::new();
    let root = project_with_build(&fx);
    let items = found_on_disk(&fx);
    std::fs::remove_file(root.join("settings.gradle.kts")).unwrap();

    let events = run_clean(&fx, &items);

    assert!(matches!(&events[0], Event::Cleaned(d) if matches!(d.outcome, Outcome::Failed(_))));
    assert!(root.join("build").exists());
}

#[test]
fn the_project_search_effect_finds_projects() {
    let fx = Fixture::new();
    project_with_build(&fx);
    let (sender, receiver) = std::sync::mpsc::channel();
    let folders = vec![fx.home().join("Projects")];

    crate::tui::perform(
        Effect::ScanPurge(0, folders),
        &fx.env,
        Mode::DryRun,
        &sender,
    );

    let Event::PurgeScanned(0, found) = receiver
        .recv_timeout(std::time::Duration::from_secs(20))
        .unwrap()
    else {
        panic!("unexpected event")
    };
    assert_eq!(found.projects[0].name, "App");
}
