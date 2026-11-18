//! Centred help overlay listing every key binding from `docs/SPEC.md`.

use crate::theme::Palette;
use crate::ui::layout::centered_rect;
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Cell, Clear, Row, Table};

pub const HELP_BINDINGS: &[(&str, &str)] = &[
    ("q, Ctrl+C", "Quit"),
    ("?, F1", "Toggle help overlay"),
    ("1, 2, 3", "Overview, processes, diagnostics"),
    ("Tab, Shift+Tab", "Next, previous screen"),
    ("Space", "Pause or resume sampling"),
    (
        "+, -",
        "Lengthen or shorten sample interval (250 ms to 10 s)",
    ),
    ("t", "Cycle built-in themes for this session"),
    ("s", "Cycle process sort key: CPU, memory, PID, name"),
    ("r", "Reverse the sort order"),
    ("/", "Filter processes by name (Enter keeps, Esc clears)"),
    ("j, k, arrows", "Move the process selection"),
    (
        "PgUp, PgDn, g, G",
        "Page, top and bottom of the process list",
    ),
    ("a", "Acknowledge current alerts to calm the banner"),
];

/// Renders the centred help overlay on top of the active screen.
pub fn draw_help(frame: &mut Frame, area: Rect, palette: &Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let popup = centered_rect(area, 74, 16);
    frame.render_widget(Clear, popup);

    let block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(palette.accent))
        .style(Style::default().bg(palette.surface).fg(palette.text))
        .title(Span::styled(
            " HELP - KEY BINDINGS (any key closes) ",
            Style::default()
                .fg(palette.heading)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let key_style = Style::default()
        .fg(palette.accent)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(palette.text);

    let rows: Vec<Row<'static>> = HELP_BINDINGS
        .iter()
        .map(|(k, desc)| {
            Row::new(vec![
                Cell::from(Span::styled(*k, key_style)),
                Cell::from(Span::styled(*desc, desc_style)),
            ])
        })
        .collect();

    let widths = [Constraint::Length(18), Constraint::Min(20)];
    let table = Table::new(rows, widths).column_spacing(2);
    frame.render_widget(table, inner);
}
