//! DIAGNOSTICS screen: full inspection tables for sensors, fans, GPUs, battery, disks, network, rules and alert log.

use crate::alert::{Level, RuleId, RuleStatus};
use crate::app::App;
use crate::model::{BatteryStatus, GpuPower, Reading, SensorKind};
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, panel_block};
use crate::ui::network::sorted_interfaces;
use crate::ui::stripes::format_rule_value;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

/// Keep each table on its last full page when the shared offset exceeds its length.
fn visible_rows(
    rows: Vec<Row<'static>>,
    inner: Rect,
    header: bool,
    offset: usize,
) -> Vec<Row<'static>> {
    let offset = offset.min(maximum_offset(rows.len(), inner, header));
    rows.into_iter().skip(offset).collect()
}

fn maximum_offset(rows: usize, inner: Rect, header: bool) -> usize {
    let capacity = inner.height.saturating_sub(u16::from(header)) as usize;
    if inner.width == 0 || capacity == 0 {
        0
    } else {
        rows.saturating_sub(capacity)
    }
}

fn table_areas(area: Rect, app: &App) -> [Rect; 6] {
    let cols = if area.width < 100 {
        Layout::horizontal([Constraint::Length(31), Constraint::Min(49)]).split(area)
    } else {
        Layout::horizontal([Constraint::Percentage(53), Constraint::Percentage(47)]).split(area)
    };

    let sensor_rows = match app.snapshot.as_ref().map(|s| &s.sensors) {
        Some(Reading::Available(s)) => (s.temps.len() + s.fans.len()).max(1) as u16,
        _ => 1,
    };
    let gpu_rows = match app.snapshot.as_ref().map(|s| &s.gpus) {
        Some(Reading::Available(g)) => g.len().max(1) as u16,
        _ => 1,
    };
    let battery_rows = match app.snapshot.as_ref().map(|s| &s.battery) {
        Some(Reading::Available(_)) => 8,
        _ => 1,
    };

    let sensors_h = sensor_rows + 3; // rows + header + borders
    let gpus_h = gpu_rows + 3; // adapters + header + borders
    let battery_h = battery_rows + 2; // its rows + borders

    let left_rows = if cols[0].height <= 22 {
        Layout::vertical([
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(4),
        ])
        .split(cols[0])
    } else if sensors_h + gpus_h + battery_h + 4 <= cols[0].height {
        Layout::vertical([
            Constraint::Length(sensors_h),
            Constraint::Length(gpus_h),
            Constraint::Length(battery_h),
            Constraint::Min(4),
        ])
        .split(cols[0])
    } else {
        let cap_gpus = gpus_h.min(cols[0].height / 4).max(4);
        let cap_bat = battery_h.min(cols[0].height / 4).max(4);
        let cap_sens = sensors_h
            .min(cols[0].height.saturating_sub(cap_gpus + cap_bat + 4))
            .max(4);
        Layout::vertical([
            Constraint::Length(cap_sens),
            Constraint::Length(cap_gpus),
            Constraint::Length(cap_bat),
            Constraint::Min(4),
        ])
        .split(cols[0])
    };

    let rule_count = app.rules.len().max(RuleId::ALL.len()) as u16;
    let rules_box_h = if cols[1].height <= 22 {
        (rule_count + 2)
            .min(cols[1].height.saturating_sub(4))
            .max(11)
    } else {
        (rule_count + 3)
            .min(cols[1].height.saturating_sub(8))
            .max(12)
    };
    let right_rows =
        Layout::vertical([Constraint::Length(rules_box_h), Constraint::Min(4)]).split(cols[1]);

    [
        left_rows[0],
        left_rows[1],
        left_rows[2],
        left_rows[3],
        right_rows[0],
        right_rows[1],
    ]
}

/// Navigation bounds of the rendered diagnostics tables, which share one
/// scroll offset: the last offset that moves at least one table, and a page
/// step no larger than the smallest data-row capacity among the tables that
/// scroll, so a page never skips a row of any table.
pub fn bounds(area: Rect, app: &App) -> (usize, usize) {
    let areas = table_areas(area, app).map(|area| panel_block("", &app.palette).inner(area));
    let mut rows = [0; 6];
    if let Some(snapshot) = &app.snapshot {
        rows[0] = snapshot
            .sensors
            .value()
            .map_or(0, |s| s.temps.len() + s.fans.len());
        rows[1] = snapshot.gpus.value().map_or(0, Vec::len);
        rows[2] = if snapshot.battery.is_available() {
            8
        } else {
            0
        };
        rows[3] = snapshot.disks.value().map_or(1, Vec::len)
            + snapshot.network.value().map_or(1, Vec::len);
    }
    rows[4] = if app.rules.is_empty() {
        RuleId::ALL.len()
    } else {
        app.rules.len()
    };
    rows[5] = app.log.len();
    let headers = [
        areas[0].height >= 2,
        areas[1].height >= 2,
        false,
        false,
        areas[4].height as usize > rows[4],
        areas[5].height >= 4,
    ];
    let mut last = 0;
    let mut page: Option<usize> = None;
    for ((rows, area), header) in rows.into_iter().zip(areas).zip(headers) {
        let max = maximum_offset(rows, area, header);
        if max > 0 {
            last = last.max(max);
            let capacity = area.height.saturating_sub(u16::from(header)) as usize;
            page = Some(page.map_or(capacity, |p| p.min(capacity)));
        }
    }
    (last, page.unwrap_or(1).max(1))
}

/// Draws the DIAGNOSTICS screen using `Table` widgets for all subsystems.
pub fn draw_diagnostics(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let [sensors, gpus, battery, io, rules, events] = table_areas(area, app);
    draw_sensors_table(frame, sensors, app);
    draw_gpus_table(frame, gpus, app);
    draw_battery_table(frame, battery, app);
    draw_io_table(frame, io, app);
    draw_rules_table(frame, rules, app);
    draw_events_table(frame, events, app);
}

fn draw_sensors_table(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "SENSORS & FANS", p);
        return;
    };
    let block = panel_block("SENSORS & FANS", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let text_style = Style::default().fg(p.text);
    let muted = Style::default().fg(p.muted);
    let unknown = Style::default().fg(p.unknown);

    let sensors = match &snap.sensors {
        Reading::Unavailable(r) => {
            let line = Line::from(Span::styled(
                fmt::unavailable_str(r, inner.width as usize),
                unknown,
            ));
            frame.render_widget(Paragraph::new(line), inner);
            return;
        }
        Reading::Available(s) if s.temps.is_empty() && s.fans.is_empty() => {
            let line = Line::from(Span::styled("unavailable (no sensors)", unknown));
            frame.render_widget(Paragraph::new(line), inner);
            return;
        }
        Reading::Available(s) => s,
    };

    let mut raw_rows: Vec<([String; 4], [Style; 4])> = Vec::new();
    for t in &sensors.temps {
        let kind_str = match t.kind {
            SensorKind::Cpu => "cpu",
            SensorKind::Gpu => "gpu",
            SensorKind::Storage => "storage",
            SensorKind::Other => "other",
        };
        let crit_str = t
            .crit_celsius
            .map(|c| format!("crit {c:.0}°C"))
            .unwrap_or_else(|| "-".to_string());
        raw_rows.push((
            [
                format!("{}:{}", t.chip, t.label),
                kind_str.to_string(),
                format!("{:.1}°C", t.celsius),
                crit_str,
            ],
            [text_style, muted, text_style, muted],
        ));
    }
    for f in &sensors.fans {
        raw_rows.push((
            [
                format!("{}:{}", f.chip, f.label),
                "fan".to_string(),
                format!("{} RPM", f.rpm),
                "-".to_string(),
            ],
            [text_style, muted, text_style, muted],
        ));
    }

    let headers = ["SENSOR", "KIND", "VALUE", "LIMIT"];
    let mut col_w = [
        headers[0].len(),
        headers[1].len(),
        headers[2].len(),
        headers[3].len(),
    ];
    for (cells, _) in &raw_rows {
        for (i, c) in cells.iter().enumerate() {
            col_w[i] = col_w[i].max(c.chars().count());
        }
    }

    let w = inner.width as usize;
    let col_indices: &[usize] = if col_w[0] + col_w[1] + col_w[2] + col_w[3] + 3 <= w {
        &[0, 1, 2, 3]
    } else if col_w[0] + col_w[1] + col_w[2] + 2 <= w {
        &[0, 1, 2]
    } else {
        &[0, 2]
    };

    let rows: Vec<Row<'static>> = raw_rows
        .into_iter()
        .map(|(cells, styles)| {
            Row::new(
                col_indices
                    .iter()
                    .map(|&i| Cell::from(Span::styled(cells[i].clone(), styles[i])))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    let constraints: Vec<Constraint> = col_indices
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            // The identifier column takes what the measurements leave: a
            // `Min` here would outrank their `Length` and squeeze them out.
            if pos == 0 {
                Constraint::Fill(1)
            } else {
                Constraint::Length(col_w[i] as u16)
            }
        })
        .collect();

    let mut table = Table::new(
        visible_rows(rows, inner, inner.height >= 2, app.diagnostics_offset),
        constraints,
    )
    .column_spacing(1);
    if inner.height >= 2 {
        let hdr_cells: Vec<&str> = col_indices.iter().map(|&i| headers[i]).collect();
        table = table.header(
            Row::new(hdr_cells).style(Style::default().fg(p.heading).add_modifier(Modifier::BOLD)),
        );
    }
    frame.render_widget(table, inner);
}

fn draw_gpus_table(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "GPUS", p);
        return;
    };
    let block = panel_block("GPUS", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let text_style = Style::default().fg(p.text);
    let muted = Style::default().fg(p.muted);
    let unknown = Style::default().fg(p.unknown);

    let list = match &snap.gpus {
        Reading::Unavailable(r) => {
            let line = Line::from(Span::styled(
                fmt::unavailable_str(r, inner.width as usize),
                unknown,
            ));
            frame.render_widget(Paragraph::new(line), inner);
            return;
        }
        Reading::Available(list) if list.is_empty() => {
            let line = Line::from(Span::styled("unavailable (no gpu)", unknown));
            frame.render_widget(Paragraph::new(line), inner);
            return;
        }
        Reading::Available(list) => list,
    };

    let mut raw_rows: Vec<([String; 6], [Style; 6])> = Vec::with_capacity(list.len());
    for g in list {
        if g.power_state == GpuPower::Suspended {
            let susp_style = Style::default().fg(p.muted).add_modifier(Modifier::BOLD);
            raw_rows.push((
                [
                    g.name.clone(),
                    g.pci_slot.clone(),
                    "SUSPENDED".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                ],
                [text_style, muted, susp_style, muted, muted, muted],
            ));
            continue;
        }

        let (util_s, util_st) = match &g.utilization_percent {
            Reading::Available(u) => (fmt::percent(*u as f64), text_style),
            Reading::Unavailable(r) => (fmt::unavailable_str(r, 11), unknown),
        };
        let (vram_s, vram_st) = match (&g.memory_used_bytes, &g.memory_total_bytes) {
            (Reading::Available(u), Reading::Available(t)) => (fmt::bytes_pair(*u, *t), text_style),
            (Reading::Unavailable(r), _) | (_, Reading::Unavailable(r)) => {
                (fmt::unavailable_str(r, 11), unknown)
            }
        };
        let (temp_s, temp_st) = match &g.temperature_celsius {
            Reading::Available(c) => (format!("{c:.0}°C"), text_style),
            Reading::Unavailable(r) => (fmt::unavailable_str(r, 11), unknown),
        };
        let (pwr_s, pwr_st) = match &g.power_watts {
            Reading::Available(w) => (format!("{w:.1}W"), text_style),
            Reading::Unavailable(r) => (fmt::unavailable_str(r, 11), unknown),
        };

        raw_rows.push((
            [
                g.name.clone(),
                g.pci_slot.clone(),
                util_s,
                vram_s,
                temp_s,
                pwr_s,
            ],
            [text_style, muted, util_st, vram_st, temp_st, pwr_st],
        ));
    }

    let headers = ["ADAPTER", "PCI", "UTIL", "VRAM", "TEMP", "PWR"];
    let mut col_w = [
        headers[0].len(),
        headers[1].len(),
        headers[2].len(),
        headers[3].len(),
        headers[4].len(),
        headers[5].len(),
    ];
    for (cells, _) in &raw_rows {
        for (i, c) in cells.iter().enumerate() {
            col_w[i] = col_w[i].max(c.chars().count());
        }
    }
    // Keep ADAPTER column wide enough for "NVIDIA GeForce RTX 4060 Laptop GPU" (34 chars) when width allows
    if inner.width >= 83 {
        col_w[0] = col_w[0].max(34);
    }

    let w = inner.width as usize;
    let candidates: &[&[usize]] = &[
        &[0, 1, 2, 3, 4, 5],
        &[0, 1, 2, 3, 4],
        &[0, 1, 2, 3],
        &[0, 1, 2],
        &[0, 2],
    ];
    let mut chosen: &[usize] = &[0, 2];
    for &cand in candidates {
        let sum_w: usize = cand.iter().map(|&i| col_w[i]).sum();
        let gaps = cand.len().saturating_sub(1);
        if sum_w + gaps <= w {
            chosen = cand;
            break;
        }
    }

    let rows: Vec<Row<'static>> = raw_rows
        .into_iter()
        .map(|(cells, styles)| {
            Row::new(
                chosen
                    .iter()
                    .map(|&i| Cell::from(Span::styled(cells[i].clone(), styles[i])))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    let constraints: Vec<Constraint> = chosen
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            // The identifier column takes what the measurements leave: a
            // `Min` here would outrank their `Length` and squeeze them out.
            if pos == 0 {
                Constraint::Fill(1)
            } else {
                Constraint::Length(col_w[i] as u16)
            }
        })
        .collect();

    let mut table = Table::new(
        visible_rows(rows, inner, inner.height >= 2, app.diagnostics_offset),
        constraints,
    )
    .column_spacing(1);
    if inner.height >= 2 {
        let hdr_cells: Vec<&str> = chosen.iter().map(|&i| headers[i]).collect();
        table = table.header(
            Row::new(hdr_cells).style(Style::default().fg(p.heading).add_modifier(Modifier::BOLD)),
        );
    }
    frame.render_widget(table, inner);
}

fn diag_battery_status(status: BatteryStatus) -> &'static str {
    match status {
        BatteryStatus::Charging => "charging",
        BatteryStatus::Discharging => "discharging",
        BatteryStatus::Full => "full",
        BatteryStatus::NotCharging => "not charging",
        BatteryStatus::Unknown => "unknown",
    }
}

fn draw_battery_table(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "BATTERY", p);
        return;
    };
    let block = panel_block("BATTERY", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let text_style = Style::default().fg(p.text);
    let muted = Style::default().fg(p.muted);
    let unknown = Style::default().fg(p.unknown);

    let b = match &snap.battery {
        Reading::Unavailable(r) => {
            let line = Line::from(Span::styled(
                fmt::unavailable_str(r, inner.width as usize),
                unknown,
            ));
            frame.render_widget(Paragraph::new(line), inner);
            return;
        }
        Reading::Available(b) => b,
    };

    let val_w = inner.width.saturating_sub(11) as usize;
    let status_span = Span::styled(diag_battery_status(b.status), text_style);
    let ac_span = match b.ac_online {
        Some(true) => Span::styled("connected", text_style),
        Some(false) => Span::styled("disconnected", text_style),
        None => fmt::unavailable_span(p, "not reported", val_w),
    };
    let pwr_span = match &b.power_watts {
        Reading::Available(w) => Span::styled(format!("{w:.1} W"), text_style),
        Reading::Unavailable(r) => fmt::unavailable_span(p, r, val_w),
    };
    let rem_span = match &b.time_remaining_secs {
        Reading::Available(s) => Span::styled(fmt::duration(*s), text_style),
        Reading::Unavailable(r) => fmt::unavailable_span(p, r, val_w),
    };
    let hlth_span = match &b.health_percent {
        Reading::Available(h) => Span::styled(fmt::percent(*h), text_style),
        Reading::Unavailable(r) => fmt::unavailable_span(p, r, val_w),
    };
    let lim_span = match b.charge_limit_percent {
        Some(l) => Span::styled(format!("{l}%"), text_style),
        None => fmt::unavailable_span(p, "not reported", val_w),
    };
    let cyc_span = match b.cycle_count {
        Some(c) => Span::styled(c.to_string(), text_style),
        None => fmt::unavailable_span(p, "not reported", val_w),
    };

    let rows = vec![
        Row::new(vec![
            Cell::from(Span::styled("CHARGE", muted)),
            Cell::from(Span::styled(
                format!("{} {:.1}%", b.name, b.percent),
                text_style,
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("STATUS", muted)),
            Cell::from(status_span),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("AC", muted)),
            Cell::from(ac_span),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("POWER", muted)),
            Cell::from(pwr_span),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("TIME LEFT", muted)),
            Cell::from(rem_span),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("HEALTH", muted)),
            Cell::from(hlth_span),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("LIMIT", muted)),
            Cell::from(lim_span),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("CYCLES", muted)),
            Cell::from(cyc_span),
        ]),
    ];

    let widths = [Constraint::Length(10), Constraint::Min(15)];
    let table = Table::new(
        visible_rows(rows, inner, false, app.diagnostics_offset),
        widths,
    )
    .column_spacing(1);
    frame.render_widget(table, inner);
}

fn draw_io_table(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let Some(snap) = &app.snapshot else {
        draw_acquiring(frame, area, "STORAGE & NETWORK", p);
        return;
    };
    let block = panel_block("STORAGE & NETWORK", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let text_style = Style::default().fg(p.text);
    let muted = Style::default().fg(p.muted);
    let unknown = Style::default().fg(p.unknown);

    let show_device = inner.width >= 56;
    let mut raw_rows: Vec<([String; 3], [Style; 3])> = Vec::new();

    match &snap.disks {
        Reading::Unavailable(r) => {
            raw_rows.push((
                [
                    "disks".to_string(),
                    fmt::unavailable_str(r, 24),
                    String::new(),
                ],
                [muted, unknown, muted],
            ));
        }
        Reading::Available(disks) => {
            for d in disks {
                let label = if show_device {
                    format!("{} ({})", d.mount_point, d.device)
                } else {
                    d.mount_point.clone()
                };
                let (rw_s, rw_st) =
                    directions(("R", &d.read_bps), ("W", &d.write_bps), text_style, unknown);
                raw_rows.push((
                    [label, fmt::bytes_pair(d.used_bytes(), d.total_bytes), rw_s],
                    [text_style, muted, rw_st],
                ));
            }
        }
    }

    match &snap.network {
        Reading::Unavailable(r) => {
            raw_rows.push((
                [
                    "net".to_string(),
                    fmt::unavailable_str(r, 24),
                    String::new(),
                ],
                [muted, unknown, muted],
            ));
        }
        Reading::Available(ifaces) => {
            for n in sorted_interfaces(ifaces) {
                let (rx_tx_s, rx_tx_st) =
                    directions(("RX", &n.rx_bps), ("TX", &n.tx_bps), text_style, unknown);
                raw_rows.push((
                    [
                        n.name.clone(),
                        format!(
                            "{}/{}",
                            fmt::reading_bytes(&n.rx_total_bytes, 11),
                            fmt::reading_bytes(&n.tx_total_bytes, 11)
                        ),
                        rx_tx_s,
                    ],
                    [text_style, muted, rx_tx_st],
                ));
            }
        }
    }

    let mut col_w = [1usize, 1usize, 1usize];
    for (cells, _) in &raw_rows {
        for (i, c) in cells.iter().enumerate() {
            col_w[i] = col_w[i].max(c.chars().count());
        }
    }

    let w = inner.width as usize;
    let include_rates = col_w[0] + col_w[1] + col_w[2] + 2 <= w;

    if include_rates {
        let rows: Vec<Row<'static>> = raw_rows
            .into_iter()
            .map(|(cells, styles)| {
                Row::new(vec![
                    Cell::from(Span::styled(cells[0].clone(), styles[0])),
                    Cell::from(Span::styled(cells[1].clone(), styles[1])),
                    Cell::from(Span::styled(cells[2].clone(), styles[2])),
                ])
            })
            .collect();
        let widths = [
            Constraint::Fill(1),
            Constraint::Length(col_w[1] as u16),
            Constraint::Length(col_w[2] as u16),
        ];
        frame.render_widget(
            Table::new(
                visible_rows(rows, inner, false, app.diagnostics_offset),
                widths,
            )
            .column_spacing(1),
            inner,
        );
    } else {
        let rows: Vec<Row<'static>> = raw_rows
            .into_iter()
            .map(|(cells, styles)| {
                Row::new(vec![
                    Cell::from(Span::styled(cells[0].clone(), styles[0])),
                    Cell::from(Span::styled(cells[1].clone(), styles[1])),
                ])
            })
            .collect();
        let widths = [Constraint::Fill(1), Constraint::Length(col_w[1] as u16)];
        frame.render_widget(
            Table::new(
                visible_rows(rows, inner, false, app.diagnostics_offset),
                widths,
            )
            .column_spacing(1),
            inner,
        );
    }
}

/// The two rates of one device, each shown on its own, so a direction that
/// is unavailable does not hide the other's reading. Both unavailable show
/// the first reason, as one reading.
fn directions(
    first: (&str, &Reading<f64>),
    second: (&str, &Reading<f64>),
    text_style: Style,
    unknown: Style,
) -> (String, Style) {
    if let (Reading::Unavailable(reason), Reading::Unavailable(_)) = (first.1, second.1) {
        return (fmt::unavailable_str(reason, 20), unknown);
    }
    let part = |(label, reading): (&str, &Reading<f64>)| match reading {
        Reading::Available(v) => format!("{label} {}", fmt::rate(*v)),
        Reading::Unavailable(_) => format!("{label} unavailable"),
    };
    (format!("{} {}", part(first), part(second)), text_style)
}

fn draw_rules_table(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let block = panel_block("ALERT RULES", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let fallback_rules: Vec<RuleStatus>;
    let rules_slice: &[RuleStatus] = if app.rules.is_empty() {
        let cfg = &app.config.alerts;
        fallback_rules = RuleId::ALL
            .iter()
            .zip(cfg.rules())
            .map(|(&id, (_, rc, _))| RuleStatus {
                rule: id,
                subject: String::new(),
                value: None,
                level: Level::Unknown,
                warn: rc.warn,
                crit: rc.crit,
                note: Some("acquiring".into()),
            })
            .collect();
        &fallback_rules
    } else {
        &app.rules
    };

    let text_style = Style::default().fg(p.text);
    let muted = Style::default().fg(p.muted);
    let unknown = Style::default().fg(p.unknown);

    // VALUE is as wide as its widest value ("unavailable", "0.09/core"),
    // so a reading is never clipped. When the table is too narrow for every
    // column, WARN and CRIT go first and then LABEL; RULE keeps at least
    // 12 cells. Each row fits its rule name on its own, so one long subject
    // (a GPU name) never hides the subjects of the other rows.
    let values: Vec<Span<'static>> = rules_slice
        .iter()
        .map(|r| match r.value {
            Some(v) => Span::styled(format_rule_value(r.rule, v), text_style),
            None => {
                let reason = r.note.as_deref().unwrap_or("no data");
                Span::styled(fmt::unavailable_str(reason, 11), unknown)
            }
        })
        .collect();
    let value_w = values
        .iter()
        .map(|v| fmt::display_width(&v.content))
        .max()
        .unwrap_or(0)
        .max("VALUE".len());
    let level_w = rules_slice
        .iter()
        .map(|r| r.level.label().len())
        .max()
        .unwrap_or(7)
        .max("LEVEL".len());
    // Cells after RULE, each with the space before it.
    let after_rule = |label: bool, limits: bool| {
        (value_w + 1) + (level_w + 1) + if label { 11 } else { 0 } + if limits { 10 } else { 0 }
    };
    let width = inner.width as usize;
    let fits = |label: bool, limits: bool| 12 + after_rule(label, limits) <= width;
    let (show_label, show_limits) = if fits(true, true) {
        (true, true)
    } else if fits(true, false) {
        (true, false)
    } else {
        (false, false)
    };
    let rule_w = width.saturating_sub(after_rule(show_label, show_limits));

    let rows: Vec<Row<'static>> = rules_slice
        .iter()
        .zip(values)
        .map(|(r, value)| {
            let lvl_color = fmt::level_color(p, r.level);
            let mut cells = vec![Cell::from(Span::styled(
                fit_rule_name(r.rule.key(), &r.subject, rule_w),
                text_style,
            ))];
            if show_label {
                cells.push(Cell::from(Span::styled(r.rule.label(), muted)));
            }
            cells.push(Cell::from(value));
            if show_limits {
                cells.push(Cell::from(Span::styled(format!("{:.1}", r.warn), muted)));
                cells.push(Cell::from(Span::styled(format!("{:.1}", r.crit), muted)));
            }
            cells.push(Cell::from(Span::styled(
                r.level.label(),
                Style::default().fg(lvl_color).add_modifier(Modifier::BOLD),
            )));
            Row::new(cells)
        })
        .collect();

    let mut widths = vec![Constraint::Min(12)];
    let mut header = vec!["RULE"];
    if show_label {
        widths.push(Constraint::Length(10));
        header.push("LABEL");
    }
    widths.push(Constraint::Length(value_w as u16));
    header.push("VALUE");
    if show_limits {
        widths.extend([Constraint::Length(4), Constraint::Length(4)]);
        header.extend(["WARN", "CRIT"]);
    }
    widths.push(Constraint::Length(level_w as u16));
    header.push("LEVEL");

    let mut table = Table::new(
        visible_rows(
            rows,
            inner,
            inner.height as usize > rules_slice.len(),
            app.diagnostics_offset,
        ),
        widths,
    )
    .column_spacing(1);
    if inner.height as usize > rules_slice.len() {
        table = table.header(
            Row::new(header).style(Style::default().fg(p.heading).add_modifier(Modifier::BOLD)),
        );
    }
    frame.render_widget(table, inner);
}

fn draw_events_table(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let block = panel_block("ALERT LOG", p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let muted = Style::default().fg(p.muted);
    let text_style = Style::default().fg(p.text);

    let widths = [
        Constraint::Length(8),
        Constraint::Min(10),
        Constraint::Length(20),
        Constraint::Length(8),
    ];

    if app.log.is_empty() {
        if inner.height >= 2 {
            let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
            let hdr_table = Table::new(Vec::<Row<'static>>::new(), widths)
                .column_spacing(1)
                .header(
                    Row::new(vec!["TIME", "RULE", "TRANSITION", "VALUE"])
                        .style(Style::default().fg(p.heading).add_modifier(Modifier::BOLD)),
                );
            frame.render_widget(hdr_table, parts[0]);
            let empty_msg = Paragraph::new(Line::from(Span::styled(
                "no level changes since start",
                muted,
            )));
            frame.render_widget(empty_msg, parts[1]);
        } else {
            let empty_msg = Paragraph::new(Line::from(Span::styled(
                "no level changes since start",
                muted,
            )));
            frame.render_widget(empty_msg, inner);
        }
        return;
    }

    let rows: Vec<Row<'static>> = app
        .log
        .iter()
        .rev()
        .map(|ev| {
            let subj = if ev.subject.is_empty() {
                ev.rule.key().to_string()
            } else {
                format!("{} {}", ev.rule.key(), ev.subject)
            };
            let to_color = fmt::level_color(p, ev.to);
            let val_str = ev
                .value
                .map(|v| format_rule_value(ev.rule, v))
                .unwrap_or_else(|| "-".to_string());
            let clock_str = crate::clock::hms(ev.at_ms, app.utc_offset_secs.unwrap_or(0));
            Row::new(vec![
                Cell::from(Span::styled(clock_str, muted)),
                Cell::from(Span::styled(subj, text_style)),
                Cell::from(Span::styled(
                    format!("{} -> {}", ev.from.label(), ev.to.label()),
                    Style::default().fg(to_color).add_modifier(Modifier::BOLD),
                )),
                Cell::from(Span::styled(val_str, text_style)),
            ])
        })
        .collect();

    let mut table = Table::new(
        visible_rows(rows, inner, inner.height >= 4, app.diagnostics_offset),
        widths,
    )
    .column_spacing(1);
    if inner.height >= 4 {
        table = table.header(
            Row::new(vec!["TIME", "RULE", "TRANSITION", "VALUE"])
                .style(Style::default().fg(p.heading).add_modifier(Modifier::BOLD)),
        );
    }
    frame.render_widget(table, inner);
}
/// "disk /boot", or the key and as much of the subject as fits with an
/// ellipsis, or the bare key when not even that fits.
fn fit_rule_name(key: &str, subject: &str, width: usize) -> String {
    if subject.is_empty() {
        return key.to_owned();
    }
    let full = format!("{key} {subject}");
    if full.chars().count() <= width {
        return full;
    }
    let room = width.saturating_sub(key.chars().count() + 2);
    if room == 0 {
        return key.to_owned();
    }
    let cut: String = subject.chars().take(room).collect();
    format!("{key} {cut}\u{2026}")
}

#[cfg(test)]
mod tests {
    use crate::app::Screen;
    use crate::model::Reading;
    use crate::ui::testkit::{buffer_lines, nominal_app, render};

    /// A 40-column identifier: longer than the whole 29-column interior of
    /// the left-hand tables at 80x24.
    const LONG_ID: &str = "thinkpad_acpi_extended_sensor:cpu_pkg_01";

    /// The left-hand column (31 cells at 80 columns) of the row that holds `needle`.
    fn left_row(lines: &[String], needle: &str) -> String {
        let row = lines
            .iter()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("a row containing {needle:?}:\n{}", lines.join("\n")));
        row.chars().take(31).collect()
    }

    #[test]
    fn long_sensor_identifier_keeps_its_value_at_80x24() {
        assert_eq!(LONG_ID.chars().count(), 40);
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        if let Some(Reading::Available(s)) = app.snapshot.as_mut().map(|s| &mut s.sensors) {
            let (chip, label) = LONG_ID.split_once(':').unwrap();
            s.temps[0].chip = chip.into();
            s.temps[0].label = label.into();
        }
        let lines = buffer_lines(&render(&app, 80, 24));
        let row = left_row(&lines, "thinkpad_acpi");
        assert!(row.contains("92.0°C"), "value column lost: {row:?}");
    }

    #[test]
    fn long_adapter_name_keeps_its_utilization_at_80x24() {
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        if let Some(Reading::Available(g)) = app.snapshot.as_mut().map(|s| &mut s.gpus) {
            g[0].name = "AMD Radeon 780M Graphics Integrated Adapter".into();
        }
        let lines = buffer_lines(&render(&app, 80, 24));
        let row = left_row(&lines, "AMD Radeon");
        assert!(row.contains("34.0%"), "utilization column lost: {row:?}");
    }

    #[test]
    fn long_mount_point_keeps_its_capacity_at_80x24() {
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        if let Some(Reading::Available(d)) = app.snapshot.as_mut().map(|s| &mut s.disks) {
            d[0].mount_point = "/run/media/someone/ExternalBackupDriveXY".into();
        }
        let lines = buffer_lines(&render(&app, 80, 24));
        let row = left_row(&lines, "/run/m");
        assert!(
            row.contains("280.0 GiB/500.0 GiB"),
            "capacity column lost: {row:?}"
        );
    }
    /// A rule's value is never clipped: VALUE is as wide as its widest
    /// value, and a narrow table drops optional columns instead.
    #[test]
    fn rule_values_are_shown_whole() {
        use crate::alert::RuleId;
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        let load = app
            .rules
            .iter_mut()
            .find(|r| r.rule == RuleId::Load)
            .unwrap();
        load.value = Some(0.09);
        load.subject = "loadprobe".into();
        let missing = app
            .rules
            .iter_mut()
            .find(|r| r.rule == RuleId::Memory)
            .unwrap();
        missing.value = None;
        missing.subject = "memprobe".into();
        for (width, height) in [(200, 60), (120, 40), (80, 24)] {
            let lines = buffer_lines(&render(&app, width, height));
            for (subject, value) in [("loadprobe", "0.09/core"), ("memprobe", "unavailable")] {
                if let Some(row) = lines.iter().find(|l| l.contains(subject)) {
                    assert!(row.contains(value), "{width}x{height}: {row:?}");
                }
            }
        }
        // At the widest size both rows are on screen.
        let wide = buffer_lines(&render(&app, 200, 60)).join("\n");
        assert!(wide.contains("loadprobe") && wide.contains("memprobe"));
    }

    /// One unavailable direction does not hide the other's reading.
    #[test]
    fn each_available_io_direction_stays_visible() {
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        let snapshot = app.snapshot.as_mut().unwrap();
        let Reading::Available(nets) = &mut snapshot.network else {
            unreachable!()
        };
        nets[0].rx_bps = Reading::Available(2048.0);
        nets[0].tx_bps = Reading::unavailable("counter not readable");
        let Reading::Available(disks) = &mut snapshot.disks else {
            unreachable!()
        };
        disks[0].read_bps = Reading::unavailable("warming up");
        disks[0].write_bps = Reading::Available(4096.0);
        let text = buffer_lines(&render(&app, 200, 60)).join("\n");
        let rx = format!("RX {} TX unavailable", crate::ui::fmt::rate(2048.0));
        let w = format!("R unavailable W {}", crate::ui::fmt::rate(4096.0));
        assert!(text.contains(&rx), "{rx} missing\n{text}");
        assert!(text.contains(&w), "{w} missing\n{text}");
    }

    /// Nominal app on the diagnostics screen with `count` temperature rows.
    fn app_with_sensors(count: usize) -> crate::app::App {
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        let snapshot = app.snapshot.as_mut().unwrap();
        let Reading::Available(sensors) = &mut snapshot.sensors else {
            unreachable!()
        };
        let template = sensors.temps[0].clone();
        sensors.temps = (0..count)
            .map(|i| crate::model::TempSensor {
                chip: "probe".into(),
                label: format!("{i:02}"),
                ..template.clone()
            })
            .collect();
        sensors.fans.clear();
        app
    }

    fn key(app: &mut crate::app::App, code: crossterm::event::KeyCode) {
        use crossterm::event::{KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// A page moves the shared offset by no more than the smallest table
    /// that scrolls can show, so no row of any table is skipped; the process
    /// page size (16 rows at 80x24) is not a diagnostics page.
    #[test]
    fn page_keys_move_diagnostics_without_skipping_rows() {
        use crossterm::event::KeyCode;
        let mut app = app_with_sensors(40);
        let shown = buffer_lines(&crate::screenshot::render(&mut app, 80, 24))
            .iter()
            .filter(|line| line.contains("probe:"))
            .count();
        assert!(shown >= 1);
        key(&mut app, KeyCode::PageDown);
        assert!(
            (1..=shown).contains(&app.diagnostics_offset),
            "moved {} with {shown} sensor rows shown",
            app.diagnostics_offset
        );
        let after = app.diagnostics_offset;
        key(&mut app, KeyCode::PageDown);
        assert_eq!(app.diagnostics_offset, after * 2);
        key(&mut app, KeyCode::PageUp);
        assert_eq!(app.diagnostics_offset, after);
    }

    /// A frame too small to draw any table does not reset the saved
    /// position; it is clamped again once the tables can be drawn.
    #[test]
    fn a_too_small_frame_keeps_the_diagnostics_position() {
        use crossterm::event::KeyCode;
        let mut app = app_with_sensors(40);
        crate::screenshot::render(&mut app, 80, 24);
        for _ in 0..5 {
            key(&mut app, KeyCode::Down);
        }
        assert_eq!(app.diagnostics_offset, 5);
        crate::screenshot::render(&mut app, 20, 5);
        assert_eq!(app.diagnostics_offset, 5);
        crate::screenshot::render(&mut app, 80, 24);
        assert_eq!(app.diagnostics_offset, 5);
        // Once the tables fit, the position is clamped as before.
        crate::screenshot::render(&mut app, 20, 5);
        app.apply_snapshot(nominal_app().snapshot.unwrap());
        crate::screenshot::render(&mut app, 200, 60);
        assert_eq!(app.diagnostics_offset, 0);
    }

    #[test]
    fn diagnostics_bottom_tracks_resizes_and_shorter_inventories() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        let snapshot = app.snapshot.as_mut().unwrap();
        let sensors = match &mut snapshot.sensors {
            Reading::Available(sensors) => sensors,
            _ => unreachable!(),
        };
        let template = sensors.temps[0].clone();
        sensors.temps = (0..100)
            .map(|i| crate::model::TempSensor {
                chip: "probe".into(),
                label: format!("{i:02}"),
                ..template.clone()
            })
            .collect();
        sensors.fans.clear();

        let mut small_offset = 0;
        for (width, height) in [(80, 24), (160, 48), (200, 60), (80, 24)] {
            crate::screenshot::render(&mut app, width, height);
            app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
            let bottom = crate::screenshot::render(&mut app, width, height);
            assert!(buffer_lines(&bottom).join("\n").contains("probe:99"));
            if width == 80 {
                small_offset = app.diagnostics_offset;
            } else {
                assert!(app.diagnostics_offset < small_offset);
            }
            app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
            let previous = crate::screenshot::render(&mut app, width, height);
            assert_ne!(
                left_row(&buffer_lines(&bottom), "probe:"),
                left_row(&buffer_lines(&previous), "probe:")
            );
        }

        let shorter = nominal_app().snapshot.unwrap();
        app.apply_snapshot(shorter);
        crate::screenshot::render(&mut app, 200, 60);
        assert_eq!(app.diagnostics_offset, 0, "all tables now fit");
    }

    #[test]
    fn diagnostics_navigation_reaches_overflow_rows_and_preserves_process_selection() {
        use crate::model::TempSensor;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        let selected_pid = app.procs.selected_pid;
        let snap = app.snapshot.as_mut().unwrap();
        let sensors = match &mut snap.sensors {
            Reading::Available(s) => s,
            _ => unreachable!(),
        };
        let template: TempSensor = sensors.temps[0].clone();
        sensors.temps = (0..15)
            .map(|i| TempSensor {
                chip: "probe".into(),
                label: format!("{i:02}"),
                ..template.clone()
            })
            .collect();
        sensors.fans.clear();
        let disk_template = snap.disks.value().unwrap()[0].clone();
        snap.disks = Reading::Available(
            (0..6)
                .map(|i| crate::model::DiskStats {
                    mount_point: format!("/disk{i}"),
                    ..disk_template.clone()
                })
                .collect(),
        );
        let initial = buffer_lines(&crate::screenshot::render(&mut app, 80, 24)).join("\n");
        assert!(initial.contains("probe:00"));
        assert!(!initial.contains("probe:14"));
        let mut seen = initial;
        for _ in 0..25 {
            app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
            seen.push_str(&buffer_lines(&crate::screenshot::render(&mut app, 80, 24)).join("\n"));
        }
        for i in 0..15 {
            assert!(
                seen.contains(&format!("probe:{i:02}")),
                "sensor {i} unreachable"
            );
        }
        for i in 0..6 {
            assert!(seen.contains(&format!("/disk{i}")), "disk {i} unreachable");
        }
        assert!(
            seen.contains("wlan0"),
            "network rows unreachable after disks"
        );
        assert!(seen.contains("CYCLES"), "battery rows unreachable");
        assert_eq!(app.procs.selected_pid, selected_pid);
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert!(
            buffer_lines(&crate::screenshot::render(&mut app, 80, 24))
                .join("\n")
                .contains("probe:00")
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
        assert!(
            buffer_lines(&crate::screenshot::render(&mut app, 80, 24))
                .join("\n")
                .contains("probe:14")
        );
        let bottom = buffer_lines(&crate::screenshot::render(&mut app, 80, 24));
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let previous = buffer_lines(&crate::screenshot::render(&mut app, 80, 24));
        assert_ne!(left_row(&bottom, "probe:"), left_row(&previous, "probe:"));
    }
}
