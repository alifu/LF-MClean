//! Drawing the screens. Drawing only reads the `App`.

mod lists;
mod pages;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::app::{App, Loading, MENU, Screen};
use crate::catalog::Safety;
use crate::format::human_size;
use crate::mode::Mode;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ACCENT: Color = Color::Cyan;

/// The first of `visible` rows to show so that `cursor` is on screen, at the bottom once the list
/// scrolls.
fn first_visible(cursor: usize, visible: usize) -> usize {
    cursor.saturating_sub(visible.saturating_sub(1))
}

/// Draws the whole screen.
pub fn draw(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    frame.render_widget(Paragraph::new(header_lines(app)), header);
    match app.screen {
        Screen::Menu => draw_menu(frame, body, app),
        Screen::Clean | Screen::Packages => lists::draw(frame, body, app),
        Screen::Purge => pages::purge(frame, body),
        Screen::Keep => pages::keep(frame, body, app),
        Screen::History => pages::history(frame, body, app),
        Screen::Review => pages::review(frame, body, app),
        Screen::Cleaning => pages::cleaning(frame, body, app),
        Screen::Summary => pages::summary(frame, body, app),
    }
    frame.render_widget(Paragraph::new(footer_lines(app)), footer);
}

fn header_lines(app: &App) -> Vec<Line<'static>> {
    let mut title = vec![
        Span::styled(
            " LF-MClean",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  v{VERSION}")),
    ];
    title.push(Span::raw("   "));
    title.push(mode_badge(app.mode));
    let chip = if cfg!(target_arch = "aarch64") {
        "Apple Silicon"
    } else {
        "Intel"
    };
    let free = app
        .free_space
        .map_or_else(|| "unknown".to_string(), human_size);
    let found = match &app.scan {
        Loading::Done(report) => human_size(report.total_bytes()),
        Loading::Running(_) => "scanning…".to_string(),
        Loading::Idle => "not scanned yet".to_string(),
    };
    let info = format!(" {chip} | Free space: {free} | Dev caches found: {found}");
    vec![Line::from(title), Line::from(info).dim()]
}

fn mode_badge(mode: Mode) -> Span<'static> {
    match mode {
        Mode::DryRun => Span::styled(
            "DRY RUN: nothing is removed",
            Style::new().fg(Color::Yellow),
        ),
        Mode::Trash => Span::styled("Moves to the Trash", Style::new().fg(Color::Green)),
        Mode::Permanent => Span::styled(
            "PERMANENT: removed for good",
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    }
}

fn footer_lines(app: &App) -> Vec<Line<'static>> {
    let keys = match app.screen {
        Screen::Menu => "↑↓ / 1-6 choose   enter open   q quit",
        Screen::Clean => {
            "↑↓ move  space tick  →← open/close  k keep  a all safe  n none  enter review  esc menu"
        }
        Screen::Packages => {
            "↑↓ move  space tick  →← open/close  k keep  o old versions  u unused  enter review  esc menu"
        }
        Screen::Keep => "↑↓ move   space mark to stop keeping   enter save   esc back",
        Screen::History | Screen::Review => "↑↓ scroll   esc back",
        Screen::Cleaning => "",
        Screen::Summary => "enter menu",
        Screen::Purge => "esc back",
    };
    let note = app.message.clone().unwrap_or_default();
    vec![
        Line::from(format!(" {keys}")).dim(),
        Line::from(format!(" {note}")).fg(Color::Yellow),
    ]
}

fn draw_menu(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines = vec![Line::raw("")];
    for (i, (_, name, about, command)) in MENU.iter().enumerate() {
        let chosen = i == app.menu_cursor;
        let marker = if chosen { " ➤ " } else { "   " };
        let text = format!("{marker}{}. {name:<12} {about:<52} {command}", i + 1);
        let style = if chosen {
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        lines.push(Line::styled(text, style));
    }
    if let Err(reason) = &app.keep {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!(" The keep list cannot be used, so nothing can be removed: {reason}"),
            Style::new().fg(Color::Red),
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// The mark and colour of a safety level, padded so what follows lines up.
fn safety_span(safety: Safety) -> Span<'static> {
    let (text, color) = match safety {
        Safety::Safe => ("● safe", Color::Green),
        Safety::Caution => ("◆ caution", Color::Yellow),
        Safety::Risky => ("▲ risky", Color::Red),
    };
    Span::styled(format!("{text:<9}"), Style::new().fg(color))
}

#[cfg(test)]
mod tests;
