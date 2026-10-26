//! Review, Cleaning, Summary, History, Keep list and Purge.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{ACCENT, first_visible, mode_badge, safety_span};
use crate::catalog::{self, Category};
use crate::clean::Outcome;
use crate::format::{human_size, items, printable};
use crate::mode::Mode;
use crate::oplog::Stage;
use crate::tui::app::{App, Loading};
use crate::tui::keep_screen::{KeepLine, lines as keep_lines};
use crate::tui::review::{CONFIRM_WORD, Review};

const RULE_WIDTH: usize = 70;
/// Lines above the keep list: the counts and a blank line.
const KEEP_TOP_LINES: usize = 2;

pub(super) fn review(frame: &mut Frame, area: Rect, app: &App) {
    let Some(review) = &app.review else { return };
    let list = review_list(review);
    let shown = review.list_rows().min(list.len());
    let more = if review.seen_all() {
        String::new()
    } else {
        "  ↓ for more".to_string()
    };
    let position = if list.len() > shown {
        format!(
            "  (lines {}–{} of {}){more}",
            review.scroll + 1,
            review.scroll + shown,
            list.len()
        )
    } else {
        String::new()
    };
    let mut lines = vec![
        Line::from(format!(" These will be removed:{position}")).bold(),
        Line::raw(""),
    ];
    lines.extend(list.into_iter().skip(review.scroll).take(shown));
    lines.push(Line::raw(""));
    lines.push(
        Line::from(format!(
            " Total: {} in {}",
            human_size(review.total_bytes()),
            items(review.items.len())
        ))
        .bold(),
    );
    lines.push(Line::from(vec![Span::raw(" "), mode_badge(app.mode)]));
    for warning in &review.warnings {
        lines.push(Line::styled(
            format!(" ⚠ {warning}"),
            Style::new().fg(Color::Yellow),
        ));
    }
    lines.push(Line::raw(""));
    let prompt = if review.needs_word() {
        format!(
            " Risky items are ticked. Type {CONFIRM_WORD} + enter to remove, esc to go back: {}",
            review.typed
        )
    } else {
        " Remove these? y yes · n / esc go back".to_string()
    };
    lines.push(Line::styled(
        prompt,
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    ));
    if let Some(note) = &review.note {
        lines.push(Line::styled(
            format!(" {note}"),
            Style::new().fg(Color::Yellow),
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// One line per item, then one per folder it removes; `Review::list_len` counts the same.
fn review_list(review: &Review) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(review.list_len());
    for (item, safety) in &review.items {
        lines.push(Line::from(vec![
            Span::raw("  "),
            safety_span(*safety),
            Span::raw(format!("  {:>9}  ", human_size(item.size_bytes))),
            Span::raw(printable(&item.name)).bold(),
        ]));
        for folder in item.removal.folders() {
            lines.push(
                Line::from(format!(
                    "               {}",
                    printable(&folder.display().to_string())
                ))
                .dim(),
            );
        }
    }
    lines
}

pub(super) fn cleaning(frame: &mut Frame, area: Rect, app: &App) {
    let Some(cleaning) = &app.cleaning else {
        return;
    };
    let done = cleaning.done.len();
    let bar_width = RULE_WIDTH - 10;
    let filled = (done * bar_width)
        .checked_div(cleaning.total)
        .unwrap_or(bar_width);
    let mut lines = vec![
        Line::from(format!(" Cleaning {done} of {}", cleaning.total)).bold(),
        Line::from(format!(
            " [{}{}]",
            "█".repeat(filled),
            "·".repeat(bar_width - filled)
        ))
        .fg(ACCENT),
        Line::raw(""),
    ];
    for item in cleaning
        .done
        .iter()
        .rev()
        .take(usize::from(area.height.saturating_sub(4)))
    {
        lines.push(outcome_line(&item.item.name, &item.outcome, app.mode));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn outcome_line(name: &str, outcome: &Outcome, mode: Mode) -> Line<'static> {
    let name = printable(name);
    match outcome {
        Outcome::Removed(bytes) => {
            Line::from(format!("   ✓ {name} · {}", human_size(*bytes))).fg(Color::Green)
        }
        Outcome::WouldRemove(bytes) => {
            Line::from(format!("   ✓ {name} · {} (dry run)", human_size(*bytes))).fg(Color::Green)
        }
        Outcome::AlreadyGone => Line::from(format!("   ◎ {name} · already gone")).dim(),
        Outcome::Kept => Line::from(format!("   ◎ {name} · skipped (kept)")).dim(),
        Outcome::Failed(reason) if mode == Mode::DryRun => {
            Line::from(format!("   ✗ {name} · would fail: {}", printable(reason))).fg(Color::Red)
        }
        Outcome::Failed(reason) => {
            Line::from(format!("   ✗ {name} · failed ({})", printable(reason))).fg(Color::Red)
        }
    }
}

pub(super) fn summary(frame: &mut Frame, area: Rect, app: &App) {
    let Some(cleaning) = &app.cleaning else {
        return;
    };
    let mut lines = Vec::new();
    for category in [Category::Xcode, Category::Android, Category::GradleKotlin] {
        let in_category: Vec<_> = cleaning
            .done
            .iter()
            .filter(|d| catalog::find(d.item.target).is_some_and(|t| t.category == category))
            .collect();
        if in_category.is_empty() {
            continue;
        }
        lines.push(
            Line::from(format!(" ➤ {}", category.title()))
                .fg(ACCENT)
                .bold(),
        );
        lines.extend(
            in_category
                .iter()
                .map(|d| outcome_line(&d.item.name, &d.outcome, app.mode)),
        );
    }
    let freed: u64 = cleaning.done.iter().map(|d| d.outcome.freed_bytes()).sum();
    let failed = cleaning
        .done
        .iter()
        .filter(|d| matches!(d.outcome, Outcome::Failed(_)))
        .count();
    let verb = match app.mode {
        Mode::DryRun => "Dry run — would free",
        Mode::Trash => "Cleanup complete — moved to Trash",
        Mode::Permanent => "Cleanup complete — removed",
    };
    let rule = "=".repeat(RULE_WIDTH);
    lines.push(Line::from(format!(" {rule}")));
    lines.push(
        Line::from(format!(
            " {verb}: {} | Items: {} | Failed: {failed}",
            human_size(freed),
            cleaning.done.len()
        ))
        .bold(),
    );
    if let Some(line) = free_space_line(app.mode, app.free_space, cleaning.free_after, freed) {
        lines.push(Line::from(line));
    }
    lines.push(Line::from(format!(" {rule}")));
    frame.render_widget(Paragraph::new(lines), area);
}

/// Moving to the Trash frees nothing until the Trash is emptied, so that is said instead of a gain.
fn free_space_line(
    mode: Mode,
    before: Option<u64>,
    after: Option<u64>,
    freed: u64,
) -> Option<String> {
    let after = after?;
    match mode {
        Mode::DryRun => None,
        Mode::Trash => Some(format!(
            " Free space: {} now, about {} after you empty the Trash",
            human_size(after),
            human_size(after.saturating_add(freed))
        )),
        Mode::Permanent => {
            let gained = after.saturating_sub(before?);
            Some(format!(
                " Free space: {} (+{})",
                human_size(after),
                human_size(gained)
            ))
        }
    }
}

pub(super) fn history(frame: &mut Frame, area: Rect, app: &App) {
    let text = match &app.history {
        Loading::Done(result) => match result.as_ref() {
            Ok(contents) => {
                let mut lines: Vec<Line> = contents
                    .entries
                    .iter()
                    .rev()
                    .filter(|e| e.stage == Stage::Finished)
                    .skip(app.history_scroll)
                    .map(|e| {
                        let result = e
                            .error
                            .as_ref()
                            .map_or_else(String::new, |err| format!("  ✗ {}", printable(err)));
                        Line::from(format!(
                            " {}  {:<9}  {:>9}  {}{result}",
                            e.time.format("%Y-%m-%d %H:%M"),
                            format!("{:?}", e.mode).to_lowercase(),
                            human_size(e.size_bytes),
                            printable(&e.path)
                        ))
                    })
                    .collect();
                if lines.is_empty() {
                    lines.push(Line::raw(" Nothing has been removed yet."));
                }
                if !contents.damaged_lines.is_empty() {
                    lines.push(Line::styled(
                        match contents.damaged_lines.len() {
                            1 => " 1 line of the log could not be read.".to_string(),
                            n => format!(" {n} lines of the log could not be read."),
                        },
                        Style::new().fg(Color::Yellow),
                    ));
                }
                lines
            }
            Err(reason) => vec![Line::styled(
                format!(" The log could not be read: {reason}"),
                Style::new().fg(Color::Red),
            )],
        },
        _ => vec![Line::raw(" Reading the log…")],
    };
    frame.render_widget(Paragraph::new(text), area);
}

pub(super) fn keep(frame: &mut Frame, area: Rect, app: &App) {
    let keep = match &app.keep {
        Ok(keep) => keep,
        Err(reason) => {
            let text = format!(
                " The keep list cannot be used: {reason}\n Fix or delete the file, then start lfmclean again."
            );
            frame.render_widget(
                Paragraph::new(text)
                    .fg(Color::Red)
                    .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }
    };
    let entries = keep_lines(keep);
    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new(
                " Nothing is kept. Press k on a cache or package in Clean or Packages to keep it.",
            ),
            area,
        );
        return;
    }
    let marked = app.keep_screen.marked.len();
    let mut lines = vec![
        Line::from(format!(
            " {} kept · {marked} marked to stop keeping",
            entries.len()
        ))
        .bold(),
        Line::raw(""),
    ];
    let visible = usize::from(area.height).saturating_sub(KEEP_TOP_LINES);
    let first = first_visible(app.keep_screen.cursor, visible);
    lines.extend(
        entries
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
            .map(|(i, entry)| keep_line(entry, i, app)),
    );
    frame.render_widget(Paragraph::new(lines), area);
}

fn keep_line(entry: &KeepLine, index: usize, app: &App) -> Line<'static> {
    let (mark, note) = if app.keep_screen.marked.contains(&index) {
        ("✗ ", "   (stop keeping)")
    } else {
        ("🔒", "")
    };
    let line = Line::from(format!(
        " {mark} {:<8} {}{note}",
        entry.kind,
        printable(&entry.text)
    ));
    if index == app.keep_screen.cursor {
        line.style(Style::new().add_modifier(Modifier::REVERSED))
    } else {
        line
    }
}

pub(super) fn purge(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new("\n Finding the build folders of your projects comes in a later version."),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_after_a_clean_depends_on_the_mode() {
        let gb = 1_000_000_000;

        assert_eq!(free_space_line(Mode::DryRun, Some(gb), Some(gb), gb), None);
        assert_eq!(
            free_space_line(Mode::Trash, Some(10 * gb), Some(10 * gb), 2 * gb).unwrap(),
            " Free space: 10.0 GB now, about 12.0 GB after you empty the Trash"
        );
        assert_eq!(
            free_space_line(Mode::Permanent, Some(10 * gb), Some(12 * gb), 2 * gb).unwrap(),
            " Free space: 12.0 GB (+2.0 GB)"
        );
        assert_eq!(free_space_line(Mode::Permanent, None, Some(gb), gb), None);
    }
}
