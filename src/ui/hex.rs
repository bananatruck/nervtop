//! Flat-topped hexagonal status cell widget and the three-cell consensus strip.

use crate::alert::{CellVote, Consensus, Level};
use crate::theme::Palette;
use crate::ui::fmt::level_color;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget};

/// Draws one flat-topped hexagonal consensus cell, or degrades to `[NAME VOTE]`
/// when `area.height < 5` or `area.width < 16`.
pub struct HexCell<'a> {
    pub name: &'a str,
    pub vote: Level,
    pub palette: &'a Palette,
}

impl<'a> HexCell<'a> {
    pub fn new(vote: &'a CellVote, palette: &'a Palette) -> Self {
        Self {
            name: vote.cell.name(),
            vote: vote.vote,
            palette,
        }
    }
}

impl Widget for HexCell<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let color = level_color(self.palette, self.vote);
        let outline_style = Style::default().fg(color);
        let vote_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
        let name_style = Style::default()
            .fg(self.palette.text)
            .add_modifier(Modifier::BOLD);

        if area.height < 5 || area.width < 16 {
            let y = area.y + area.height / 2;
            let line = Line::from(vec![
                Span::styled("[", outline_style),
                Span::styled(self.name, name_style),
                Span::styled(" ", outline_style),
                Span::styled(self.vote.label(), vote_style),
                Span::styled("]", outline_style),
            ]);
            buf.set_line(area.x, y, &line, area.width);
            return;
        }

        let w = area.width as usize;
        let top_y = area.y + (area.height.saturating_sub(5)) / 2;
        let inner_narrow = w.saturating_sub(4);
        let inner_wide = w.saturating_sub(2);

        // Row 0:   __________
        let top_bar = format!("  {}  ", "_".repeat(inner_narrow));
        buf.set_stringn(area.x, top_y, top_bar, w, outline_style);

        // Row 1:  /   NAME   \
        let centered_name = center_text(self.name, inner_narrow);
        let row1 = Line::from(vec![
            Span::styled(" /", outline_style),
            Span::styled(centered_name, name_style),
            Span::styled("\\ ", outline_style),
        ]);
        buf.set_line(area.x, top_y + 1, &row1, area.width);

        // Row 2: /    VOTE    \
        let centered_vote = center_text(self.vote.label(), inner_wide);
        let row2 = Line::from(vec![
            Span::styled("/", outline_style),
            Span::styled(centered_vote, vote_style),
            Span::styled("\\", outline_style),
        ]);
        buf.set_line(area.x, top_y + 2, &row2, area.width);

        // Row 3: \            /
        let row3 = Line::from(vec![
            Span::styled("\\", outline_style),
            Span::styled(" ".repeat(inner_wide), outline_style),
            Span::styled("/", outline_style),
        ]);
        buf.set_line(area.x, top_y + 3, &row3, area.width);

        // Row 4:  \__________/
        let bot_bar = format!(" \\{}/ ", "_".repeat(inner_narrow));
        buf.set_stringn(area.x, top_y + 4, bot_bar, w, outline_style);
    }
}

fn center_text(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= width {
        return chars.into_iter().take(width).collect();
    }
    let total_pad = width - chars.len();
    let left_pad = total_pad / 2;
    let right_pad = total_pad - left_pad;
    let mut out = String::with_capacity(width);
    for _ in 0..left_pad {
        out.push(' ');
    }
    for c in chars {
        out.push(c);
    }
    for _ in 0..right_pad {
        out.push(' ');
    }
    out
}

/// Draws the three-cell consensus strip and verdict box.
pub fn draw_consensus(frame: &mut Frame, area: Rect, consensus: &Consensus, palette: &Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let reasons: Vec<String> = consensus
        .cells
        .iter()
        .filter(|c| matches!(c.vote, Level::Caution | Level::Critical))
        .filter_map(|c| c.reason.as_ref().map(|r| format!("{}: {r}", c.cell.name())))
        .collect();

    // The full layout puts three 22-column cells beside a bordered box whose
    // first line is the verdict. `Min` outranks `Length`, so a narrow frame
    // would clip that line; use the compact layout unless it fits whole.
    let verdict_cells = super::fmt::display_width(&format!(
        "VERDICT {} / {}",
        consensus.verdict.label(),
        consensus.agreement
    ));
    let full_width = 3 * 22 + 2 + verdict_cells;
    if area.height < 5 || usize::from(area.width) < full_width {
        let v_color = level_color(palette, consensus.verdict);
        if area.height >= 2 {
            let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
            let cols = Layout::horizontal([
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ])
            .split(rows[0]);

            for (i, vote) in consensus.cells.iter().enumerate() {
                frame.render_widget(HexCell::new(vote, palette), cols[i]);
            }

            let mut detail_spans = vec![
                Span::styled(
                    format!("VERDICT {}", consensus.verdict.label()),
                    Style::default().fg(v_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" / ", Style::default().fg(palette.muted)),
                Span::styled(
                    consensus.agreement.to_string(),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            if !reasons.is_empty() {
                detail_spans.push(Span::styled("  ", Style::default().fg(palette.muted)));
                detail_spans.push(Span::styled(
                    reasons.join(" | "),
                    Style::default().fg(v_color),
                ));
            }
            frame.render_widget(Paragraph::new(Line::from(detail_spans)), rows[1]);
        } else {
            let cols = Layout::horizontal([
                Constraint::Length(21),
                Constraint::Length(21),
                Constraint::Length(21),
                Constraint::Min(10),
            ])
            .split(area);
            for (i, vote) in consensus.cells.iter().enumerate() {
                frame.render_widget(HexCell::new(vote, palette), cols[i]);
            }
            let verdict_line = Line::from(vec![
                Span::styled(
                    format!("VERDICT {}", consensus.verdict.label()),
                    Style::default().fg(v_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" / ", Style::default().fg(palette.muted)),
                Span::styled(
                    consensus.agreement.to_string(),
                    Style::default().fg(palette.text),
                ),
            ]);
            frame.render_widget(Paragraph::new(verdict_line), cols[3]);
        }
        return;
    }

    let cols = Layout::horizontal([
        Constraint::Length(22),
        Constraint::Length(22),
        Constraint::Length(22),
        Constraint::Min(20),
    ])
    .split(area);

    for (i, vote) in consensus.cells.iter().enumerate() {
        frame.render_widget(HexCell::new(vote, palette), cols[i]);
    }

    let v_color = level_color(palette, consensus.verdict);
    let block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(v_color))
        .title(Span::styled(
            " CONSENSUS ",
            Style::default()
                .fg(palette.heading)
                .add_modifier(Modifier::BOLD),
        ));

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            format!("VERDICT {}", consensus.verdict.label()),
            Style::default().fg(v_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" / ", Style::default().fg(palette.muted)),
        Span::styled(
            consensus.agreement.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if reasons.is_empty() && consensus.verdict == Level::Unknown {
        lines.push(Line::from(Span::styled(
            "NO DATA - nothing could be judged yet",
            Style::default().fg(palette.unknown),
        )));
    } else if reasons.is_empty() {
        lines.push(Line::from(Span::styled(
            "All judged domains within configured bounds",
            Style::default().fg(palette.muted),
        )));
    } else {
        for r in reasons {
            lines.push(Line::from(Span::styled(r, Style::default().fg(v_color))));
        }
    }

    frame.render_widget(Paragraph::new(lines).block(block), cols[3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alert::CellId;
    use crate::theme::ThemeName;

    fn buffer_text(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// The verdict and agreement stay complete: a frame too narrow for the
    /// three cells and the whole bordered verdict line uses the compact
    /// layout, as a short frame does; a wide one keeps the full layout.
    #[test]
    fn the_verdict_line_is_complete_at_every_width() {
        let app = crate::ui::testkit::nominal_app();
        let verdict = format!(
            "VERDICT {} / {}",
            app.consensus.verdict.label(),
            app.consensus.agreement
        );
        for (width, full) in [(80, false), (94, false), (120, true), (200, true)] {
            let text =
                crate::ui::testkit::buffer_text(&crate::ui::testkit::render(&app, width, 40));
            assert!(
                text.contains(&verdict),
                "{width}: {verdict} missing\n{text}"
            );
            assert_eq!(text.contains(" CONSENSUS "), full, "{width}");
        }
    }

    #[test]
    fn hex_cell_renders_full_hexagon_when_at_least_16x5() {
        let palette = Palette::builtin(ThemeName::Nerv);
        let vote = CellVote {
            cell: CellId::Compute,
            vote: Level::Nominal,
            reason: None,
        };
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        HexCell::new(&vote, &palette).render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(text.contains("COMPUTE-1"), "{text}");
        assert!(text.contains("NOMINAL"), "{text}");
        assert!(text.contains("____"), "{text}");
        assert!(text.contains('/'), "{text}");
        assert!(text.contains('\\'), "{text}");
        assert!(!text.contains("[COMPUTE-1"), "{text}");
        // Check level colour applied to outline
        assert_eq!(buf[(2, 0)].fg, palette.nominal);
    }

    #[test]
    fn hex_cell_degrades_to_bracketed_line_when_short_or_narrow() {
        let palette = Palette::builtin(ThemeName::Nerv);
        let vote = CellVote {
            cell: CellId::Compute,
            vote: Level::Nominal,
            reason: None,
        };

        // Under 5 rows
        let area_short = Rect::new(0, 0, 22, 3);
        let mut buf_short = Buffer::empty(area_short);
        HexCell::new(&vote, &palette).render(area_short, &mut buf_short);
        let text_short = buffer_text(&buf_short);
        assert!(text_short.contains("[COMPUTE-1 NOMINAL]"), "{text_short}");

        // Under 16 columns
        let area_narrow = Rect::new(0, 0, 15, 6);
        let mut buf_narrow = Buffer::empty(area_narrow);
        HexCell::new(&vote, &palette).render(area_narrow, &mut buf_narrow);
        let text_narrow = buffer_text(&buf_narrow);
        assert!(text_narrow.contains("[COMPUTE-1"), "{text_narrow}");
    }

    #[test]
    fn hex_cell_never_draws_reason_inside_hexagon() {
        let palette = Palette::builtin(ThemeName::Nerv);
        let vote = CellVote {
            cell: CellId::Thermal,
            vote: Level::Caution,
            reason: Some("CPU TEMP k10temp Tctl 94.4°C".into()),
        };
        let area = Rect::new(0, 0, 22, 5);
        let mut buf = Buffer::empty(area);
        HexCell::new(&vote, &palette).render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(text.contains("THERMAL-2"), "{text}");
        assert!(text.contains("CAUTION"), "{text}");
        assert!(!text.contains("CPU TEMP"), "{text}");
        assert!(!text.contains("k10temp"), "{text}");
    }

    #[test]
    fn hex_cell_handles_tiny_1x1_and_empty_without_panic() {
        let palette = Palette::builtin(ThemeName::Nerv);
        let vote = CellVote {
            cell: CellId::Thermal,
            vote: Level::Critical,
            reason: Some("CPU TEMP 98.0°C".into()),
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        HexCell::new(&vote, &palette).render(Rect::new(0, 0, 1, 1), &mut buf);
        HexCell::new(&vote, &palette).render(Rect::new(0, 0, 0, 0), &mut buf);
    }
}
