//! THERMAL panel: hottest temperature sensors first (marked when within 10°C of crit) and fan RPMs.

use crate::app::App;
use crate::model::{Reading, TempSensor};
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Draws the THERMAL telemetry panel.
pub fn draw_thermal(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "THERMAL", p);
        return;
    };
    let sensors = match &snap.sensors {
        Reading::Available(s) => s,
        Reading::Unavailable(r) => {
            draw_unavailable(frame, area, "THERMAL", r, p);
            return;
        }
    };

    let block = panel_block("THERMAL", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if sensors.temps.is_empty() && sensors.fans.is_empty() {
        let p_empty = Paragraph::new(Line::from(Span::styled(
            "unavailable (no hwmon sensors)",
            Style::default().fg(p.unknown),
        )));
        frame.render_widget(p_empty, inner);
        return;
    }

    let mut sorted_temps: Vec<&TempSensor> = sensors.temps.iter().collect();
    sorted_temps.sort_by(|a, b| b.celsius.total_cmp(&a.celsius));

    let total_rows = inner.height as usize;
    let fan_rows = if sensors.fans.is_empty()
        || total_rows == 0
        || (total_rows == 1 && !sorted_temps.is_empty())
    {
        0
    } else {
        1
    };
    let temp_rows = total_rows.saturating_sub(fan_rows);

    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(total_rows);

    for t in sorted_temps.into_iter().take(temp_rows) {
        let near_crit = t.crit_celsius.is_some_and(|crit| t.celsius >= crit - 10.0);
        let at_crit = t.crit_celsius.is_some_and(|crit| t.celsius >= crit);
        let val_color = if at_crit || t.celsius >= 95.0 {
            p.alert
        } else if near_crit || t.celsius >= 80.0 {
            p.caution
        } else {
            p.nominal
        };

        let name = if inner.width >= 44 {
            format!("{}:{}", t.chip, t.label)
        } else {
            t.label.clone()
        };

        // The temperature keeps its cells; the label gets what is left,
        // shortened if needed, and is padded to 14 cells when there is room.
        let temp = format!("{:5.1}°C", t.celsius);
        let room = usize::from(inner.width).saturating_sub(super::fmt::display_width(&temp) + 1);
        let name = super::fmt::fit_width(&name, room);
        let pad = 14
            .min(room)
            .saturating_sub(super::fmt::display_width(&name));
        let mut spans = vec![
            Span::styled(format!("{name}{} ", " ".repeat(pad)), text_style),
            Span::styled(
                temp,
                Style::default().fg(val_color).add_modifier(Modifier::BOLD),
            ),
        ];

        if near_crit {
            let crit_val = t.crit_celsius.unwrap_or(0.0);
            spans.push(Span::styled(
                format!(" ! (crit {crit_val:.0}°C)"),
                Style::default().fg(p.alert).add_modifier(Modifier::BOLD),
            ));
        } else if let Some(crit_val) = t.crit_celsius
            && inner.width >= 42
        {
            spans.push(Span::styled(format!(" / {crit_val:.0}°C"), muted));
        }

        lines.push(Line::from(spans));
    }

    // If only 1 row was available and we have both temps and fans, append fan summary on that row
    if !sensors.fans.is_empty() {
        let w = inner.width as usize;
        let mut fan_spans = vec![Span::styled("FAN ", muted)];
        let mut used_w = 4usize;
        for (i, f) in sensors.fans.iter().enumerate() {
            let lbl = format!("{} ", f.label);
            let val = format!("{} RPM", f.rpm);
            let sep_w = if i > 0 { 2 } else { 0 };
            let entry_w = sep_w + lbl.chars().count() + val.chars().count();
            if i > 0 && used_w + entry_w > w {
                break;
            }
            if i > 0 {
                fan_spans.push(Span::styled("  ", muted));
            }
            fan_spans.push(Span::styled(lbl, muted));
            fan_spans.push(Span::styled(val, text_style));
            used_w += entry_w;
        }
        if fan_rows > 0 {
            lines.push(Line::from(fan_spans));
        } else if let Some(first) = lines.first_mut() {
            first.spans.push(Span::styled("  ", muted));
            first.spans.extend(fan_spans);
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use crate::model::Reading;
    use crate::ui::testkit::{buffer_text, nominal_app, render};

    /// The temperature keeps its cells; a long label is shortened instead.
    #[test]
    fn a_long_sensor_label_never_hides_the_temperature() {
        for (width, height) in [(80, 24), (120, 40)] {
            let mut app = nominal_app();
            let snapshot = app.snapshot.as_mut().unwrap();
            let Reading::Available(sensors) = &mut snapshot.sensors else {
                unreachable!()
            };
            sensors.temps[0].label = "VeryLongVendorSpecificSensorLabel-Package".into();
            sensors.temps[0].celsius = 99.7;
            let text = buffer_text(&render(&app, width, height));
            assert!(text.contains("99.7°C"), "{width}x{height}\n{text}");
        }
    }
}
