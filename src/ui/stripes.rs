//! Warning-stripe banner widget shown when consensus verdict is `Level::Critical`.

use crate::alert::{Level, RuleId, RuleStatus};
use crate::app::App;
use crate::theme::Palette;
use crate::ui::fmt::{self, level_color};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
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

        // Measure and cut by display width, then let `set_stringn` write
        // whole graphemes, so a double-width character keeps its blank
        // continuation cell instead of having the next character written
        // into it (where the terminal would never show it).
        let padded = format!("  {}  ", self.label);
        let w = area.width as usize;
        let shown = fit_cells(&padded, w);
        let shown_w = fmt::display_width(&shown);
        let start_x = area.x + (w.saturating_sub(shown_w) / 2) as u16;
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
        let label_area = Rect::new(start_x, mid_y, shown_w as u16, 1).intersection(area);
        buf.set_style(label_area, label_style);
        buf.set_stringn(
            start_x,
            mid_y,
            &shown,
            label_area.width as usize,
            label_style,
        );
    }
}

/// The longest prefix of `s` made of whole graphemes that fits in `max` cells.
fn fit_cells(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for g in Span::raw(s).styled_graphemes(Style::default()) {
        let gw = fmt::display_width(g.symbol);
        if used + gw > max {
            break;
        }
        out.push_str(g.symbol);
        used += gw;
    }
    out
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

    /// Column of the first cell whose symbol is `symbol`.
    fn column_of(buf: &Buffer, symbol: &str) -> u16 {
        (0..buf.area.width)
            .find(|&x| buf[(x, 0)].symbol() == symbol)
            .unwrap_or_else(|| panic!("{symbol:?} in {:?}", buffer_line(buf)))
    }

    #[test]
    fn wide_characters_in_the_label_keep_their_continuation_cells() {
        let palette = Palette::builtin(ThemeName::Nerv);
        for acknowledged in [false, true] {
            let area = Rect::new(0, 0, 40, 1);
            let mut buf = Buffer::empty(area);
            WarningStripes::new(
                "CRITICAL  DISK /mnt/数据  97.0%",
                Level::Critical,
                acknowledged,
                &palette,
            )
            .render(area, &mut buf);
            let first = column_of(&buf, "数");
            assert_eq!(buf[(first + 1, 0)].symbol(), " ", "continuation after 数");
            assert_eq!(buf[(first + 2, 0)].symbol(), "据");
            assert_eq!(buf[(first + 3, 0)].symbol(), " ", "continuation after 据");
            assert_eq!(buf[(first + 4, 0)].symbol(), " ");
            assert_eq!(buf[(first + 6, 0)].symbol(), "9");
            // "  CRITICAL  DISK /mnt/数据  97.0%  " is 35 cells wide, so it
            // starts at column (40 - 35) / 2 = 2.
            assert_eq!(column_of(&buf, "C"), 4);
            assert_eq!(
                buf[(first, 0)].bg,
                if acknowledged {
                    palette.surface
                } else {
                    palette.alert
                }
            );
        }
    }

    #[test]
    fn wide_label_is_cut_by_display_width() {
        let palette = Palette::builtin(ThemeName::Nerv);
        // Six double-width characters plus padding need 16 cells; only 9 exist.
        let area = Rect::new(0, 0, 9, 1);
        let mut buf = Buffer::empty(area);
        WarningStripes::new("数据数据数据", Level::Critical, true, &palette).render(area, &mut buf);
        let line = buffer_line(&buf);
        // Two padding cells, then three whole wide characters (six cells),
        // each followed by its blank continuation cell, and the last cell
        // stays blank instead of holding half a glyph.
        assert_eq!(buf[(2, 0)].symbol(), "数", "{line:?}");
        assert_eq!(buf[(3, 0)].symbol(), " ", "{line:?}");
        assert_eq!(buf[(4, 0)].symbol(), "据", "{line:?}");
        assert_eq!(buf[(5, 0)].symbol(), " ", "{line:?}");
        assert_eq!(buf[(6, 0)].symbol(), "数", "{line:?}");
        assert_eq!(buf[(7, 0)].symbol(), " ", "{line:?}");
        assert_eq!(buf[(8, 0)].symbol(), " ", "{line:?}");
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
