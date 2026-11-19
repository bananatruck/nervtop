//! STORAGE panel: per-mount usage bar (coloured by matching disk alert rule), used/total and I/O rates.

use crate::alert::{Level, RuleId};
use crate::app::App;
use crate::model::{DiskStats, Reading};
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Draws the STORAGE telemetry panel.
pub fn draw_storage(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "STORAGE", p);
        return;
    };
    let disks = match &snap.disks {
        Reading::Available(list) => list,
        Reading::Unavailable(r) => {
            draw_unavailable(frame, area, "STORAGE", r, p);
            return;
        }
    };

    let block = panel_block("STORAGE", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if disks.is_empty() {
        let p_empty = Paragraph::new(Line::from(Span::styled(
            "unavailable (no disks mounted)",
            Style::default().fg(p.unknown),
        )));
        frame.render_widget(p_empty, inner);
        return;
    }

    let w = inner.width as usize;
    let two_line_per_disk = w < 64 && inner.height as usize >= disks.len().saturating_mul(2);
    let mut lines: Vec<Line<'static>> = Vec::new();

    for d in disks {
        if lines.len() >= inner.height as usize {
            break;
        }
        render_disk(d, app, w, two_line_per_disk, &mut lines);
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_disk(
    d: &DiskStats,
    app: &App,
    width: usize,
    two_line: bool,
    lines: &mut Vec<Line<'static>>,
) {
    let p = &app.palette;
    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let bold_text = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
    let unknown = Style::default().fg(p.unknown);

    let rule_level = app
        .rules
        .iter()
        .find(|r| r.rule == RuleId::Disk && r.subject == d.mount_point)
        .or_else(|| app.rules.iter().find(|r| r.rule == RuleId::Disk))
        .map(|r| r.level)
        .unwrap_or(Level::Nominal);

    let bar_color = match rule_level {
        Level::Unknown => p.nominal,
        lvl => fmt::level_color(p, lvl),
    };

    // A zero-sized filesystem has no usage percentage (the disk rule calls
    // it UNKNOWN), so it shows "unavailable" and an unknown-coloured gauge
    // instead of a nominal 0.0%.
    let pct = d.used_percent();
    let bar_w = if width >= 64 { 8 } else { 4 };
    let (filled, empty) = fmt::bar(bar_w, pct.unwrap_or(0.0));
    let (filled_color, empty_color) = match pct {
        Some(_) => (bar_color, p.border),
        None => (p.unknown, p.unknown),
    };
    let (pct_s, pct_style) = match pct {
        Some(v) => (fmt::percent(v), bold_text),
        None => ("unavailable".to_string(), unknown),
    };
    let pair_s = fmt::bytes_pair(d.used_bytes(), d.total_bytes);
    let reason_room = if width >= 60 { 24 } else { 11 };

    let r_span = match &d.read_bps {
        Reading::Available(bps) => Span::styled(fmt::rate(*bps), text_style),
        Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
    };
    let w_span = match &d.write_bps {
        Reading::Available(bps) => Span::styled(fmt::rate(*bps), text_style),
        Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, reason_room), unknown),
    };

    // The gauge, percentage and capacity are reserved first; the mount
    // point gets the columns that are left and is shortened only when it
    // would not fit whole.
    if two_line {
        let pct_cell = format!(" {pct_s:>5} ");
        let metrics_w = 1 + bar_w + pct_cell.len() + pair_s.len();
        let room = width.saturating_sub(metrics_w);
        let label = fmt::fit_width(&d.mount_point, room);
        let pad = 8usize.min(room).saturating_sub(fmt::display_width(&label));
        lines.push(Line::from(vec![
            Span::styled(format!("{label}{} ", " ".repeat(pad)), bold_text),
            Span::styled(filled, Style::default().fg(filled_color)),
            Span::styled(empty, Style::default().fg(empty_color)),
            Span::styled(pct_cell, pct_style),
            Span::styled(pair_s, text_style),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  R ", muted),
            r_span,
            Span::styled("  W ", muted),
            w_span,
        ]));
    } else {
        let metrics_w = 1 + bar_w + 1 + pct_s.len() + 1 + pair_s.len();
        let label = fmt::fit_width(&d.mount_point, width.saturating_sub(metrics_w));
        let base_len = fmt::display_width(&label) + metrics_w;
        let rates_len = 3 + r_span.content.chars().count() + 3 + w_span.content.chars().count();
        let mut row = vec![
            Span::styled(format!("{label} "), bold_text),
            Span::styled(filled, Style::default().fg(filled_color)),
            Span::styled(empty, Style::default().fg(empty_color)),
            Span::styled(format!(" {pct_s} "), pct_style),
            Span::styled(pair_s, text_style),
        ];
        if base_len + rates_len <= width {
            row.push(Span::styled(" R ", muted));
            row.push(r_span);
            row.push(Span::styled(" W ", muted));
            row.push(w_span);
        }
        lines.push(Line::from(row));
    }
}

#[cfg(test)]
mod tests {
    use super::draw_storage;
    use crate::app::App;
    use crate::model::Reading;
    use crate::ui::testkit::{buffer_lines, nominal_app};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    /// Draws only the STORAGE panel into a `width` x `height` terminal.
    fn panel(app: &App, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw_storage(frame, frame.area(), app))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_index(lines: &[String], needle: &str) -> usize {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("a row containing {needle:?}:\n{}", lines.join("\n")))
    }

    #[test]
    fn zero_sized_disk_shows_unavailable_percent_and_unknown_gauge() {
        let mut app = nominal_app();
        if let Some(Reading::Available(d)) = app.snapshot.as_mut().map(|s| &mut s.disks) {
            d[1].total_bytes = 0;
            d[1].available_bytes = 0;
        }
        // Width 80 draws one line per disk, width 40 two lines per disk.
        for (width, height) in [(80, 6), (40, 6)] {
            let buf = panel(&app, width, height);
            let lines = buffer_lines(&buf);
            let y = row_index(&lines, "/home");
            let row = &lines[y];
            assert!(row.contains("unavailable"), "{width}: {row:?}");
            assert!(!row.contains("0.0%"), "{width}: {row:?}");
            let gauge_x = row
                .chars()
                .position(|c| c == '─' || c == '━')
                .expect("a gauge on the row");
            let gauge_fg = buf[(gauge_x as u16, y as u16)].fg;
            assert_eq!(gauge_fg, app.palette.unknown, "{width}: gauge colour");
        }
    }

    #[test]
    fn long_mount_point_leaves_room_for_percent_and_capacity() {
        let mut app = nominal_app();
        if let Some(Reading::Available(d)) = app.snapshot.as_mut().map(|s| &mut s.disks) {
            d[0].mount_point = "/run/media/alice/ExternalBackupDrive".into();
        }
        // 40 columns is the STORAGE panel at 80 columns (38 inner columns).
        // Height 6 gives two lines per disk, height 4 one line each.
        for height in [6, 4] {
            let lines = buffer_lines(&panel(&app, 40, height));
            let row = &lines[row_index(&lines, "/run/")];
            assert!(row.contains("56.0%"), "height {height}: {row:?}");
            assert!(
                row.contains("280.0 GiB/500.0 GiB"),
                "height {height}: {row:?}"
            );
        }
    }

    #[test]
    fn mount_point_stays_whole_when_it_fits() {
        let mut app = nominal_app();
        if let Some(Reading::Available(d)) = app.snapshot.as_mut().map(|s| &mut s.disks) {
            d[0].mount_point = "/run/media/alice/ExternalBackupDrive".into();
        }
        let lines = buffer_lines(&panel(&app, 100, 4));
        let row = &lines[row_index(&lines, "/run/")];
        assert!(
            row.contains("/run/media/alice/ExternalBackupDrive "),
            "{row:?}"
        );
        assert!(row.contains("56.0%"), "{row:?}");
    }
}
