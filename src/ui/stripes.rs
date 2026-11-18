//! Warning-stripe banner widget shown when consensus verdict is `Level::Critical`.

use crate::alert::{Level, RuleId, RuleStatus};
use crate::app::App;
use crate::theme::Palette;
use crate::ui::fmt::level_color;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;

/// Character used to draw diagonal warning stripes when unacknowledged.
pub const STRIPE_CHAR: char = '╱';

/// Draws a full-width warning-stripe bar with a centred label.
/// When `acknowledged` is true, omits the diagonal stripes and renders only the centred label.
pub struct WarningStripes<'a> {
    pub label: &'a str,
    pub level: Level,
    pub acknowledged: bool,
    pub palette: &'a Palette,
}

impl<'a> WarningStripes<'a> {
    pub fn new(label: &'a str, level: Level, acknowledged: bool, palette: &'a Palette) -> Self {
        Self {
            label,
            level,
            acknowledged,
            palette,
        }
    }
}

impl Widget for WarningStripes<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let color = level_color(self.palette, self.level);
        let dark = self.palette.stripe_dark;

        for y in area.y..area.y.saturating_add(area.height) {
            for x in area.x..area.x.saturating_add(area.width) {
                let cell = &mut buf[(x, y)];
                if self.acknowledged {
                    cell.set_char(' ');
                    cell.set_style(Style::default().fg(color).bg(self.palette.surface));
                } else {
                    let band = ((x as usize) + (y as usize)) / 2;
                    let style = if band.is_multiple_of(2) {
                        Style::default().fg(color).bg(dark)
                    } else {
                        Style::default().fg(dark).bg(color)
                    };
                    cell.set_char(STRIPE_CHAR);
                    cell.set_style(style);
                }
            }
        }

        let padded = format!("  {}  ", self.label);
        let chars: Vec<char> = padded.chars().collect();
        let w = area.width as usize;
        let shown: Vec<char> = chars.into_iter().take(w).collect();
        let start_x = area.x + (w.saturating_sub(shown.len()) / 2) as u16;
        let mid_y = area.y + area.height / 2;
        let label_style = if self.acknowledged {
            Style::default()
                .fg(color)
                .bg(self.palette.surface)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(dark)
                .bg(color)
                .add_modifier(Modifier::BOLD)
        };

        for (i, ch) in shown.into_iter().enumerate() {
            let cx = start_x.saturating_add(i as u16);
            if cx < area.x.saturating_add(area.width) {
                let cell = &mut buf[(cx, mid_y)];
                cell.set_char(ch);
                cell.set_style(label_style);
            }
        }
    }
}

/// Formats a `RuleStatus` value with its unit.
pub fn format_rule_value(rule: RuleId, value: f64) -> String {
    match rule {
        RuleId::Load => format!("{value:.2}{}", rule.unit()),
        _ => format!("{value:.1}{}", rule.unit()),
    }
}

/// Builds the banner label for the worst rule in `app`.
pub fn worst_banner_label(app: &App) -> String {
    let worst_rule: Option<&RuleStatus> = app
        .rules
        .iter()
        .filter(|r| r.level == Level::Critical)
        .max_by(|a, b| {
            let va = a.value.unwrap_or(0.0);
            let vb = b.value.unwrap_or(0.0);
            va.total_cmp(&vb)
        })
        .or_else(|| app.rules.iter().max_by_key(|r| r.level));

    if let Some(r) = worst_rule
        && r.level != Level::Unknown
    {
        let subject = if r.subject.is_empty() {
            String::new()
        } else {
            format!(" {}", r.subject)
        };
        if let Some(v) = r.value {
            return format!(
                "{}  {}{}  {}",
                r.level.label(),
                r.rule.label(),
                subject,
                format_rule_value(r.rule, v)
            );
        }
        return format!("{}  {}{}", r.level.label(), r.rule.label(), subject);
    }

    if let Some(reason) = app
        .consensus
        .cells
        .iter()
        .filter(|c| c.vote == Level::Critical)
        .find_map(|c| c.reason.as_deref())
    {
        return format!("CRITICAL  {reason}");
    }

    "CRITICAL  ALERT ACTIVE".to_string()
}

/// Draws the warning-stripe banner for `app` into `area`.
pub fn draw_banner(frame: &mut Frame, area: Rect, app: &App) {
    if app.consensus.verdict != Level::Critical {
        return;
    }
    let label = worst_banner_label(app);
    let widget = WarningStripes::new(&label, Level::Critical, app.acknowledged, &app.palette);
    frame.render_widget(widget, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeName;

    fn buffer_line(buf: &Buffer) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol())
            .collect::<String>()
    }

    #[test]
    fn stripes_draw_diagonal_bands_when_not_acknowledged() {
        let palette = Palette::builtin(ThemeName::Nerv);
        let area = Rect::new(0, 0, 50, 1);
        let mut buf = Buffer::empty(area);
        WarningStripes::new("CRITICAL  DISK /  97.0%", Level::Critical, false, &palette)
            .render(area, &mut buf);
        let line = buffer_line(&buf);
        assert!(line.contains("CRITICAL  DISK /  97.0%"), "{line}");
        assert!(line.contains(STRIPE_CHAR), "{line}");
        assert_eq!(buf[(0, 0)].fg, palette.alert);
        assert_eq!(buf[(0, 0)].bg, palette.stripe_dark);
        assert_eq!(buf[(2, 0)].fg, palette.stripe_dark);
        assert_eq!(buf[(2, 0)].bg, palette.alert);
    }

    #[test]
    fn stripes_are_absent_when_acknowledged() {
        let palette = Palette::builtin(ThemeName::Nerv);
        let area = Rect::new(0, 0, 50, 1);
        let mut buf = Buffer::empty(area);
        WarningStripes::new("CRITICAL  DISK /  97.0%", Level::Critical, true, &palette)
            .render(area, &mut buf);
        let line = buffer_line(&buf);
        assert!(line.contains("CRITICAL  DISK /  97.0%"), "{line}");
        assert!(!line.contains(STRIPE_CHAR), "{line}");
    }

    #[test]
    fn stripes_handle_tiny_rects_without_panic() {
        let palette = Palette::builtin(ThemeName::Nerv);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        WarningStripes::new("CRITICAL", Level::Critical, false, &palette)
            .render(Rect::new(0, 0, 1, 1), &mut buf);
        WarningStripes::new("CRITICAL", Level::Critical, true, &palette)
            .render(Rect::new(0, 0, 0, 0), &mut buf);
    }
}
