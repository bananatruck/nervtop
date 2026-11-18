//! CPU panel: total load, load averages, frequency, brand, per-core bars and history sparkline.

use crate::app::App;
use crate::model::Reading;
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Sparkline};

/// Draws the CPU telemetry panel.
pub fn draw_cpu(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "CPU", p);
        return;
    };
    let cpu = match &snap.cpu {
        Reading::Available(c) => c,
        Reading::Unavailable(r) => {
            draw_unavailable(frame, area, "CPU", r, p);
            return;
        }
    };

    let block = panel_block("CPU", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let show_sparkline = area.height >= 6 && inner.height >= 3;
    let spark_h = if show_sparkline {
        if inner.height >= 8 { 2 } else { 1 }
    } else {
        0
    };
    let text_h = inner.height.saturating_sub(spark_h);
    let chunks =
        Layout::vertical([Constraint::Length(text_h), Constraint::Length(spark_h)]).split(inner);
    let text_area = chunks[0];
    let spark_area = chunks[1];

    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let bold_text = Style::default().fg(p.text).add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Summary row: total %, load averages, frequency, brand
    let mut summary = vec![
        Span::styled("TOT ", muted),
        Span::styled(fmt::percent(cpu.total as f64), bold_text),
        Span::styled("  LOAD ", muted),
        Span::styled(
            format!(
                "{:.2} {:.2} {:.2}",
                cpu.load_avg[0], cpu.load_avg[1], cpu.load_avg[2]
            ),
            text_style,
        ),
    ];
    if let Some(mhz) = cpu.freq_mhz
        && mhz > 0
    {
        summary.push(Span::styled("  ", muted));
        summary.push(Span::styled(fmt::freq_mhz(mhz), text_style));
    }
    if let Some(brand) = &cpu.brand
        && !brand.is_empty()
        && text_area.width >= 52
    {
        summary.push(Span::styled("  ", muted));
        summary.push(Span::styled(brand.clone(), muted));
    }
    lines.push(Line::from(summary));

    if let Some(brand) = &cpu.brand
        && !brand.is_empty()
        && text_area.width < 52
        && text_area.height >= 3
    {
        lines.push(Line::from(Span::styled(brand.clone(), muted)));
    }

    // Per-core bars
    let core_rows_avail = (text_area.height as usize).saturating_sub(lines.len());
    if core_rows_avail > 0 && !cpu.per_core.is_empty() {
        let w = text_area.width as usize;
        let cols_per_row = if w >= 72 {
            4
        } else if w >= 36 {
            2
        } else {
            1
        };
        let col_w = (w / cols_per_row).max(1);
        // Each column shows: "C00 " (4) + bar + " 100%" (5) + " " (1) = 10 + bar_w
        let bar_w = col_w.saturating_sub(10).clamp(2, 16);

        for chunk in cpu
            .per_core
            .chunks(cols_per_row)
            .take(core_rows_avail)
            .enumerate()
        {
            let (row_idx, cores) = chunk;
            let mut row_spans = Vec::with_capacity(cores.len() * 4);
            for (c_offset, &pct) in cores.iter().enumerate() {
                let core_id = row_idx * cols_per_row + c_offset;
                let bar_color = fmt::cpu_bar_color(p, pct);
                row_spans.push(Span::styled(format!("C{core_id:02} "), muted));
                row_spans.push(Span::styled(
                    fmt::bar(bar_w, pct as f64),
                    Style::default().fg(bar_color),
                ));
                row_spans.push(Span::styled(format!(" {pct:3.0}% "), text_style));
            }
            lines.push(Line::from(row_spans));
        }
    }

    frame.render_widget(Paragraph::new(lines), text_area);

    if show_sparkline && spark_area.width > 0 && spark_area.height > 0 {
        let data: Vec<u64> = app
            .history
            .cpu_total
            .tail(spark_area.width as usize)
            .map(|v| v.round().clamp(0.0, 100.0) as u64)
            .collect();
        let spark = Sparkline::default()
            .data(&data)
            .max(100)
            .style(Style::default().fg(p.nominal).bg(p.background));
        frame.render_widget(spark, spark_area);
    }
}
