//! POWER panel: battery percentage bar, status, AC state, power draw, time remaining, health and charge limit.

use crate::alert::{Level, RuleId};
use crate::app::App;
use crate::model::{BatteryStatus, Reading};
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Human-readable label for `BatteryStatus`.
pub fn status_label(status: BatteryStatus) -> &'static str {
    match status {
        BatteryStatus::Charging => "CHARGING",
        BatteryStatus::Discharging => "DISCHARGING",
        BatteryStatus::Full => "FULL",
        BatteryStatus::NotCharging => "NOT CHARGING",
        BatteryStatus::Unknown => "UNKNOWN",
    }
}

/// Draws the POWER telemetry panel.
pub fn draw_power(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "POWER", p);
        return;
    };
    let bat = match &snap.battery {
        Reading::Available(b) => b,
        Reading::Unavailable(r) => {
            draw_unavailable(frame, area, "POWER", r, p);
            return;
        }
    };

    let block = panel_block("POWER", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rule_level = app
        .rules
        .iter()
        .find(|r| r.rule == RuleId::Battery)
        .map(|r| r.level)
        .unwrap_or(Level::Nominal);
    let bar_color = match rule_level {
        Level::Caution | Level::Critical => fmt::level_color(p, rule_level),
        _ => p.nominal,
    };

    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let bold_text = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
    let unknown = Style::default().fg(p.unknown);

    let w = inner.width as usize;
    let bar_w = if w >= 54 { 8 } else { 4 };
    let reason_room = if w >= 60 { 48 } else { 11 };

    let (filled, empty) = fmt::bar(bar_w, bat.percent);
    let mut row1 = vec![
        Span::styled(format!("{} ", bat.name), bold_text),
        Span::styled(filled, Style::default().fg(bar_color)),
        Span::styled(empty, Style::default().fg(p.border)),
        Span::styled(format!(" {} ", fmt::percent(bat.percent)), bold_text),
        Span::styled(status_label(bat.status), text_style),
    ];
    if let Some(ac) = bat.ac_online {
        row1.push(Span::styled(if ac { "  AC:ON" } else { "  AC:OFF" }, muted));
    }

    let pwr_span = match &bat.power_watts {
        Reading::Available(w_val) => Span::styled(format!("{w_val:.1}W"), text_style),
        Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
    };
    let rem_span = match &bat.time_remaining_secs {
        Reading::Available(s) => Span::styled(fmt::duration(*s), text_style),
        Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
    };
    let hlth_span = match &bat.health_percent {
        Reading::Available(h) => Span::styled(fmt::percent(*h), text_style),
        Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
    };

    let mut lines: Vec<Line<'static>> = Vec::new();

    if inner.height == 1 {
        row1.push(Span::styled(" ", muted));
        row1.push(pwr_span);
        row1.push(Span::styled(" ", muted));
        row1.push(rem_span);
        row1.push(Span::styled(" ", muted));
        row1.push(hlth_span);
        lines.push(Line::from(row1));
    } else if inner.height == 2 {
        lines.push(Line::from(row1));
        let row2_base_len = 4
            + pwr_span.content.chars().count()
            + 5
            + rem_span.content.chars().count()
            + 6
            + hlth_span.content.chars().count();
        let mut row2 = vec![
            Span::styled("PWR ", muted),
            pwr_span,
            Span::styled(" REM ", muted),
            rem_span,
            Span::styled(" HLTH ", muted),
            hlth_span,
        ];
        if let Some(lim) = bat.charge_limit_percent {
            let lim_s = format!(" LIM {lim}%");
            if row2_base_len + lim_s.len() <= w {
                row2.push(Span::styled(lim_s, muted));
            }
        }
        lines.push(Line::from(row2));
    } else {
        lines.push(Line::from(row1));
        lines.push(Line::from(vec![
            Span::styled("PWR ", muted),
            pwr_span,
            Span::styled("  REM ", muted),
            rem_span,
        ]));
        let mut row3 = vec![Span::styled("HLTH ", muted), hlth_span];
        if let Some(lim) = bat.charge_limit_percent {
            row3.push(Span::styled(format!("  LIM {lim}%"), text_style));
        }
        if let Some(cyc) = bat.cycle_count {
            row3.push(Span::styled(format!("  CYC {cyc}"), muted));
        }
        lines.push(Line::from(row3));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}
