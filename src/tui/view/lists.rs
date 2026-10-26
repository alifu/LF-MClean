//! The Clean and Packages lists.

use std::fmt::Write;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{ACCENT, first_visible, safety_span};
use crate::format::{human_size, items};
use crate::system::tools_using;
use crate::tui::app::{App, Loading, Screen};
use crate::tui::list::{ListState, Row, Tick};

const SIZE_WIDTH: usize = 9;
/// Lines above the list: the totals and a blank line.
const TOP_LINES: u16 = 2;

pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let (list, loading) = if app.screen == Screen::Packages {
        (&app.package_list, loading_text(&app.packages))
    } else {
        (&app.clean_list, loading_text(&app.scan))
    };
    if let Some(text) = loading {
        frame.render_widget(Paragraph::new(format!("\n {text}")), area);
        return;
    }
    if list.sections.is_empty() {
        frame.render_widget(Paragraph::new("\n Nothing found to clean here."), area);
        return;
    }
    let mut lines = vec![totals(list, app), Line::raw("")];
    let visible = usize::from(area.height.saturating_sub(TOP_LINES));
    let rows = list.rows();
    let first = first_visible(list.cursor, visible);
    let width = usize::from(area.width);
    for (index, row) in rows.iter().enumerate().skip(first).take(visible) {
        lines.push(row_line(list, *row, index == list.cursor, width));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn loading_text<T>(loading: &Loading<T>) -> Option<String> {
    match loading {
        Loading::Idle => Some("Not scanned yet.".to_string()),
        Loading::Running(now) if now.is_empty() => Some("Scanning…".to_string()),
        Loading::Running(now) => Some(format!("Scanning {now}…")),
        Loading::Done(_) => None,
    }
}

fn totals(list: &ListState, app: &App) -> Line<'static> {
    let picked: Vec<_> = list.picked();
    let mut text = format!(
        " Found {} · Selected {} ({})",
        human_size(list.total_bytes()),
        human_size(list.picked_bytes()),
        items(picked.len())
    );
    let mut busy: Vec<&str> = picked
        .iter()
        .flat_map(|(_, entry)| tools_using(&app.running, entry.item.target))
        .map(crate::system::Tool::name)
        .collect();
    busy.sort_unstable();
    busy.dedup();
    if !busy.is_empty() {
        let _ = write!(text, "   ⚠ in use by: {}", busy.join(", "));
    }
    if let Err(reason) = &app.keep {
        return Line::styled(
            format!(" Keep list broken, nothing can be removed: {reason}"),
            Style::new().fg(Color::Red),
        );
    }
    Line::from(text).bold()
}

fn row_line(list: &ListState, row: Row, at_cursor: bool, width: usize) -> Line<'static> {
    let line = match row {
        Row::Heading(s) => {
            let title = format!(" ─ {} ", list.sections[s].title);
            let rule = "─".repeat(width.saturating_sub(title.chars().count() + 1));
            return Line::styled(format!("{title}{rule}"), Style::new().fg(ACCENT));
        }
        Row::Group(s, g) => {
            let group = list.group(s, g);
            let mark = if group.all_kept() {
                "🔒 "
            } else {
                match list.tick_of(group) {
                    Tick::All => "[x]",
                    Tick::Some => "[~]",
                    Tick::None => "[ ]",
                }
            };
            let arrow = if list.open.contains(&group.key) {
                "▾"
            } else {
                "▸"
            };
            Line::from(vec![
                Span::raw(format!(
                    " {mark} {arrow} {:<40} {:>SIZE_WIDTH$}  ",
                    group.label,
                    human_size(group.total_bytes())
                )),
                safety_span(group.safety),
                Span::raw(format!("  {}", group.detail)).dim(),
            ])
        }
        Row::Entry(s, g, e) => {
            let entry = &list.group(s, g).entries[e];
            let mark = if entry.kept {
                "🔒 "
            } else if list.ticked.contains(&entry.item.key()) {
                "[x]"
            } else {
                "[ ]"
            };
            let size = format!(
                "{}{}",
                human_size(entry.item.size_bytes),
                if entry.item.partial { "+" } else { "" }
            );
            let kept = if entry.kept { "  kept" } else { "" };
            Line::from(vec![
                Span::raw(format!(
                    "     {mark} {:<38} {size:>SIZE_WIDTH$}  ",
                    entry.label
                )),
                Span::raw(format!("{}{kept}", entry.detail)).dim(),
            ])
        }
    };
    if at_cursor {
        line.style(Style::new().add_modifier(Modifier::REVERSED))
    } else {
        line
    }
}
