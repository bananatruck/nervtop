//! One-shot sampling and complete plain text or JSON reports.
use crate::{
    alert::{AlertEngine, Evaluation},
    collect::Collector,
    config::AlertConfig,
    format::*,
    model::*,
};
use std::{fmt::Write, thread, time::Duration};
pub fn once(
    collector: &mut Collector,
    interval_ms: u64,
    config: &AlertConfig,
) -> (Snapshot, Evaluation) {
    collector.sample();
    thread::sleep(Duration::from_millis(250));
    collector.sample();
    let remainder = interval_ms.saturating_sub(250);
    if remainder > 0 {
        thread::sleep(Duration::from_millis(remainder));
    }
    let snapshot = collector.sample();
    let evaluation = AlertEngine::default().evaluate(&snapshot, config);
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
fn section<T>(out: &mut String, name: &str, value: &Reading<T>, f: impl FnOnce(&T) -> String) {
    let _ = writeln!(out, "\n{name}\n{}", reading(value, f));
}
pub fn text(s: &Snapshot, eval: &Evaluation) -> String {
    let mut out = format!(
        "nervtop sample {} at {} ms\nHOST\nhostname: {}\nkernel: {}\nuptime: {}\n",
        s.seq,
        s.taken_at_ms,
        reading(&s.host.hostname, Clone::clone),
        reading(&s.host.kernel, Clone::clone),
        reading(&s.host.uptime_secs, |v| fmt_duration(*v))
    );
    section(&mut out, "CPU", &s.cpu, |c| {
        format!(
            "total: {}\ncores: {}\nload averages: {:?}\nfrequency: {}\nbrand: {}",
            fmt_percent(c.total as f64),
            c.per_core
                .iter()
                .map(|v| fmt_percent(*v as f64))
                .collect::<Vec<_>>()
                .join(", "),
            c.load_avg,
            optional(c.freq_mhz, |v| format!("{v} MHz")),
            optional(c.brand.as_deref(), str::to_owned)
        )
    });
    section(&mut out, "MEMORY", &s.memory, |m| {
        format!(
            "total: {}\navailable: {}\nused: {}\nswap total: {}\nswap used: {}",
            fmt_bytes(m.total_bytes),
            fmt_bytes(m.available_bytes),
            fmt_bytes(m.used_bytes()),
            fmt_bytes(m.swap_total_bytes),
            fmt_bytes(m.swap_used_bytes)
        )
    });
    section(&mut out, "STORAGE", &s.disks, |ds| {
        ds.iter()
            .map(|d| {
                format!(
                    "{} {} {}\ntotal: {} available: {}\nread: {} write: {}",
                    d.mount_point,
                    d.device,
                    d.file_system,
                    fmt_bytes(d.total_bytes),
                    fmt_bytes(d.available_bytes),
                    reading(&d.read_bps, |v| fmt_rate(*v)),
                    reading(&d.write_bps, |v| fmt_rate(*v))
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    section(&mut out, "NETWORK", &s.network, |ns| {
        ns.iter()
            .map(|n| {
                format!(
                    "{} rx: {} tx: {} rx total: {} tx total: {}",
                    n.name,
                    reading(&n.rx_bps, |v| fmt_rate(*v)),
                    reading(&n.tx_bps, |v| fmt_rate(*v)),
                    fmt_bytes(n.rx_total_bytes),
                    fmt_bytes(n.tx_total_bytes)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    section(&mut out, "PROCESSES", &s.processes, |ps| {
        ps.iter()
            .map(|p| {
                format!(
                    "{} {} {} {} {} {} kernel thread: {}",
                    p.pid,
                    p.user,
                    fmt_percent(p.cpu_percent as f64),
                    fmt_bytes(p.memory_bytes),
                    p.state,
                    p.name,
                    p.kernel_thread
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    section(&mut out, "THERMAL", &s.sensors, |ss| {
        let mut lines: Vec<_> = ss
            .temps
            .iter()
            .map(|t| {
                format!(
                    "{} {} {:?}: {:.1} °C critical: {}",
                    t.chip,
                    t.label,
                    t.kind,
                    t.celsius,
                    optional(t.crit_celsius, |v| format!("{v:.1} °C"))
                )
            })
            .collect();
        lines.extend(
            ss.fans
                .iter()
                .map(|f| format!("{} {}: {} RPM", f.chip, f.label, f.rpm)),
        );
        lines.join("\n")
    });
    section(&mut out, "GPU", &s.gpus, |gs| {
        gs.iter().map(|g| format!("{} {} {:?} {:?}\nutilization: {}\nmemory used: {} total: {}\ntemperature: {}\npower: {}",g.pci_slot,g.name,g.vendor,g.power_state,reading(&g.utilization_percent,|v|fmt_percent(*v as f64)),reading(&g.memory_used_bytes,|v|fmt_bytes(*v)),reading(&g.memory_total_bytes,|v|fmt_bytes(*v)),reading(&g.temperature_celsius,|v|format!("{v:.1} °C")),reading(&g.power_watts,|v|format!("{v:.1} W")))).collect::<Vec<_>>().join("\n")
    });
    section(&mut out, "POWER", &s.battery, |b| {
        format!(
            "{} {:?} {}\nAC online: {}\npower: {}\ntime remaining: {}\nhealth: {}\ncharge limit: {}\ncycles: {}",
            b.name,
            b.status,
            fmt_percent(b.percent),
            optional(b.ac_online, |v| v.to_string()),
            reading(&b.power_watts, |v| format!("{v:.1} W")),
            reading(&b.time_remaining_secs, |v| fmt_duration(*v)),
            reading(&b.health_percent, |v| fmt_percent(*v)),
            optional(b.charge_limit_percent, |v| fmt_percent(v as f64)),
            optional(b.cycle_count, |v| v.to_string())
        )
    });
    out.push_str("\nRULES\n");
    for r in &eval.rules {
        let _ = writeln!(
            out,
            "{} {}: {} {} warn={} crit={}",
            r.rule.label(),
            r.subject,
            optional(r.value, |v| format!("{v}{}", r.rule.unit())),
            r.level,
            r.warn,
            r.crit
        );
        if let Some(note) = &r.note {
            let _ = writeln!(out, "unavailable ({note})");
        }
    }
    let _ = writeln!(
        out,
        "\nCONSENSUS: {} {}",
        eval.consensus.verdict, eval.consensus.agreement
    );
    for c in &eval.consensus.cells {
        let _ = writeln!(
            out,
            "{}: {} {}",
            c.cell.name(),
            c.vote,
            c.reason.as_deref().unwrap_or("")
        );
    }
    out
}
pub fn json(snapshot: &Snapshot, eval: &Evaluation) -> String {
    serde_json::to_string_pretty(&serde_json::json!({"snapshot": snapshot, "rules": eval.rules, "consensus": eval.consensus})).expect("report contains serializable data")
}
