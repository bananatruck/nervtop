//! DIAGNOSTICS screen: full inspection tables for sensors, fans, GPUs, battery, disks, network, rules and alert log.

use crate::alert::{Level, RuleId, RuleStatus};
use crate::app::App;
use crate::model::{GpuPower, Reading, SensorKind};
use crate::ui::fmt;
use crate::ui::header::{draw_acquiring, panel_block};
use crate::ui::power::status_label;
use crate::ui::stripes::format_rule_value;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Cell, Row, Table};

/// Draws the DIAGNOSTICS screen using `Table` widgets for all subsystems.
pub fn draw_diagnostics(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let cols = if area.width < 100 {
        Layout::horizontal([Constraint::Length(31), Constraint::Min(49)]).split(area)
    } else {
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).split(area)
    };

    let left_rows = if cols[0].height <= 22 {
        Layout::vertical([
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(4),
        ])
        .split(cols[0])
    } else {
        Layout::vertical([
            Constraint::Percentage(30),
            Constraint::Percentage(24),
            Constraint::Percentage(22),
            Constraint::Percentage(24),
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

    let rows: Vec<Row<'static>> = match &snap.sensors {
        Reading::Unavailable(r) => vec![Row::new(vec![
            Cell::from(Span::styled(
                fmt::unavailable_str(r, inner.width as usize),
                unknown,
            )),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
        ])],
        Reading::Available(s) => {
            let mut out = Vec::new();
            for t in &s.temps {
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
                out.push(Row::new(vec![
                    Cell::from(Span::styled(format!("{}:{}", t.chip, t.label), text_style)),
                    Cell::from(Span::styled(kind_str, muted)),
                    Cell::from(Span::styled(format!("{:.1}°C", t.celsius), text_style)),
                    Cell::from(Span::styled(crit_str, muted)),
                ]));
            }
            for f in &s.fans {
                out.push(Row::new(vec![
                    Cell::from(Span::styled(format!("{}:{}", f.chip, f.label), text_style)),
                    Cell::from(Span::styled("fan", muted)),
                    Cell::from(Span::styled(format!("{} RPM", f.rpm), text_style)),
                    Cell::from(Span::styled("-", muted)),
                ]));
            }
            if out.is_empty() {
                out.push(Row::new(vec![
                    Cell::from(Span::styled("unavailable (no sensors)", unknown)),
                    Cell::from(""),
                    Cell::from(""),
                    Cell::from(""),
                ]));
            }
            out
        }
    };

    let widths = [
        Constraint::Min(12),
        Constraint::Length(7),
        Constraint::Length(9),
        Constraint::Length(11),
    ];
    let mut table = Table::new(rows, widths).column_spacing(1);
    if inner.height >= 4 {
        table = table.header(
            Row::new(vec!["SENSOR", "KIND", "VALUE", "LIMIT"])
                .style(Style::default().fg(p.heading).add_modifier(Modifier::BOLD)),
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

    let rows: Vec<Row<'static>> = match &snap.gpus {
        Reading::Unavailable(r) => vec![Row::new(vec![
            Cell::from(Span::styled(
                fmt::unavailable_str(r, inner.width as usize),
                unknown,
            )),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
        ])],
        Reading::Available(list) if list.is_empty() => vec![Row::new(vec![
            Cell::from(Span::styled("unavailable (no gpu)", unknown)),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
        ])],
        Reading::Available(list) => list
            .iter()
            .map(|g| {
                if g.power_state == GpuPower::Suspended {
                    return Row::new(vec![
                        Cell::from(Span::styled(
                            format!("{} {}", g.pci_slot, g.name),
                            text_style,
                        )),
                        Cell::from(Span::styled(
                            "SUSPENDED",
                            Style::default().fg(p.muted).add_modifier(Modifier::BOLD),
                        )),
                        Cell::from(Span::styled("-", muted)),
                        Cell::from(Span::styled("-", muted)),
                        Cell::from(Span::styled("-", muted)),
                    ]);
                }
                let util_cell = match &g.utilization_percent {
                    Reading::Available(u) => {
                        Cell::from(Span::styled(fmt::percent(*u as f64), text_style))
                    }
                    Reading::Unavailable(r) => {
                        Cell::from(Span::styled(fmt::unavailable_str(r, 11), unknown))
                    }
                };
                let vram_cell = match (&g.memory_used_bytes, &g.memory_total_bytes) {
                    (Reading::Available(u), Reading::Available(t)) => {
                        Cell::from(Span::styled(fmt::bytes_pair(*u, *t), text_style))
                    }
                    (Reading::Unavailable(r), _) | (_, Reading::Unavailable(r)) => {
                        Cell::from(Span::styled(fmt::unavailable_str(r, 11), unknown))
                    }
                };
                let temp_cell = match &g.temperature_celsius {
                    Reading::Available(c) => {
                        Cell::from(Span::styled(format!("{c:.0}°C"), text_style))
                    }
                    Reading::Unavailable(r) => {
                        Cell::from(Span::styled(fmt::unavailable_str(r, 11), unknown))
                    }
                };
                let pwr_cell = match &g.power_watts {
                    Reading::Available(w) => {
                        Cell::from(Span::styled(format!("{w:.1}W"), text_style))
                    }
                    Reading::Unavailable(r) => {
                        Cell::from(Span::styled(fmt::unavailable_str(r, 11), unknown))
                    }
                };
                Row::new(vec![
                    Cell::from(Span::styled(
                        format!("{} {}", g.pci_slot, g.name),
                        text_style,
                    )),
                    util_cell,
                    vram_cell,
                    temp_cell,
                    pwr_cell,
                ])
            })
            .collect(),
    };

    let widths = if inner.width < 48 {
        [
            Constraint::Min(9),
            Constraint::Length(11),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    } else {
        [
            Constraint::Min(10),
            Constraint::Length(11),
            Constraint::Length(15),
            Constraint::Length(7),
            Constraint::Length(7),
        ]
    };
    let mut table = Table::new(rows, widths).column_spacing(1);
    if inner.height >= 4 {
        table = table.header(
            Row::new(vec!["ADAPTER", "UTIL", "VRAM", "TEMP", "PWR"])
                .style(Style::default().fg(p.heading).add_modifier(Modifier::BOLD)),
        );
    }
    frame.render_widget(table, inner);
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

    let rows: Vec<Row<'static>> = match &snap.battery {
        Reading::Unavailable(r) => vec![Row::new(vec![
            Cell::from(Span::styled(
                fmt::unavailable_str(r, inner.width as usize),
                unknown,
            )),
            Cell::from(""),
        ])],
        Reading::Available(b) => {
            let ac_str = match b.ac_online {
                Some(true) => "online",
                Some(false) => "offline",
                None => "unknown",
            };
            let pwr_cell = match &b.power_watts {
                Reading::Available(w) => Span::styled(format!("{w:.1} W"), text_style),
                Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, 18), unknown),
            };
            let rem_cell = match &b.time_remaining_secs {
                Reading::Available(s) => Span::styled(fmt::duration(*s), text_style),
                Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, 18), unknown),
            };
            let hlth_cell = match &b.health_percent {
                Reading::Available(h) => Span::styled(fmt::percent(*h), text_style),
                Reading::Unavailable(r) => Span::styled(fmt::unavailable_str(r, 18), unknown),
            };
            let lim_str = b
                .charge_limit_percent
                .map(|l| format!("{l}%"))
                .unwrap_or_else(|| "-".to_string());
            let cyc_str = b
                .cycle_count
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".to_string());

            vec![
                Row::new(vec![
                    Cell::from(Span::styled(
                        format!(
                            "{} {} {} (AC {})",
                            b.name,
                            fmt::percent(b.percent),
                            status_label(b.status),
                            ac_str
                        ),
                        text_style,
                    )),
                    Cell::from(pwr_cell),
                ]),
                Row::new(vec![
                    Cell::from(Span::styled(
                        format!("limit {lim_str}  cycles {cyc_str}"),
                        muted,
                    )),
                    Cell::from(rem_cell),
                ]),
                Row::new(vec![
                    Cell::from(Span::styled("health", muted)),
                    Cell::from(hlth_cell),
                ]),
            ]
        }
    };

    let widths = [Constraint::Min(18), Constraint::Length(16)];
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
    let mut rows: Vec<Row<'static>> = Vec::new();

    match &snap.disks {
        Reading::Unavailable(r) => {
            rows.push(Row::new(vec![
                Cell::from(Span::styled("disks", muted)),
                Cell::from(Span::styled(fmt::unavailable_str(r, 20), unknown)),
                Cell::from(""),
            ]));
        }
        Reading::Available(disks) => {
            for d in disks {
                let rw = match (&d.read_bps, &d.write_bps) {
                    (Reading::Available(r), Reading::Available(w)) => Span::styled(
                        format!("R {} W {}", fmt::rate(*r), fmt::rate(*w)),
                        text_style,
                    ),
                    (Reading::Unavailable(r), _) | (_, Reading::Unavailable(r)) => {
                        Span::styled(fmt::unavailable_str(r, 16), unknown)
                    }
                };
                rows.push(Row::new(vec![
                    Cell::from(Span::styled(
                        format!("{} ({})", d.mount_point, d.device),
                        text_style,
                    )),
                    Cell::from(Span::styled(
                        fmt::bytes_pair(d.used_bytes(), d.total_bytes),
                        muted,
                    )),
                    Cell::from(rw),
                ]));
            }
        }
    }

    match &snap.network {
        Reading::Unavailable(r) => {
            rows.push(Row::new(vec![
                Cell::from(Span::styled("net", muted)),
                Cell::from(Span::styled(fmt::unavailable_str(r, 20), unknown)),
                Cell::from(""),
            ]));
        }
        Reading::Available(ifaces) => {
            for n in ifaces {
                let rx_tx = match (&n.rx_bps, &n.tx_bps) {
                    (Reading::Available(rx), Reading::Available(tx)) => Span::styled(
                        format!("RX {} TX {}", fmt::rate(*rx), fmt::rate(*tx)),
                        text_style,
                    ),
                    (Reading::Unavailable(r), _) | (_, Reading::Unavailable(r)) => {
                        Span::styled(fmt::unavailable_str(r, 16), unknown)
                    }
                };
                rows.push(Row::new(vec![
                    Cell::from(Span::styled(n.name.clone(), text_style)),
                    Cell::from(Span::styled(
                        format!(
                            "{}/{}",
                            fmt::bytes(n.rx_total_bytes),
                            fmt::bytes(n.tx_total_bytes)
                        ),
                        muted,
                    )),
                    Cell::from(rx_tx),
                ]));
            }
        }
    }

    let widths = [
        Constraint::Min(10),
        Constraint::Length(15),
        Constraint::Min(14),
    ];
    let table = Table::new(rows, widths).column_spacing(1);
    frame.render_widget(table, inner);
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

    let rows: Vec<Row<'static>> = rules_slice
        .iter()
        .map(|r| {
            let key_subj = if r.subject.is_empty() || inner.width < 56 {
                r.rule.key().to_string()
            } else {
                format!("{} {}", r.rule.key(), r.subject)
            };
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

    let widths = [
        Constraint::Min(12),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(8),
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

    let rows: Vec<Row<'static>> = if app.log.is_empty() {
        vec![Row::new(vec![
            Cell::from(Span::styled("no alert transitions recorded", muted)),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
        ])]
    } else {
        app.log
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
                Row::new(vec![
                    Cell::from(Span::styled(fmt::hms_utc(ev.at_ms), muted)),
                    Cell::from(Span::styled(subj, text_style)),
                    Cell::from(Span::styled(
                        format!("{} -> {}", ev.from.label(), ev.to.label()),
                        Style::default().fg(to_color).add_modifier(Modifier::BOLD),
                    )),
                    Cell::from(Span::styled(val_str, text_style)),
                ])
            })
            .collect()
    };

    let widths = [
        Constraint::Length(8),
        Constraint::Min(10),
        Constraint::Length(18),
        Constraint::Length(8),
    ];
    let mut table = Table::new(rows, widths).column_spacing(1);
    if inner.height >= 4 {
        table = table.header(
            Row::new(vec!["TIME", "RULE", "TRANSITION", "VALUE"])
                .style(Style::default().fg(p.heading).add_modifier(Modifier::BOLD)),
        );
    }
    frame.render_widget(table, inner);
}
