//! Top header bar, bottom key-hint footer and shared panel block builders.

use crate::app::{App, InputMode, Screen};
use crate::model::Reading;
use crate::theme::Palette;
use crate::ui::fmt;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

/// Builds a standard bordered panel block with its title in `palette.heading` and border in `palette.border`.
pub fn panel_block<'a>(title: &'a str, palette: &Palette) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(palette.border))
        .style(Style::default().bg(palette.background).fg(palette.text))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(palette.heading)
                .add_modifier(Modifier::BOLD),
        ))
}

/// Renders a panel showing `"acquiring"` in `palette.muted` when `app.snapshot` is `None`.
pub fn draw_acquiring(frame: &mut Frame, area: Rect, title: &str, palette: &Palette) {
    let block = panel_block(title, palette);
    let p = Paragraph::new(Line::from(Span::styled(
        "acquiring",
        Style::default().fg(palette.muted),
    )))
    .block(block);
    frame.render_widget(p, area);
}

/// Renders a panel showing `unavailable` (and reason when room permits) in `palette.unknown`.
pub fn draw_unavailable(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    reason: &str,
    palette: &Palette,
) {
    let block = panel_block(title, palette);
    let inner_w = area.width.saturating_sub(2) as usize;
    let text = fmt::unavailable_str(reason, inner_w);
    let p = Paragraph::new(Line::from(Span::styled(
        text,
        Style::default().fg(palette.unknown),
    )))
    .block(block);
    frame.render_widget(p, area);
}

/// Draws the one-line top header bar.
pub fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let base_style = Style::default().bg(p.background).fg(p.text);
    let muted = Style::default().bg(p.background).fg(p.muted);
    let unknown = Style::default().bg(p.background).fg(p.unknown);
    let accent = Style::default()
        .bg(p.background)
        .fg(p.accent)
        .add_modifier(Modifier::BOLD);

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled("NERVTOP ", accent));

    if let Some(snap) = &app.snapshot {
        let reason_room = if area.width >= 120 { 28 } else { 11 };
        match &snap.host.hostname {
            Reading::Available(h) => spans.push(Span::styled(h.clone(), base_style)),
            Reading::Unavailable(r) => {
                spans.push(Span::styled(fmt::unavailable_str(r, reason_room), unknown));
            }
        }
        spans.push(Span::styled(" ", muted));
        match &snap.host.kernel {
            Reading::Available(k) => spans.push(Span::styled(k.clone(), muted)),
            Reading::Unavailable(r) => {
                spans.push(Span::styled(fmt::unavailable_str(r, reason_room), unknown));
            }
        }
        spans.push(Span::styled(" ", muted));
        match &snap.host.uptime_secs {
            Reading::Available(u) => {
                spans.push(Span::styled(format!("up {}", fmt::duration(*u)), muted));
            }
            Reading::Unavailable(r) => {
                spans.push(Span::styled(fmt::unavailable_str(r, reason_room), unknown));
            }
        }
    } else {
        spans.push(Span::styled("acquiring", muted));
    }

    spans.push(Span::styled(" ", muted));
    for scr in Screen::ALL {
        let label = format!("{}:{}", scr.key(), scr.title());
        if scr == app.screen {
            spans.push(Span::styled(format!("[{label}]"), accent));
        } else {
            spans.push(Span::styled(format!(" {label} "), muted));
        }
    }

    spans.push(Span::styled(format!(" {}ms", app.interval_ms), muted));
    if area.width >= 100 {
        spans.push(Span::styled(format!(" {}", app.theme.as_str()), muted));
    }
    if app.paused {
        spans.push(Span::styled(
            " PAUSED",
            Style::default()
                .bg(p.background)
                .fg(p.caution)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let paragraph = Paragraph::new(Line::from(spans)).style(base_style);
    frame.render_widget(paragraph, area);
}

/// Draws the one-line bottom key-hint footer bar.
pub fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let base_style = Style::default().bg(p.surface).fg(p.muted);
    let key_style = Style::default()
        .bg(p.surface)
        .fg(p.accent)
        .add_modifier(Modifier::BOLD);
    let text_style = Style::default().bg(p.surface).fg(p.text);

    if app.procs.mode == InputMode::Filter {
        let line = Line::from(vec![
            Span::styled(" FILTER /", key_style),
            Span::styled(format!("{}█", app.procs.filter), text_style),
            Span::styled("  Enter keep  Esc clear", base_style),
        ]);
        frame.render_widget(Paragraph::new(line).style(base_style), area);
        return;
    }

    let pairs: &[(&str, &str)] = if area.width < 100 {
        &[
            ("1-3/Tab", "scr "),
            ("s/r", "sort "),
            ("/", "flt "),
            ("j/k", "sel "),
            ("Spc", "pause "),
            ("+/-", "rate "),
            ("t", "thm "),
            ("a", "ack "),
            ("?", "help "),
            ("q", "quit"),
        ]
    } else {
        &[
            ("1/2/3/Tab", "screen  "),
            ("s/r", "sort/rev  "),
            ("/", "filter  "),
            ("j/k", "move  "),
            ("Space", "pause  "),
            ("+/-", "interval  "),
            ("t", "theme  "),
            ("a", "ack  "),
            ("?/F1", "help  "),
            ("q", "quit"),
        ]
    };

    let mut spans = Vec::with_capacity(pairs.len() * 2 + 1);
    spans.push(Span::styled(" ", base_style));
    for (k, desc) in pairs {
        spans.push(Span::styled(*k, key_style));
        spans.push(Span::styled(format!(" {desc}"), base_style));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)).style(base_style), area);
}
