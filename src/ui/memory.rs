//! MEMORY panel: RAM and swap gauges with binary byte units.

use crate::alert::{Level, RuleId};
use crate::app::App;
use crate::model::Reading;
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph, Sparkline};

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

    let used = mem.used_bytes();
    let ram_pct = mem.used_percent().unwrap_or(0.0);
    let ram_ratio = if ram_pct.is_finite() {
        (ram_pct / 100.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let ram_label = format!(
        "RAM  {} ({})",
        fmt::bytes_pair(used, mem.total_bytes),
        fmt::percent(ram_pct)
    );

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    let ram_gauge = Gauge::default()
        .ratio(ram_ratio)
        .label(Span::styled(
            ram_label,
            Style::default().fg(p.text).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().fg(p.muted).bg(p.surface))
        .gauge_style(Style::default().fg(ram_color).bg(p.surface));
    frame.render_widget(ram_gauge, rows[0]);

    if rows[1].height > 0 {
        if mem.swap_total_bytes == 0 {
            let line = Line::from(vec![
                Span::styled("SWAP ", Style::default().fg(p.muted)),
                Span::styled("none configured", Style::default().fg(p.muted)),
            ]);
            frame.render_widget(Paragraph::new(line), rows[1]);
        } else {
            let swap_pct = mem.swap_used_percent().unwrap_or(0.0);
            let swap_ratio = if swap_pct.is_finite() {
                (swap_pct / 100.0).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let swap_label = format!(
                "SWAP {} ({})",
                fmt::bytes_pair(mem.swap_used_bytes, mem.swap_total_bytes),
                fmt::percent(swap_pct)
            );
            let swap_gauge = Gauge::default()
                .ratio(swap_ratio)
                .label(Span::styled(
                    swap_label,
                    Style::default().fg(p.text).add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().fg(p.muted).bg(p.surface))
                .gauge_style(Style::default().fg(swap_color).bg(p.surface));
            frame.render_widget(swap_gauge, rows[1]);
        }
    }

    if rows[2].height > 0 {
        let extra = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(rows[2]);
        let avail_line = Line::from(vec![
            Span::styled("AVAIL ", Style::default().fg(p.muted)),
            Span::styled(fmt::bytes(mem.available_bytes), Style::default().fg(p.text)),
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
