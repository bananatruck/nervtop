//! STORAGE panel: per-mount usage bar (coloured by matching disk alert rule), used/total and I/O rates.

use crate::alert::{Level, RuleId};
use crate::app::App;
use crate::model::{DiskStats, Reading};
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Draws the STORAGE telemetry panel.
pub fn draw_storage(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "STORAGE", p);
        return;
    };
    let disks = match &snap.disks {
        Reading::Available(list) => list,
        Reading::Unavailable(r) => {
            draw_unavailable(frame, area, "STORAGE", r, p);
            return;
        }
    };

    let block = panel_block("STORAGE", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if disks.is_empty() {
        let p_empty = Paragraph::new(Line::from(Span::styled(
            "unavailable (no disks mounted)",
            Style::default().fg(p.unknown),
        )));
        frame.render_widget(p_empty, inner);
        return;
    }

    let w = inner.width as usize;
    let two_line_per_disk = w < 64 && inner.height as usize >= disks.len().saturating_mul(2);
    let mut lines: Vec<Line<'static>> = Vec::new();

    for d in disks {
        if lines.len() >= inner.height as usize {
            break;
        }
        render_disk(d, app, w, two_line_per_disk, &mut lines);
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_disk(
    d: &DiskStats,
    app: &App,
    width: usize,
    two_line: bool,
    lines: &mut Vec<Line<'static>>,
) {
    let p = &app.palette;
    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let bold_text = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
    let unknown = Style::default().fg(p.unknown);

    let rule_level = app
        .rules
        .iter()
        .find(|r| r.rule == RuleId::Disk && r.subject == d.mount_point)
        .or_else(|| app.rules.iter().find(|r| r.rule == RuleId::Disk))
        .map(|r| r.level)
        .unwrap_or(Level::Nominal);

    let bar_color = match rule_level {
        Level::Unknown => p.nominal,
        lvl => fmt::level_color(p, lvl),
    };

    let pct = d.used_percent().unwrap_or(0.0);
    let bar_w = if width >= 64 { 8 } else { 4 };
    let reason_room = if width >= 60 { 24 } else { 11 };

    let r_span = match &d.read_bps {
        Reading::Available(bps) => Span::styled(fmt::rate(*bps), text_style),
        Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
    };
    let w_span = match &d.write_bps {
        Reading::Available(bps) => Span::styled(fmt::rate(*bps), text_style),
        Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
    };

    if two_line {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<8} ", d.mount_point), bold_text),
            Span::styled(fmt::bar(bar_w, pct), Style::default().fg(bar_color)),
            Span::styled(format!(" {:>5} ", fmt::percent(pct)), bold_text),
            Span::styled(fmt::bytes_pair(d.used_bytes(), d.total_bytes), text_style),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  R ", muted),
            r_span,
            Span::styled("  W ", muted),
            w_span,
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", d.mount_point), bold_text),
            Span::styled(fmt::bar(bar_w, pct), Style::default().fg(bar_color)),
            Span::styled(format!(" {} ", fmt::percent(pct)), bold_text),
            Span::styled(
                format!("{} ", fmt::bytes_pair(d.used_bytes(), d.total_bytes)),
                text_style,
            ),
            Span::styled("R ", muted),
            r_span,
            Span::styled(" W ", muted),
            w_span,
        ]));
    }
}
