//! One-shot sampling and complete plain text or JSON reports.
use crate::{
    alert::{AlertEngine, Evaluation, format_rule_value},
    clock,
    collect::Collector,
    config::AlertConfig,
    format::*,
    model::*,
    redact::Redactor,
};
use std::{fmt::Write, thread, time::Duration};

/// Processes listed in the text report, busiest first.
pub const REPORT_PROCESSES: usize = 15;

/// Takes a warm-up sample, a baseline sample 250 ms later, and the reported
/// sample one full `interval_ms` after the baseline, and judges it.
///
/// A one-shot report has no history, so alert levels are instantaneous:
/// `sustain` is treated as 1 here. The live monitor applies the configured
/// sustain.
pub fn once(
    collector: &mut Collector,
    interval_ms: u64,
    config: &AlertConfig,
    redactor: Option<&mut Redactor>,
) -> (Snapshot, Evaluation) {
    collector.sample();
    thread::sleep(Duration::from_millis(250));
    collector.sample();
    // Rates and CPU usage in the reported sample cover one full interval.
    thread::sleep(Duration::from_millis(interval_ms));
    let mut snapshot = collector.sample();
    if let Some(r) = redactor {
        r.apply(&mut snapshot);
    }
    let instant = AlertConfig {
        sustain: 1,
        ..config.clone()
    };
    let evaluation = AlertEngine::default().evaluate(&snapshot, &instant);
    (snapshot, evaluation)
}

fn reading<T>(v: &Reading<T>, f: impl FnOnce(&T) -> String) -> String {
    match v {
        Reading::Available(v) => f(v),
        Reading::Unavailable(r) => format!("unavailable ({r})"),
    }
}

fn optional<T>(v: Option<T>, f: impl FnOnce(T) -> String) -> String {
    v.map(f)
        .unwrap_or_else(|| "unavailable (not reported)".into())
}

fn heading(out: &mut String, name: &str) {
    let _ = writeln!(out, "\n{name}");
}

fn field(out: &mut String, key: &str, value: impl AsRef<str>) {
    // At least one space even when the key is longer than the column.
    let _ = writeln!(out, "  {key:<15} {}", value.as_ref());
}

fn celsius(v: f64) -> String {
    format!("{v:.1} °C")
}

fn watts(v: f64) -> String {
    format!("{v:.1} W")
}

fn kind_name(k: SensorKind) -> &'static str {
    match k {
        SensorKind::Cpu => "cpu",
        SensorKind::Gpu => "gpu",
        SensorKind::Storage => "storage",
        SensorKind::Other => "other",
    }
}

fn battery_status(s: BatteryStatus) -> &'static str {
    match s {
        BatteryStatus::Charging => "charging",
        BatteryStatus::Discharging => "discharging",
        BatteryStatus::Full => "full",
        BatteryStatus::NotCharging => "not charging",
        BatteryStatus::Unknown => "unknown",
    }
}

/// A complete plain text report: every reading, every rule and the consensus.
pub fn text(s: &Snapshot, eval: &Evaluation) -> String {
    let when = match clock::local_offset_secs((s.taken_at_ms / 1000) as i64) {
        Some(offset) => format!("{} local time", clock::hms(s.taken_at_ms, offset)),
        None => format!("{} UTC", clock::hms(s.taken_at_ms, 0)),
    };
    let mut out = format!("nervtop report, sample {} at {when}\n", s.seq);

    heading(&mut out, "HOST");
    field(
        &mut out,
        "hostname",
        reading(&s.host.hostname, Clone::clone),
    );
    field(&mut out, "kernel", reading(&s.host.kernel, Clone::clone));
    field(
        &mut out,
        "uptime",
        reading(&s.host.uptime_secs, |v| fmt_duration(*v)),
    );

    heading(&mut out, "CPU");
    match &s.cpu {
        Reading::Available(c) => {
            field(&mut out, "total", fmt_percent(c.total as f64));
            let cores: Vec<String> = c.per_core.iter().map(|v| format!("{v:.0}")).collect();
            field(&mut out, "cores (%)", cores.join(" "));
            field(
                &mut out,
                "load average",
                reading(&c.load_avg, |l| {
                    format!("{:.2} {:.2} {:.2}", l[0], l[1], l[2])
                }),
            );
            field(
                &mut out,
                "frequency",
                optional(c.freq_mhz, |v| format!("{v} MHz")),
            );
            field(
                &mut out,
                "model",
                optional(c.brand.as_deref(), str::to_owned),
            );
        }
        Reading::Unavailable(r) => field(&mut out, "cpu", format!("unavailable ({r})")),
    }

    heading(&mut out, "MEMORY");
    match &s.memory {
        Reading::Available(m) => {
            field(
                &mut out,
                "ram",
                format!(
                    "{} of {} used ({})",
                    fmt_bytes(m.used_bytes()),
                    fmt_bytes(m.total_bytes),
                    optional(m.used_percent(), fmt_percent)
                ),
            );
            field(&mut out, "available", fmt_bytes(m.available_bytes));
            let swap = if m.swap_total_bytes == 0 {
                "none configured".to_owned()
            } else {
                format!(
                    "{} of {} used ({})",
                    fmt_bytes(m.swap_used_bytes),
                    fmt_bytes(m.swap_total_bytes),
                    optional(m.swap_used_percent(), fmt_percent)
                )
            };
            field(&mut out, "swap", swap);
        }
        Reading::Unavailable(r) => field(&mut out, "memory", format!("unavailable ({r})")),
    }

    heading(&mut out, "STORAGE");
    match &s.disks {
        Reading::Available(ds) if ds.is_empty() => field(&mut out, "disks", "none listed"),
        Reading::Available(ds) => {
            for d in ds {
                field(
                    &mut out,
                    &d.mount_point,
                    format!(
                        "{} of {} used ({}), {} {}, read {}, write {}",
                        fmt_bytes(d.used_bytes()),
                        fmt_bytes(d.total_bytes),
                        optional(d.used_percent(), fmt_percent),
                        d.device,
                        d.file_system,
                        reading(&d.read_bps, |v| fmt_rate(*v)),
                        reading(&d.write_bps, |v| fmt_rate(*v))
                    ),
                );
            }
        }
        Reading::Unavailable(r) => field(&mut out, "disks", format!("unavailable ({r})")),
    }

    heading(&mut out, "NETWORK");
    match &s.network {
        Reading::Available(ns) if ns.is_empty() => {
            field(&mut out, "interfaces", "none with traffic")
        }
        Reading::Available(ns) => {
            for n in ns {
                field(
                    &mut out,
                    &n.name,
                    format!(
                        "down {}, up {}, total down {}, total up {}",
                        reading(&n.rx_bps, |v| fmt_rate(*v)),
                        reading(&n.tx_bps, |v| fmt_rate(*v)),
                        fmt_bytes(n.rx_total_bytes),
                        fmt_bytes(n.tx_total_bytes)
                    ),
                );
            }
        }
        Reading::Unavailable(r) => field(&mut out, "network", format!("unavailable ({r})")),
    }

    heading(&mut out, "TEMPERATURES AND FANS");
    match &s.sensors {
        Reading::Available(ss) => {
            let mut temps: Vec<&TempSensor> = ss.temps.iter().collect();
            temps.sort_by(|a, b| b.celsius.total_cmp(&a.celsius));
            for t in temps {
                let crit = t
                    .crit_celsius
                    .map(|c| format!(", critical at {}", celsius(c)))
                    .unwrap_or_default();
                field(
                    &mut out,
                    &format!("{} {}", t.chip, t.label),
                    format!("{} ({}){crit}", celsius(t.celsius), kind_name(t.kind)),
                );
            }
            for f in &ss.fans {
                field(
                    &mut out,
                    &format!("{} {}", f.chip, f.label),
                    format!("{} RPM", f.rpm),
                );
            }
        }
        Reading::Unavailable(r) => field(&mut out, "sensors", format!("unavailable ({r})")),
    }

    heading(&mut out, "GPU");
    match &s.gpus {
        Reading::Available(gs) => {
            for g in gs {
                let state = match g.power_state {
                    GpuPower::Active => "active",
                    GpuPower::Suspended => "suspended (not woken to read it)",
                };
                field(&mut out, &g.pci_slot, format!("{}, {state}", g.name));
                field(
                    &mut out,
                    "  utilisation",
                    reading(&g.utilization_percent, |v| fmt_percent(*v as f64)),
                );
                field(
                    &mut out,
                    "  memory used",
                    reading(&g.memory_used_bytes, |v| fmt_bytes(*v)),
                );
                field(
                    &mut out,
                    "  memory total",
                    reading(&g.memory_total_bytes, |v| fmt_bytes(*v)),
                );
                field(
                    &mut out,
                    "  temperature",
                    reading(&g.temperature_celsius, |v| celsius(*v)),
                );
                field(&mut out, "  power", reading(&g.power_watts, |v| watts(*v)));
            }
        }
        Reading::Unavailable(r) => field(&mut out, "gpu", format!("unavailable ({r})")),
    }

    heading(&mut out, "BATTERY");
    match &s.battery {
        Reading::Available(b) => {
            field(
                &mut out,
                &b.name,
                format!("{}, {}", fmt_percent(b.percent), battery_status(b.status)),
            );
            field(
                &mut out,
                "ac adapter",
                optional(b.ac_online, |v| {
                    if v { "connected" } else { "disconnected" }.to_owned()
                }),
            );
            field(&mut out, "power", reading(&b.power_watts, |v| watts(*v)));
            field(
                &mut out,
                "time remaining",
                reading(&b.time_remaining_secs, |v| fmt_duration(*v)),
            );
            field(
                &mut out,
                "health",
                reading(&b.health_percent, |v| fmt_percent(*v)),
            );
            field(
                &mut out,
                "charge limit",
                optional(b.charge_limit_percent, |v| format!("{v}%")),
            );
            field(
                &mut out,
                "cycles",
                optional(b.cycle_count, |v| v.to_string()),
            );
        }
        Reading::Unavailable(r) => field(&mut out, "battery", format!("unavailable ({r})")),
    }

    heading(&mut out, "PROCESSES");
    match &s.processes {
        Reading::Available(ps) => {
            let mut busiest: Vec<&ProcessInfo> = ps.iter().collect();
            busiest.sort_by(|a, b| {
                b.cpu_percent
                    .total_cmp(&a.cpu_percent)
                    .then(b.memory_bytes.cmp(&a.memory_bytes))
            });
            let _ = writeln!(
                out,
                "  {} processes; the {} busiest:",
                ps.len(),
                busiest.len().min(REPORT_PROCESSES)
            );
            let _ = writeln!(
                out,
                "  {:>8} {:<12} {:>7} {:>11} {:<7} NAME",
                "PID", "USER", "CPU", "MEMORY", "STATE"
            );
            for p in busiest.into_iter().take(REPORT_PROCESSES) {
                let _ = writeln!(
                    out,
                    "  {:>8} {:<12} {:>7} {:>11} {:<7} {}",
                    p.pid,
                    p.user,
                    fmt_percent(p.cpu_percent as f64),
                    fmt_bytes(p.memory_bytes),
                    p.state,
                    p.name
                );
            }
        }
        Reading::Unavailable(r) => field(&mut out, "processes", format!("unavailable ({r})")),
    }

    heading(&mut out, "ALERT RULES (instantaneous levels)");
    for r in &eval.rules {
        let name = if r.subject.is_empty() {
            r.rule.label().to_owned()
        } else {
            format!("{} {}", r.rule.label(), r.subject)
        };
        let detail = match &r.note {
            Some(note) => format!("unavailable ({note})"),
            None => format!(
                "{}, warn {}, crit {}",
                r.value_text(),
                format_rule_value(r.rule, r.warn),
                format_rule_value(r.rule, r.crit)
            ),
        };
        let _ = writeln!(out, "  {:<9} {name}: {detail}", r.level.label());
    }

    let _ = writeln!(
        out,
        "\nCONSENSUS: {} ({})",
        eval.consensus.verdict, eval.consensus.agreement
    );
    for c in &eval.consensus.cells {
        let reason = c
            .reason
            .as_deref()
            .map(|r| format!(", {r}"))
            .unwrap_or_default();
        let _ = writeln!(out, "  {:<11} {}{reason}", c.cell.name(), c.vote);
    }
    out
}

/// The snapshot, rule states and consensus as pretty JSON.
pub fn json(snapshot: &Snapshot, eval: &Evaluation) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "snapshot": snapshot,
        "rules": eval.rules,
        "consensus": eval.consensus,
    }))
    .expect("report contains serializable data")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alert::{Level, RuleId};
    use crate::collect::fake;

    #[test]
    fn once_judges_instantaneous_levels_without_sustain() {
        let mut snap = fake::sample_snapshot();
        snap.disks = Reading::Available(vec![DiskStats {
            mount_point: "/".into(),
            device: "nvme0n1p8".into(),
            file_system: "ext4".into(),
            total_bytes: 100,
            available_bytes: 3,
            read_bps: Reading::Available(0.0),
            write_bps: Reading::Available(0.0),
        }]);
        let mut collector = Collector::new(fake::sources_with(snap));
        let config = AlertConfig::default();
        assert_eq!(config.sustain, 3);
        let (_, eval) = once(&mut collector, 250, &config, None);
        let disk = eval.rules.iter().find(|r| r.rule == RuleId::Disk).unwrap();
        assert_eq!(disk.level, Level::Critical);
        let text = text(&fake::sample_snapshot(), &eval);
        assert!(
            text.contains("CRITICAL  DISK /: 97.0%, warn 90.0%, crit 95.0%"),
            "{text}"
        );
    }

    #[test]
    fn long_keys_stay_separated_from_values() {
        let mut out = String::new();
        field(&mut out, "mt7921_phy0 temp1", "50.0 °C");
        assert_eq!(out, "  mt7921_phy0 temp1 50.0 °C\n");
    }

    #[test]
    fn reported_sample_is_a_full_interval_after_the_baseline() {
        let mut collector = Collector::new(fake::sources_with(fake::sample_snapshot()));
        let started = std::time::Instant::now();
        once(&mut collector, 250, &AlertConfig::default(), None);
        // 250 ms warm-up to baseline, then one full 250 ms interval.
        assert!(
            started.elapsed() >= Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn processes_are_limited_to_the_busiest() {
        let mut snap = fake::sample_snapshot();
        let template = snap.processes.value().unwrap()[0].clone();
        snap.processes = Reading::Available(
            (0..40)
                .map(|i| ProcessInfo {
                    pid: i,
                    cpu_percent: i as f32,
                    name: format!("proc{i}"),
                    ..template.clone()
                })
                .collect(),
        );
        let eval = AlertEngine::default().evaluate(&snap, &AlertConfig::default());
        let out = text(&snap, &eval);
        assert!(out.contains("40 processes; the 15 busiest:"), "{out}");
        assert!(out.contains(" proc39\n"));
        assert!(!out.contains(" proc24\n"));
    }
}
