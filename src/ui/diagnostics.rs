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

/// Draws the DIAGNOSTICS screen using `Table` widgets for all subsystems.
pub fn draw_diagnostics(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }

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

    draw_sensors_table(frame, left_rows[0], app);
    draw_gpus_table(frame, left_rows[1], app);
    draw_battery_table(frame, left_rows[2], app);
    draw_io_table(frame, left_rows[3], app);
    draw_rules_table(frame, right_rows[0], app);
    draw_events_table(frame, right_rows[1], app);
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

    let mut table = Table::new(rows, constraints).column_spacing(1);
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

    let mut table = Table::new(rows, constraints).column_spacing(1);
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
    let table = Table::new(rows, widths).column_spacing(1);
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
                let (rw_s, rw_st) = match (&d.read_bps, &d.write_bps) {
                    (Reading::Available(r), Reading::Available(w)) => (
                        format!("R {} W {}", fmt::rate(*r), fmt::rate(*w)),
                        text_style,
                    ),
                    (Reading::Unavailable(r), _) | (_, Reading::Unavailable(r)) => {
                        (fmt::unavailable_str(r, 20), unknown)
                    }
                };
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
                let (rx_tx_s, rx_tx_st) = match (&n.rx_bps, &n.tx_bps) {
                    (Reading::Available(rx), Reading::Available(tx)) => (
                        format!("RX {} TX {}", fmt::rate(*rx), fmt::rate(*tx)),
                        text_style,
                    ),
                    (Reading::Unavailable(r), _) | (_, Reading::Unavailable(r)) => {
                        (fmt::unavailable_str(r, 20), unknown)
                    }
                };
                raw_rows.push((
                    [
                        n.name.clone(),
                        format!(
                            "{}/{}",
                            fmt::bytes(n.rx_total_bytes),
                            fmt::bytes(n.tx_total_bytes)
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
        frame.render_widget(Table::new(rows, widths).column_spacing(1), inner);
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
        frame.render_widget(Table::new(rows, widths).column_spacing(1), inner);
    }
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

    // Other 5 columns take 10 + 6 + 4 + 4 + 8 + 5 spaces = 37 columns; the
    // rule column gets the rest. Each row fits on its own, so one long
    // subject (a GPU name) never hides the subjects of the other rows.
    let rule_w = (inner.width as usize).saturating_sub(37);

    let rows: Vec<Row<'static>> = rules_slice
        .iter()
        .map(|r| {
            let key_subj = fit_rule_name(r.rule.key(), &r.subject, rule_w);
            let val_span = match r.value {
                Some(v) => Span::styled(format_rule_value(r.rule, v), text_style),
                None => {
                    let reason = r.note.as_deref().unwrap_or("no data");
                    Span::styled(fmt::unavailable_str(reason, 11), unknown)
                }
            };
            let lvl_color = fmt::level_color(p, r.level);
            Row::new(vec![
                Cell::from(Span::styled(key_subj, text_style)),
                Cell::from(Span::styled(r.rule.label(), muted)),
                Cell::from(val_span),
                Cell::from(Span::styled(format!("{:.1}", r.warn), muted)),
                Cell::from(Span::styled(format!("{:.1}", r.crit), muted)),
                Cell::from(Span::styled(
                    r.level.label(),
                    Style::default().fg(lvl_color).add_modifier(Modifier::BOLD),
                )),
            ])
        })
        .collect();

    let max_level_len = rules_slice
        .iter()
        .map(|r| r.level.label().len())
        .max()
        .unwrap_or(7)
        .max(5) as u16;

    let widths = [
        Constraint::Min(12),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(max_level_len),
    ];

    let mut table = Table::new(rows, widths).column_spacing(1);
    if inner.height as usize > rules_slice.len() {
        table = table.header(
            Row::new(vec!["RULE", "LABEL", "VALUE", "WARN", "CRIT", "LEVEL"])
                .style(Style::default().fg(p.heading).add_modifier(Modifier::BOLD)),
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

    let mut table = Table::new(rows, widths).column_spacing(1);
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
}
