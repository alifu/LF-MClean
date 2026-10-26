use super::*;

// Reviewing and cleaning.

#[test]
fn enter_with_nothing_ticked_says_so() {
    let fx = Fixture::new();

    let (app, _) = press(&on_clean(&fx), &[Key::Enter]);

    assert_eq!(app.screen, Screen::Clean);
    assert!(app.message.unwrap().contains("Nothing is ticked"));
}

#[test]
fn safe_items_are_removed_after_y() {
    let fx = Fixture::new();
    let (review, _) = press(&on_clean(&fx), &[Key::Char('a'), Key::Enter]);

    let (cleaning, effects) = press(&review, &[Key::Char('y')]);

    assert_eq!(review.screen, Screen::Review);
    assert!(!review.review.as_ref().unwrap().needs_word());
    assert_eq!(cleaning.screen, Screen::Cleaning);
    let Effect::Clean(items) = &effects[0] else {
        panic!("{effects:?}")
    };
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["App-abc", "Lib-def", "modules-2"]);
}

#[test]
fn a_review_longer_than_the_screen_must_be_scrolled_before_y() {
    let fx = Fixture::new();
    // 11 body lines leave 2 for the 6 list lines (3 items, each with 1 folder), after the title,
    // the lines below the list and the simctl warning.
    let short = on_clean(&fx).with_body_rows(11);
    let (review, _) = press(&short, &[Key::Char('a'), Key::Enter]);

    let (refused, refused_effects) = press(&review, &[Key::Char('y')]);
    let (scrolled, _) = press(&refused, &[Key::Down, Key::Down, Key::Down, Key::Down]);
    let (cleaning, effects) = press(&scrolled, &[Key::Char('y')]);

    assert_eq!((refused.screen, refused_effects), (Screen::Review, vec![]));
    assert!(refused.review.as_ref().unwrap().note.is_some());
    assert!(scrolled.review.as_ref().unwrap().seen_all());
    assert_eq!(cleaning.screen, Screen::Cleaning);
    assert!(matches!(&effects[..], [Effect::Clean(items)] if items.len() == 3));
}

#[test]
fn space_pages_through_a_long_review() {
    let fx = Fixture::new();
    let short = on_clean(&fx).with_body_rows(11);
    let (review, _) = press(&short, &[Key::Char('a'), Key::Enter]);

    let (paged, _) = press(&review, &[Key::Char(' '), Key::Char(' ')]);

    let paged = paged.review.unwrap();
    assert_eq!(paged.scroll, 4);
    assert!(paged.seen_all());
}

#[test]
fn a_taller_screen_counts_the_newly_shown_lines_as_seen() {
    let fx = Fixture::new();
    let short = on_clean(&fx).with_body_rows(11);
    let (review, _) = press(&short, &[Key::Char('a'), Key::Enter]);

    let tall = review.clone().with_body_rows(40);

    assert!(!review.review.unwrap().seen_all());
    assert!(tall.review.unwrap().seen_all());
}

#[test]
fn risky_items_need_the_word_typed() {
    let fx = Fixture::new();
    // Down to Archives (risky), tick it.
    let (review, _) = press(&on_clean(&fx), &[Key::Down, Key::Char(' '), Key::Enter]);

    let (after_y, y_effects) = press(&review, &[Key::Char('y'), Key::Enter]);
    let (typo, typo_effects) = press(&review, &[word("delet"), vec![Key::Enter]].concat());
    let (fixed, effects) = press(
        &review,
        &[
            word("deletx"),
            vec![Key::Backspace, Key::Char('e'), Key::Enter],
        ]
        .concat(),
    );

    assert!(review.review.as_ref().unwrap().needs_word());
    assert_eq!((after_y.screen, y_effects), (Screen::Review, vec![]));
    assert_eq!((typo.screen, typo_effects), (Screen::Review, vec![]));
    assert_eq!(typo.review.unwrap().typed, "", "a wrong word is cleared");
    assert_eq!(
        fixed.screen,
        Screen::Cleaning,
        "{:?}",
        fixed.review.map(|r| r.typed)
    );
    assert!(matches!(&effects[..], [Effect::Clean(items)] if items.len() == 1));
    assert_eq!(CONFIRM_WORD, "delete");
}

#[test]
fn esc_on_review_goes_back_with_the_ticks() {
    let fx = Fixture::new();
    let (review, _) = press(&on_clean(&fx), &[Key::Char('a'), Key::Enter]);

    let (back, effects) = press(&review, &[Key::Esc]);

    assert_eq!((back.screen, effects), (Screen::Clean, vec![]));
    assert_eq!(ticked_names(&back).len(), 3);
}

#[test]
fn review_warns_about_running_tools_that_use_the_picked_caches() {
    let fx = Fixture::new();
    let clean = on_clean(&fx)
        .on_event(Event::Tools(Ok(vec![
            crate::system::Tool::Xcode,
            crate::system::Tool::Emulator,
        ])))
        .0;

    let (review, _) = press(&clean, &[Key::Char('a'), Key::Enter]);

    let warnings = review.review.unwrap().warnings;
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("Xcode is running and uses DerivedData")),
        "{warnings:?}"
    );
    assert!(
        !warnings.iter().any(|w| w.contains("emulator")),
        "no emulator item is picked"
    );
}

#[test]
fn review_says_so_when_running_apps_could_not_be_checked() {
    let fx = Fixture::new();
    let clean = on_clean(&fx)
        .on_event(Event::Tools(Err("ps not found".to_string())))
        .0;

    let (review, _) = press(&clean, &[Key::Char('a'), Key::Enter]);

    let warnings = review.review.unwrap().warnings;
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("could not be checked (ps not found)")),
        "{warnings:?}"
    );
}

#[test]
fn a_refused_clean_returns_to_review_with_the_reason() {
    let fx = Fixture::new();
    let (cleaning, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );

    let (app, _) = cleaning.on_event(Event::CleanRefused("keep list broken".to_string()));

    assert_eq!(app.screen, Screen::Review);
    assert!(
        app.message
            .unwrap()
            .contains("Nothing was removed: keep list broken")
    );
}

#[test]
fn cleaning_progress_leads_to_the_summary_and_then_a_fresh_scan() {
    let fx = Fixture::new();
    let (cleaning, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );
    let done = Cleaned {
        item: item("gradle.caches", "modules-2", 5, 1),
        outcome: Outcome::Removed(5),
    };

    let (progress, _) = cleaning.on_event(Event::Cleaned(done));
    let (summary, _) = progress.on_event(Event::CleanFinished(Some(42_000 * MB)));
    let (menu, _) = summary.on_key(Key::Enter);
    let (clean_again, effects) = menu.on_key(Key::Char('1'));

    assert_eq!(progress.cleaning.as_ref().unwrap().done.len(), 1);
    assert_eq!(summary.screen, Screen::Summary);
    assert_eq!(
        summary.cleaning.as_ref().unwrap().free_after,
        Some(42_000 * MB)
    );
    assert_eq!(menu.screen, Screen::Menu);
    assert_eq!(effects, [Effect::Scan(1)]);
    assert_eq!(ticked_names(&clean_again), Vec::<String>::new());
}

#[test]
fn a_kept_item_is_not_offered_again_by_the_rescan_after_a_clean() {
    let fx = Fixture::new();
    let (cleaning, _) = press(
        &on_clean(&fx),
        &[Key::Char('k'), Key::Char('a'), Key::Enter, Key::Char('y')],
    );

    let (summary, _) = cleaning.on_event(Event::CleanFinished(None));
    let (menu, _) = summary.on_key(Key::Enter);
    let (scanning, effects) = menu.on_key(Key::Char('1'));
    let (rescanned, _) = scanning.on_event(Event::Scanned(
        scanning.scan_generation,
        crate::tui::testkit::report(),
    ));
    let (picked, _) = press(&rescanned, &[Key::Char('a')]);

    assert_eq!(effects, [Effect::Scan(1)]);
    assert!(rescanned.clean_list.group(0, 0).all_kept());
    // DerivedData is kept; Archives are risky, Pixel_8 risky, Kotlin/Native caution.
    assert_eq!(ticked_names(&picked), ["modules-2"]);
}

#[test]
fn keys_do_nothing_while_cleaning() {
    let fx = Fixture::new();
    let (cleaning, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );

    let (still, effects) = press(&cleaning, &[Key::Esc, Key::Char('q'), Key::Enter]);

    assert_eq!((still.screen, effects), (Screen::Cleaning, vec![]));
}
