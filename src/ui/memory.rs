//! MEMORY panel: RAM and swap usage bars with binary byte units.

use crate::alert::{Level, RuleId};
use crate::app::App;
use crate::model::Reading;
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Sparkline};

/// Draws the MEMORY panel.
pub fn draw_memory(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "MEMORY", p);
        return;
    };
    let mem = match &snap.memory {
        Reading::Available(m) => m,
        Reading::Unavailable(r) => {
            draw_unavailable(frame, area, "MEMORY", r, p);
            return;
        }
    };

    let block = panel_block("MEMORY", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let ram_level = app
        .rules
        .iter()
        .find(|r| r.rule == RuleId::Memory)
        .map(|r| r.level)
        .unwrap_or(Level::Nominal);
    let ram_color = match ram_level {
        Level::Caution | Level::Critical => fmt::level_color(p, ram_level),
        _ => p.nominal,
    };

    let swap_level = app
        .rules
        .iter()
        .find(|r| r.rule == RuleId::Swap)
        .map(|r| r.level)
        .unwrap_or(Level::Nominal);
    let swap_color = match swap_level {
        Level::Caution | Level::Critical => fmt::level_color(p, swap_level),
        _ => p.nominal,
    };

    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let bold_text = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
    let border_style = Style::default().fg(p.border);

    let bar_w = if inner.width >= 40 { 8 } else { 4 };
    let used = mem.used_bytes();
    let ram_pct = mem.used_percent().unwrap_or(0.0);
    let (ram_filled, ram_empty) = fmt::bar(bar_w, ram_pct);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    let ram_line = Line::from(vec![
        Span::styled("RAM  ", bold_text),
        Span::styled(ram_filled, Style::default().fg(ram_color)),
        Span::styled(ram_empty, border_style),
        Span::styled(
            format!(" {}  ", fmt::bytes_pair(used, mem.total_bytes)),
            text_style,
        ),
        Span::styled(fmt::percent(ram_pct), bold_text),
    ]);
    frame.render_widget(Paragraph::new(ram_line), rows[0]);

    if rows[1].height > 0 {
        if mem.swap_total_bytes == 0 {
            let line = Line::from(vec![
                Span::styled("SWAP ", bold_text),
                Span::styled("none configured", muted),
            ]);
            frame.render_widget(Paragraph::new(line), rows[1]);
        } else {
            let swap_pct = mem.swap_used_percent().unwrap_or(0.0);
            let (swap_filled, swap_empty) = fmt::bar(bar_w, swap_pct);
            let swap_line = Line::from(vec![
                Span::styled("SWAP ", bold_text),
                Span::styled(swap_filled, Style::default().fg(swap_color)),
                Span::styled(swap_empty, border_style),
                Span::styled(
                    format!(
                        " {}  ",
                        fmt::bytes_pair(mem.swap_used_bytes, mem.swap_total_bytes)
                    ),
                    text_style,
                ),
                Span::styled(fmt::percent(swap_pct), bold_text),
            ]);
            frame.render_widget(Paragraph::new(swap_line), rows[1]);
        }
    }

    if rows[2].height > 0 {
        let extra = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(rows[2]);
        let avail_line = Line::from(vec![
            Span::styled("AVAIL ", muted),
            Span::styled(fmt::bytes(mem.available_bytes), text_style),
        ]);
        frame.render_widget(Paragraph::new(avail_line), extra[0]);

        if extra[1].height > 0 && area.height >= 6 {
            let data: Vec<u64> = app
                .history
                .memory_percent
                .tail(extra[1].width as usize)
                .map(|v| v.round().clamp(0.0, 100.0) as u64)
                .collect();
            let spark = Sparkline::default()
                .data(&data)
                .max(100)
                .style(Style::default().fg(ram_color).bg(p.background));
            frame.render_widget(spark, extra[1]);
        }
    }
}
