//! NETWORK panel: per-interface RX/TX rates and totals, plus RX/TX sparklines when space allows.

use crate::app::App;
use crate::model::{NetInterface, Reading};
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Sparkline};

/// Returns network interfaces ordered busiest first:
/// by current `rx+tx` rate descending (unavailable rates count as `0`),
/// then available lifetime `rx+tx` totals descending, then name ascending.
pub fn sorted_interfaces(ifaces: &[NetInterface]) -> Vec<&NetInterface> {
    fn clean_rate(r: &Reading<f64>) -> f64 {
        match r {
            Reading::Available(v) if v.is_finite() && *v > 0.0 => *v,
            _ => 0.0,
        }
    }
    let mut sorted: Vec<&NetInterface> = ifaces.iter().collect();
    sorted.sort_by(|a, b| {
        let rate_a = clean_rate(&a.rx_bps) + clean_rate(&a.tx_bps);
        let rate_b = clean_rate(&b.rx_bps) + clean_rate(&b.tx_bps);
        let total_a = u128::from(a.rx_total_bytes.value().copied().unwrap_or(0))
            + u128::from(a.tx_total_bytes.value().copied().unwrap_or(0));
        let total_b = u128::from(b.rx_total_bytes.value().copied().unwrap_or(0))
            + u128::from(b.tx_total_bytes.value().copied().unwrap_or(0));
        rate_b
            .total_cmp(&rate_a)
            .then_with(|| total_b.cmp(&total_a))
            .then_with(|| a.name.cmp(&b.name))
    });
    sorted
}

/// Draws the NETWORK telemetry panel.
pub fn draw_network(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "NETWORK", p);
        return;
    };
    let ifaces = match &snap.network {
        Reading::Available(list) => list,
        Reading::Unavailable(r) => {
            draw_unavailable(frame, area, "NETWORK", r, p);
            return;
        }
    };

    let block = panel_block("NETWORK", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if ifaces.is_empty() {
        let p_empty = Paragraph::new(Line::from(Span::styled(
            "unavailable (no active interfaces)",
            Style::default().fg(p.unknown),
        )));
        frame.render_widget(p_empty, inner);
        return;
    }

    let show_sparklines = area.height >= 6 && inner.height >= 3;
    let spark_h = u16::from(show_sparklines);
    let text_h = inner.height.saturating_sub(spark_h);
    let chunks =
        Layout::vertical([Constraint::Length(text_h), Constraint::Length(spark_h)]).split(inner);
    let text_area = chunks[0];
    let spark_area = chunks[1];

    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let bold_text = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
    let unknown = Style::default().fg(p.unknown);

    let w = text_area.width as usize;
    let reason_room = if w >= 60 { 22 } else { 11 };
    let mut lines: Vec<Line<'static>> = Vec::new();

    let sorted = sorted_interfaces(ifaces);
    for iface in sorted.into_iter().take(text_area.height as usize) {
        let rx_span = match &iface.rx_bps {
            Reading::Available(bps) => Span::styled(fmt::rate(*bps), text_style),
            Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
        };
        let tx_span = match &iface.tx_bps {
            Reading::Available(bps) => Span::styled(fmt::rate(*bps), text_style),
            Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
        };

        let mut spans = vec![
            Span::styled(format!("{} ", iface.name), bold_text),
            Span::styled("RX ", muted),
            rx_span,
            Span::styled(" TX ", muted),
            tx_span,
        ];
        if w >= 48 {
            spans.push(Span::styled(
                format!(
                    "  ({} / {})",
                    fmt::reading_bytes(&iface.rx_total_bytes, 11),
                    fmt::reading_bytes(&iface.tx_total_bytes, 11)
                ),
                muted,
            ));
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), text_area);

    if show_sparklines && spark_area.width >= 4 && spark_area.height > 0 {
        let halves = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(spark_area);
        let rx_w = halves[0].width as usize;
        let tx_w = halves[1].width as usize;
        let rx_data: Vec<u64> = app
            .history
            .net_rx_bps
            .tail(rx_w)
            .map(|v| v.max(0.0).round() as u64)
            .collect();
        let tx_data: Vec<u64> = app
            .history
            .net_tx_bps
            .tail(tx_w)
            .map(|v| v.max(0.0).round() as u64)
            .collect();

        let rx_max = rx_data.iter().copied().max().unwrap_or(1).max(1);
        let tx_max = tx_data.iter().copied().max().unwrap_or(1).max(1);

        frame.render_widget(
            Sparkline::default()
                .data(&rx_data)
                .max(rx_max)
                .style(Style::default().fg(p.nominal).bg(p.background)),
            halves[0],
        );
        frame.render_widget(
            Sparkline::default()
                .data(&tx_data)
                .max(tx_max)
                .style(Style::default().fg(p.accent).bg(p.background)),
            halves[1],
        );
    }
}
