//! PROCESSES overview panel and full-screen interactive process table.

use crate::app::{App, InputMode, SortKey};
use crate::model::Reading;
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, draw_unavailable, panel_block};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

/// Draws the compact PROCESSES panel on the OVERVIEW screen (top rows by current sort).
pub fn draw_processes_overview(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "PROCESSES", p);
        return;
    };
    if let Reading::Unavailable(r) = &snap.processes {
        draw_unavailable(frame, area, "PROCESSES", r, p);
        return;
    }

    let dir_arrow = if app.procs.descending { "▼" } else { "▲" };
    let title = format!("PROCESSES [{}{}]", app.procs.sort.label(), dir_arrow);
    let block = panel_block(&title, p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let visible = app.visible_processes();
    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let bold_text = Style::default().fg(p.text).add_modifier(Modifier::BOLD);

    if visible.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("no matching processes", muted))),
            inner,
        );
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let max_rows = inner.height as usize;

    if max_rows >= 3 {
        lines.push(Line::from(vec![
            Span::styled(format!("{:>6} ", "PID"), muted),
            Span::styled(format!("{:>5} ", "CPU%"), muted),
            Span::styled(format!("{:>8} ", "MEM"), muted),
            Span::styled("NAME", muted),
        ]));
    }

    let take_n = max_rows.saturating_sub(lines.len());
    for proc in visible.into_iter().take(take_n) {
        let is_sel = app.procs.selected_pid == Some(proc.pid);
        let row_style = if is_sel {
            Style::default()
                .fg(p.accent)
                .bg(p.surface)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            text_style
        };
        let name_s = if is_sel { row_style } else { bold_text };
        lines.push(Line::from(vec![
            Span::styled(format!("{:>6} ", proc.pid), row_style),
            Span::styled(
                match proc.cpu_percent.value() {
                    Some(v) => format!("{v:>4.1}% "),
                    None => format!("{:>5} ", "--"),
                },
                row_style,
            ),
            Span::styled(format!("{:>8} ", fmt::bytes(proc.memory_bytes)), row_style),
            Span::styled(proc.name.clone(), name_s),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draws the full-height PROCESSES screen.
pub fn draw_processes_screen(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "PROCESSES", p);
        return;
    };
    let all_procs = match &snap.processes {
        Reading::Available(list) => list,
        Reading::Unavailable(r) => {
            draw_unavailable(frame, area, "PROCESSES", r, p);
            return;
        }
    };

    let visible = app.visible_processes();
    let total_count = all_procs.len();
    let shown_count = visible.len();

    let block = panel_block("PROCESSES", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    let bar_area = chunks[0];
    let table_area = chunks[1];

    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);
    let accent = Style::default().fg(p.accent).add_modifier(Modifier::BOLD);

    let dir_str = if app.procs.descending {
        "▼ DESC"
    } else {
        "▲ ASC"
    };
    let filter_display = match app.procs.mode {
        InputMode::Filter => format!("/{}█", app.procs.filter),
        InputMode::Normal => {
            if app.procs.filter.is_empty() {
                "none".to_string()
            } else {
                app.procs.filter.clone()
            }
        }
    };
    let filter_style = if app.procs.mode == InputMode::Filter || !app.procs.filter.is_empty() {
        accent
    } else {
        muted
    };

    let bar_line = Line::from(vec![
        Span::styled("SORT ", muted),
        Span::styled(format!("{} {}", app.procs.sort.label(), dir_str), accent),
        Span::styled("   FILTER ", muted),
        Span::styled(filter_display, filter_style),
        Span::styled("   COUNT ", muted),
        Span::styled(format!("{shown_count} of {total_count}"), text_style),
    ]);
    frame.render_widget(Paragraph::new(bar_line), bar_area);

    if table_area.height == 0 {
        return;
    }

    let col_title = |key: SortKey, base: &str| -> String {
        if app.procs.sort == key {
            let arrow = if app.procs.descending { "▼" } else { "▲" };
            format!("{base} {arrow}")
        } else {
            base.to_string()
        }
    };

    let header_style = Style::default()
        .fg(p.heading)
        .bg(p.surface)
        .add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from(col_title(SortKey::Pid, "PID")),
        Cell::from("USER"),
        Cell::from(col_title(SortKey::Cpu, "CPU%")),
        Cell::from(col_title(SortKey::Memory, "MEM")),
        Cell::from("STATE"),
        Cell::from(col_title(SortKey::Name, "NAME")),
    ])
    .style(header_style);

    let selected_idx = app
        .procs
        .selected_pid
        .and_then(|pid| visible.iter().position(|pr| pr.pid == pid));

    let highlight_style = Style::default()
        .fg(p.accent)
        .bg(p.surface)
        .add_modifier(Modifier::BOLD | Modifier::REVERSED);

    let rows: Vec<Row<'static>> = visible
        .iter()
        .map(|pr| {
            let is_selected = app.procs.selected_pid == Some(pr.pid);
            let style = if is_selected {
                highlight_style
            } else {
                Style::default().fg(p.text).bg(p.background)
            };
            Row::new(vec![
                Cell::from(pr.pid.to_string()),
                Cell::from(pr.user.clone()),
                Cell::from(
                    pr.cpu_percent
                        .value()
                        .map_or_else(|| "--".to_owned(), |v| fmt::percent(*v as f64)),
                ),
                Cell::from(fmt::bytes(pr.memory_bytes)),
                Cell::from(pr.state.clone()),
                Cell::from(pr.name.clone()),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(8),
        Constraint::Length(12),
        Constraint::Length(9),
        Constraint::Length(11),
        Constraint::Length(8),
        Constraint::Min(12),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let max_offset = visible.len().saturating_sub(1);
    let mut state = TableState::default()
        .with_offset(app.procs.offset.min(max_offset))
        .with_selected(selected_idx);

    frame.render_stateful_widget(table, table_area, &mut state);
}
