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
    let width = area.width as usize;
    let p = &app.palette;
    let base_style = Style::default().bg(p.background).fg(p.text);
    let muted = Style::default().bg(p.background).fg(p.muted);
    let unknown = Style::default().bg(p.background).fg(p.unknown);
    let accent = Style::default()
        .bg(p.background)
        .fg(p.accent)
        .add_modifier(Modifier::BOLD);
    let paused_style = Style::default()
        .bg(p.background)
        .fg(p.caution)
        .add_modifier(Modifier::BOLD);

    let clock_str = match (&app.snapshot, app.utc_offset_secs) {
        (Some(snap), Some(offset)) => Some(crate::clock::hms(snap.taken_at_ms, offset)),
        _ => None,
    };

    let candidates = [
        (true, true, true, true, false),
        (false, true, true, true, false),
        (false, false, true, true, false),
        (false, false, false, true, false),
        (false, false, false, true, true),
        (false, false, false, false, false),
        (false, false, false, false, true),
    ];

    let mut chosen_left: Vec<(String, Style)> = Vec::new();
    for (keep_kernel, keep_uptime, keep_theme, keep_host, short_tabs) in candidates {
        let parts = build_header_parts(
            app,
            keep_kernel,
            keep_uptime,
            keep_theme,
            keep_host,
            short_tabs,
            base_style,
            muted,
            unknown,
            accent,
            paused_style,
        );
        let left_len: usize = parts
            .iter()
            .map(|(s, _)| s.chars().count())
            .sum::<usize>()
            .saturating_add(parts.len().saturating_sub(1));
        let total_len = match &clock_str {
            Some(c) => left_len + 1 + c.chars().count(),
            None => left_len,
        };
        chosen_left = parts;
        if total_len <= width {
            break;
        }
    }

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(chosen_left.len() * 2 + 2);
    let mut left_len = 0usize;
    for (idx, (text, style)) in chosen_left.into_iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" ", muted));
            left_len += 1;
        }
        left_len += text.chars().count();
        spans.push(Span::styled(text, style));
    }

    if let Some(clock) = clock_str {
        let clock_len = clock.chars().count();
        let pad = width.saturating_sub(left_len + clock_len).max(1);
        spans.push(Span::styled(" ".repeat(pad), muted));
        spans.push(Span::styled(clock, base_style));
    }

    let paragraph = Paragraph::new(Line::from(spans)).style(base_style);
    frame.render_widget(paragraph, area);
}

fn whole_unavailable(reason: &str) -> String {
    let r = reason.trim();
    if r.is_empty() {
        "unavailable".to_string()
    } else {
        format!("unavailable ({r})")
    }
}

#[allow(clippy::too_many_arguments)]
fn build_header_parts(
    app: &App,
    keep_kernel: bool,
    keep_uptime: bool,
    keep_theme: bool,
    keep_host: bool,
    short_tabs: bool,
    base_style: Style,
    muted: Style,
    unknown: Style,
    accent: Style,
    paused_style: Style,
) -> Vec<(String, Style)> {
    let mut parts: Vec<(String, Style)> = Vec::new();
    parts.push(("NERVTOP".to_string(), accent));

    if let Some(snap) = &app.snapshot {
        if keep_host {
            match &snap.host.hostname {
                Reading::Available(h) => parts.push((h.clone(), base_style)),
                Reading::Unavailable(r) => parts.push((whole_unavailable(r), unknown)),
            }
        }
        if keep_kernel {
            match &snap.host.kernel {
                Reading::Available(k) => parts.push((k.clone(), muted)),
                Reading::Unavailable(r) => parts.push((whole_unavailable(r), unknown)),
            }
        }
        if keep_uptime {
            match &snap.host.uptime_secs {
                Reading::Available(u) => parts.push((format!("up {}", fmt::duration(*u)), muted)),
                Reading::Unavailable(r) => parts.push((whole_unavailable(r), unknown)),
            }
        }
    } else if keep_host {
        parts.push(("acquiring".to_string(), muted));
    }

    for scr in Screen::ALL {
        let label = if short_tabs {
            match scr {
                Screen::Overview => "1 OVR".to_string(),
                Screen::Processes => "2 PROC".to_string(),
                Screen::Diagnostics => "3 DIAG".to_string(),
            }
        } else {
            format!("{}:{}", scr.key(), scr.title())
        };
        if scr == app.screen {
            parts.push((format!("[{label}]"), accent));
        } else {
            parts.push((label, muted));
        }
    }

    parts.push((format!("{}ms", app.interval_ms), muted));
    if keep_theme {
        parts.push((app.theme.as_str().to_string(), muted));
    }
    if app.paused {
        parts.push(("PAUSED".to_string(), paused_style));
    }

    parts
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
