//! CPU panel: total load, load averages, frequency, brand, per-core bars and history sparkline.

use crate::app::App;
use crate::model::Reading;
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Sparkline};

const CORE_BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Maps a core load percentage (`0.0..=100.0`) to `' '` for `0` or one of `▁▂▃▄▅▆▇█`.
pub fn core_glyph(pct: f32) -> char {
    if !pct.is_finite() || pct <= 0.0 {
        return ' ';
    }
    let clamped = pct.clamp(0.0, 100.0);
    let idx = (((clamped / 100.0) * 8.0).ceil() as usize)
        .saturating_sub(1)
        .min(7);
    CORE_BLOCKS[idx]
}

fn fit_words(s: &str, max_w: usize) -> String {
    if s.chars().count() <= max_w {
        return s.to_string();
    }
    let mut out = String::new();
    for word in s.split_whitespace() {
        let next_len = if out.is_empty() {
            word.chars().count()
        } else {
            out.chars().count() + 1 + word.chars().count()
        };
        if next_len > max_w {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

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

    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let bold_text = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
    let border_style = Style::default().fg(p.border);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Summary row: total %, load averages, frequency, brand
    let w = inner.width as usize;
    let sep = if w >= 44 { "  " } else { " " };
    let tot_s = fmt::percent(cpu.total as f64);
    let load_s = match &cpu.load_avg {
        Reading::Available(l) => format!("{:.2} {:.2} {:.2}", l[0], l[1], l[2]),
        Reading::Unavailable(_) => "unavailable".to_owned(),
    };
    let mut summary_len = 4 + tot_s.len() + sep.len() + 5 + load_s.len();
    let mut summary = vec![
        Span::styled("TOT ", muted),
        Span::styled(tot_s, bold_text),
        Span::styled(format!("{sep}LOAD "), muted),
        Span::styled(load_s, text_style),
    ];
    if let Some(mhz) = cpu.freq_mhz
        && mhz > 0
    {
        let freq_s = fmt::freq_mhz(mhz);
        if summary_len + sep.len() + freq_s.len() <= w {
            summary_len += sep.len() + freq_s.len();
            summary.push(Span::styled(sep, muted));
            summary.push(Span::styled(freq_s, text_style));
        }
    }
    let mut brand_shown_inline = false;
    if let Some(brand) = &cpu.brand
        && !brand.is_empty()
    {
        let brand_fit = fit_words(brand, w.saturating_sub(summary_len + sep.len()));
        if !brand_fit.is_empty() && w >= 52 {
            summary.push(Span::styled(sep, muted));
            summary.push(Span::styled(brand_fit, muted));
            brand_shown_inline = true;
        }
    }
    lines.push(Line::from(summary));

    if let Some(brand) = &cpu.brand
        && !brand.is_empty()
        && !brand_shown_inline
        && inner.height >= 3
    {
        let brand_fit = fit_words(brand, w);
        if !brand_fit.is_empty() {
            lines.push(Line::from(Span::styled(brand_fit, muted)));
        }
    }

    // Per-core bars (full mode when all core groups fit, compact one-line strip otherwise)
    let core_rows_avail = (inner.height as usize).saturating_sub(lines.len());
    if core_rows_avail > 0 && !cpu.per_core.is_empty() {
        let w = inner.width as usize;
        let cols_per_row = if w >= 72 {
            4
        } else if w >= 36 {
            2
        } else {
            1
        };
        let rows_needed = cpu.per_core.len().div_ceil(cols_per_row);

        if core_rows_avail >= rows_needed {
            let col_w = (w / cols_per_row).max(1);
            // Each column shows: "C00 " (4) + bar + " 100%" (5) + " " (1) = 10 + bar_w
            let bar_w = col_w.saturating_sub(10).clamp(2, 16);

            for (row_idx, cores) in cpu.per_core.chunks(cols_per_row).enumerate() {
                let mut row_spans = Vec::with_capacity(cores.len() * 5);
                for (c_offset, &pct) in cores.iter().enumerate() {
                    let core_id = row_idx * cols_per_row + c_offset;
                    let bar_color = fmt::cpu_bar_color(p, pct);
                    let (filled, empty) = fmt::bar(bar_w, pct as f64);
                    row_spans.push(Span::styled(format!("C{core_id:02} "), muted));
                    row_spans.push(Span::styled(filled, Style::default().fg(bar_color)));
                    row_spans.push(Span::styled(empty, border_style));
                    row_spans.push(Span::styled(format!(" {pct:3.0}% "), text_style));
                }
                lines.push(Line::from(row_spans));
            }
        } else {
            // One glyph per core when they fit; otherwise each glyph shows the
            // busiest core of a group, so the strip and the max stay visible.
            let budget = (inner.width as usize).saturating_sub(6 + 15).max(1);
            let group = cpu.per_core.len().div_ceil(budget).max(1);
            let mut strip = Vec::with_capacity(budget + 2);
            strip.push(Span::styled("CORES ", muted));
            for chunk in cpu.per_core.chunks(group) {
                let pct = chunk.iter().copied().fold(0.0_f32, f32::max);
                let ch = core_glyph(pct);
                let color = fmt::cpu_bar_color(p, pct);
                strip.push(Span::styled(ch.to_string(), Style::default().fg(color)));
            }
            if let Some((max_id, &max_pct)) = cpu
                .per_core
                .iter()
                .enumerate()
                .max_by(|(ia, a), (ib, b)| a.total_cmp(b).then_with(|| ib.cmp(ia)))
            {
                strip.push(Span::styled(
                    format!(" max C{max_id:02} {max_pct:.0}%"),
                    text_style,
                ));
            }
            lines.push(Line::from(strip));
        }
    }

    let used_rows = (lines.len() as u16).min(inner.height);
    let rem_rows = inner.height.saturating_sub(used_rows);
    if rem_rows >= 2 {
        lines.push(Line::from(Span::styled("HISTORY", muted)));
    }

    let text_h = (lines.len() as u16).min(inner.height);
    let spark_h = inner.height.saturating_sub(text_h);
    let text_area = Rect::new(inner.x, inner.y, inner.width, text_h);
    frame.render_widget(Paragraph::new(lines), text_area);

    if spark_h > 0 && inner.width > 0 {
        let spark_area = Rect::new(inner.x, inner.y + text_h, inner.width, spark_h);
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
