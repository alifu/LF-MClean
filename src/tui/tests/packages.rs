use super::*;

// Packages.

#[test]
fn o_ticks_older_versions_that_no_project_uses() {
    let fx = Fixture::new();

    let (app, _) = press(&on_packages(&fx, Usage::default()), &[Key::Char('o')]);

    // 10.25.0 is older than the newest but a lock file names it.
    assert_eq!(ticked_names(&app), ["Firebase/10.20.0"]);
}

#[test]
fn u_ticks_unused_packages_only_when_every_project_was_read() {
    let fx = Fixture::new();
    let mut incomplete = Usage::default();
    incomplete
        .problems
        .push("Documents/Locked could not be searched".to_string());

    let (sure, _) = press(&on_packages(&fx, Usage::default()), &[Key::Char('u')]);
    let (unsure, _) = press(&on_packages(&fx, incomplete), &[Key::Char('u')]);

    assert_eq!(ticked_names(&sure), ["Alamofire/5.8.0"]);
    assert_eq!(ticked_names(&unsure), Vec::<String>::new());
    assert!(unsure.message.unwrap().contains("not certain"));
}

#[test]
fn k_on_a_package_keeps_every_version_and_on_a_version_only_it() {
    let fx = Fixture::new();
    let packages = on_packages(&fx, Usage::default());

    let (package, _) = press(&packages, &[Key::Down, Key::Char('k')]);
    let (version, _) = press(
        &packages,
        &[Key::Down, Key::Right, Key::Down, Key::Char('k')],
    );

    let written = |app: &App| {
        app.keep
            .clone()
            .unwrap()
            .packages()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(written(&package), ["pods:Firebase"]);
    // Made-up items carry no version in their path, so a version keep falls back to the item.
    assert_eq!(
        version.keep.clone().unwrap().targets().count() + written(&version).len(),
        1
    );
}

#[test]
fn a_package_the_keep_file_cannot_name_is_kept_by_its_folder() {
    let fx = Fixture::new();
    let mut found = crate::tui::testkit::browse();
    found.packages[0].name = "Odd Name".to_string();
    let sections = crate::tui::sections::package_sections(
        &found,
        &Usage::default(),
        &KeepList::empty(&fx.env),
        now(),
    );

    let group = &sections[0].groups[0];

    assert_eq!(group.label, "Odd Name");
    assert!(matches!(
        group.keep_ref,
        crate::tui::list::KeepRef::Target(_)
    ));
}
