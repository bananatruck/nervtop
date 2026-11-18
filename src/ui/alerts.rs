//! ALERTS panel: active non-nominal rules first, followed by recent alert log events.

use crate::alert::{Level, RuleStatus};
use crate::app::App;
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, panel_block};
use crate::ui::stripes::format_rule_value;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Draws the ALERTS panel on the OVERVIEW screen.
pub fn draw_alerts(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    if app.snapshot.is_none() && app.rules.is_empty() && app.log.is_empty() {
        draw_acquiring(frame, area, "ALERTS", p);
        return;
    }

    let block = panel_block("ALERTS", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let max_rows = inner.height as usize;
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(max_rows);
    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);

    let mut active: Vec<&RuleStatus> = app
        .rules
        .iter()
        .filter(|r| matches!(r.level, Level::Caution | Level::Critical))
        .collect();
    active.sort_by_key(|a| std::cmp::Reverse(a.level));

    if active.is_empty() {
        lines.push(Line::from(Span::styled(
            "NOMINAL - no active alerts",
            Style::default().fg(p.nominal),
        )));
    } else {
        for r in active {
            if lines.len() >= max_rows {
                break;
            }
            let c = fmt::level_color(p, r.level);
            let subj = if r.subject.is_empty() {
                String::new()
            } else {
                format!(" {}", r.subject)
            };
            let val_str = r
                .value
                .map(|v| format!(" {}", format_rule_value(r.rule, v)))
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", r.level.label()),
                    Style::default().fg(c).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{}{subj}", r.rule.label()), text_style),
                Span::styled(val_str, Style::default().fg(c).add_modifier(Modifier::BOLD)),
            ]));
        }
    }

    let remaining = max_rows.saturating_sub(lines.len());
    for ev in app.log.iter().rev().take(remaining) {
        let to_color = fmt::level_color(p, ev.to);
        let subj = if ev.subject.is_empty() {
            String::new()
        } else {
            format!(" {}", ev.subject)
        };
        let val_str = ev
            .value
            .map(|v| format!(" ({})", format_rule_value(ev.rule, v)))
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", fmt::hms_utc(ev.at_ms)), muted),
            Span::styled(format!("{}{subj} ", ev.rule.label()), text_style),
            Span::styled(
                format!("{}->{}", ev.from.label(), ev.to.label()),
                Style::default().fg(to_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(val_str, muted),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}
