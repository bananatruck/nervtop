//! GPU panel: one block per adapter with utilisation, VRAM, temperature, power and suspended state.

use crate::app::App;
use crate::model::{GpuPower, GpuStats, Reading};
use crate::theme::Palette;
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Draws the GPU telemetry panel.
pub fn draw_gpu(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "GPU", p);
        return;
    };
    let gpus = match &snap.gpus {
        Reading::Available(list) => list,
        Reading::Unavailable(r) => {
            draw_unavailable(frame, area, "GPU", r, p);
            return;
        }
    };

    let block = panel_block("GPU", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if gpus.is_empty() {
        let p_empty = Paragraph::new(Line::from(Span::styled(
            "unavailable (no gpu adapters)",
            Style::default().fg(p.unknown),
        )));
        frame.render_widget(p_empty, inner);
        return;
    }

    let w = inner.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let multi_line = inner.height as usize >= gpus.len().saturating_mul(2);

    for gpu in gpus {
        if lines.len() >= inner.height as usize {
            break;
        }
        render_adapter_lines(gpu, p, w, multi_line, &mut lines);
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_adapter_lines(
    gpu: &GpuStats,
    p: &Palette,
    width: usize,
    multi_line: bool,
    lines: &mut Vec<Line<'static>>,
) {
    let name_style = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let unknown = Style::default().fg(p.unknown);

    if gpu.power_state == GpuPower::Suspended {
        lines.push(Line::from(vec![
            Span::styled(gpu.name.clone(), name_style),
            Span::styled("  ", muted),
            Span::styled(
                "SUSPENDED",
                Style::default().fg(p.muted).add_modifier(Modifier::BOLD),
            ),
        ]));
        return;
    }

    let reason_room = if width >= 54 { 26 } else { 11 };

    let util_spans: Vec<Span<'static>> = match &gpu.utilization_percent {
        Reading::Available(u) => {
            let bar_w = if width >= 50 { 8 } else { 4 };
            vec![
                Span::styled(fmt::bar(bar_w, *u as f64), Style::default().fg(p.nominal)),
                Span::styled(format!(" {}", fmt::percent(*u as f64)), text_style),
            ]
        }
        Reading::Unavailable(r) => {
            vec![Span::styled(fmt::unavailable_str(r, reason_room), unknown)]
        }
    };

    let vram_span = match (&gpu.memory_used_bytes, &gpu.memory_total_bytes) {
        (Reading::Available(used), Reading::Available(tot)) => {
            Span::styled(fmt::bytes_pair(*used, *tot), text_style)
        }
        (Reading::Unavailable(r), _) | (_, Reading::Unavailable(r)) => {
            Span::styled(fmt::unavailable_str(r, reason_room), unknown)
        }
    };

    let temp_span = match &gpu.temperature_celsius {
        Reading::Available(c) => Span::styled(format!("{c:.0}°C"), text_style),
        Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
    };

    let pwr_span = match &gpu.power_watts {
        Reading::Available(w) => Span::styled(format!("{w:.1}W"), text_style),
        Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
    };

    if multi_line {
        let mut row1 = vec![
            Span::styled(gpu.name.clone(), name_style),
            Span::styled("  ", muted),
        ];
        row1.extend(util_spans);
        lines.push(Line::from(row1));

        lines.push(Line::from(vec![
            Span::styled("VRAM ", muted),
            vram_span,
            Span::styled("  TEMP ", muted),
            temp_span,
            Span::styled("  PWR ", muted),
            pwr_span,
        ]));
    } else {
        let mut row = vec![
            Span::styled(gpu.name.clone(), name_style),
            Span::styled(" ", muted),
        ];
        row.extend(util_spans);
        row.push(Span::styled(" VRAM ", muted));
        row.push(vram_span);
        row.push(Span::styled(" ", muted));
        row.push(temp_span);
        row.push(Span::styled(" ", muted));
        row.push(pwr_span);
        lines.push(Line::from(row));
    }
}
