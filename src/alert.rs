//! Alert rules and the three-cell consensus.
//!
//! The types here are the contract between the alert engine, the app state and
//! the UI. See docs/SPEC.md, "Alert rules" and "Consensus panel".

use serde::Serialize;
use std::fmt;

/// Severity of one rule or one consensus cell.
///
/// `Unknown` means the input could not be read. It is never treated as
/// nominal and never as an alert. `Ord` ranks severity for "worst of":
/// Unknown < Nominal < Caution < Critical, and `worst` skips Unknown unless
/// every input is Unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Unknown,
    Nominal,
    Caution,
    Critical,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Unknown => "NO DATA",
            Level::Nominal => "NOMINAL",
            Level::Caution => "CAUTION",
            Level::Critical => "CRITICAL",
        }
    }

    /// The most severe level in `levels`; Unknown only when every level is
    /// Unknown or `levels` is empty.
    pub fn worst(levels: impl IntoIterator<Item = Level>) -> Level {
        levels.into_iter().max().unwrap_or(Level::Unknown)
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Every alert rule. The config key of each is `RuleId::key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleId {
    CpuUsage,
    Load,
    CpuTemp,
    GpuTemp,
    Memory,
    Swap,
    Disk,
    StorageTemp,
    Battery,
}

impl RuleId {
    pub const ALL: [RuleId; 9] = [
        RuleId::CpuUsage,
        RuleId::Load,
        RuleId::CpuTemp,
        RuleId::GpuTemp,
        RuleId::Memory,
        RuleId::Swap,
        RuleId::Disk,
        RuleId::StorageTemp,
        RuleId::Battery,
    ];

    pub fn key(self) -> &'static str {
        match self {
            RuleId::CpuUsage => "cpu_usage",
            RuleId::Load => "load",
            RuleId::CpuTemp => "cpu_temp",
            RuleId::GpuTemp => "gpu_temp",
            RuleId::Memory => "memory",
            RuleId::Swap => "swap",
            RuleId::Disk => "disk",
            RuleId::StorageTemp => "storage_temp",
            RuleId::Battery => "battery",
        }
    }

    /// Short upper-case label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            RuleId::CpuUsage => "CPU LOAD",
            RuleId::Load => "LOAD AVG",
            RuleId::CpuTemp => "CPU TEMP",
            RuleId::GpuTemp => "GPU TEMP",
            RuleId::Memory => "MEMORY",
            RuleId::Swap => "SWAP",
            RuleId::Disk => "DISK",
            RuleId::StorageTemp => "DRIVE TEMP",
            RuleId::Battery => "BATTERY",
        }
    }

    /// Unit suffix for values of this rule.
    pub fn unit(self) -> &'static str {
        match self {
            RuleId::CpuUsage | RuleId::Memory | RuleId::Swap | RuleId::Disk | RuleId::Battery => {
                "%"
            }
            RuleId::Load => "/core",
            RuleId::CpuTemp | RuleId::GpuTemp | RuleId::StorageTemp => "°C",
        }
    }

    /// True when the rule fires at or below its thresholds.
    pub fn fires_below(self) -> bool {
        matches!(self, RuleId::Battery)
    }

    /// The consensus cell that judges this rule.
    pub fn cell(self) -> CellId {
        match self {
            RuleId::CpuUsage | RuleId::Load => CellId::Compute,
            RuleId::CpuTemp | RuleId::GpuTemp | RuleId::StorageTemp => CellId::Thermal,
            RuleId::Memory | RuleId::Swap | RuleId::Disk | RuleId::Battery => CellId::Resource,
        }
    }
}

/// Current state of one rule applied to one subject.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleStatus {
    pub rule: RuleId,
    /// What the rule looked at: a mount point for `disk`, a sensor for
    /// temperatures, empty when the rule has a single subject.
    pub subject: String,
    /// Current value, `None` when the source is unavailable.
    pub value: Option<f64>,
    /// Current level after sustain and hysteresis.
    pub level: Level,
    pub warn: f64,
    pub crit: f64,
    /// Why the level is Unknown, when it is.
    pub note: Option<String>,
}

/// The three consensus cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellId {
    Compute,
    Thermal,
    Resource,
}

impl CellId {
    pub const ALL: [CellId; 3] = [CellId::Compute, CellId::Thermal, CellId::Resource];

    /// Display name, for example "COMPUTE-1".
    pub fn name(self) -> &'static str {
        match self {
            CellId::Compute => "COMPUTE-1",
            CellId::Thermal => "THERMAL-2",
            CellId::Resource => "RESOURCE-3",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellVote {
    pub cell: CellId,
    pub vote: Level,
    /// The rule status that decided the vote, if any rule was at Caution or worse.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Agreement {
    /// All three cells voted the same level.
    Unanimous,
    /// Two cells agree and one differs.
    Majority,
    /// All three cells voted differently.
    ThreeWay,
}

impl Agreement {
    /// Classifies three votes.
    pub fn of(votes: [Level; 3]) -> Agreement {
        let [a, b, c] = votes;
        if a == b && b == c {
            Agreement::Unanimous
        } else if a == b || b == c || a == c {
            Agreement::Majority
        } else {
            Agreement::ThreeWay
        }
    }
}

impl fmt::Display for Agreement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Agreement::Unanimous => "UNANIMOUS",
            Agreement::Majority => "SPLIT 2-1",
            Agreement::ThreeWay => "SPLIT 1-1-1",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Consensus {
    pub cells: [CellVote; 3],
    /// Worst vote among the cells.
    pub verdict: Level,
    pub agreement: Agreement,
}

/// A change in one rule's level, kept in the alert log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlertEvent {
    /// Milliseconds since the Unix epoch, from the snapshot that caused it.
    pub at_ms: u64,
    pub rule: RuleId,
    pub subject: String,
    pub from: Level,
    pub to: Level,
    pub value: Option<f64>,
}

use crate::config::{AlertConfig, RuleConfig};
use crate::model::{BatteryStatus, GpuPower, SensorKind, Snapshot};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct AlertEngine {
    states: BTreeMap<(RuleId, String), RuleState>,
}

#[derive(Debug, Clone)]
struct RuleState {
    level: Level,
    warn_samples: u32,
    crit_samples: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub rules: Vec<RuleStatus>,
    pub consensus: Consensus,
    pub events: Vec<AlertEvent>,
}

impl AlertEngine {
    pub fn evaluate(&mut self, snapshot: &Snapshot, config: &AlertConfig) -> Evaluation {
        let mut rules = Vec::new();
        let mut events = Vec::new();
        let mut next = BTreeMap::new();
        for (rule, (_, cfg, _)) in RuleId::ALL.into_iter().zip(config.rules()) {
            if !cfg.enabled {
                continue;
            }
            for (subject, value, note, nominal) in inputs(snapshot, rule) {
                // Disks are judged per mount. Every other rule judges one
                // value (for temperatures, the hottest sensor), so its state
                // must survive the hottest sensor changing name.
                let key = (rule, state_subject(rule, &subject));
                let mut state = self.states.remove(&key).unwrap_or(RuleState {
                    level: Level::Unknown,
                    warn_samples: 0,
                    crit_samples: 0,
                });
                let previous = state.level;
                state.update(value, nominal, rule.fires_below(), cfg, config.sustain);
                if previous != state.level
                    && !matches!(
                        (previous, state.level),
                        (Level::Unknown, Level::Nominal) | (Level::Nominal, Level::Unknown)
                    )
                {
                    events.push(AlertEvent {
                        at_ms: snapshot.taken_at_ms,
                        rule,
                        subject: subject.clone(),
                        from: previous,
                        to: state.level,
                        value,
                    });
                }
                rules.push(RuleStatus {
                    rule,
                    subject,
                    value,
                    level: state.level,
                    warn: cfg.warn,
                    crit: cfg.crit,
                    note,
                });
                next.insert(key, state);
            }
        }
        self.states = next;
        let cells = CellId::ALL.map(|cell| {
            let vote = Level::worst(
                rules
                    .iter()
                    .filter(|r| r.rule.cell() == cell)
                    .map(|r| r.level),
            );
            let reason = if vote >= Level::Caution {
                rules
                    .iter()
                    .find(|r| r.rule.cell() == cell && r.level == vote)
                    .map(describe)
            } else {
                None
            };
            CellVote { cell, vote, reason }
        });
        let votes = cells.each_ref().map(|c| c.vote);
        Evaluation {
            rules,
            consensus: Consensus {
                cells,
                verdict: Level::worst(votes),
                agreement: Agreement::of(votes),
            },
            events,
        }
    }
}

impl RuleState {
    fn update(
        &mut self,
        value: Option<f64>,
        nominal: bool,
        below: bool,
        cfg: &RuleConfig,
        sustain: u32,
    ) {
        let Some(value) = value else {
            self.level = Level::Unknown;
            self.warn_samples = 0;
            self.crit_samples = 0;
            return;
        };
        let fires = |threshold| {
            if below {
                value <= threshold
            } else {
                value >= threshold
            }
        };
        let cleared = |threshold| {
            if below {
                value > threshold + cfg.hysteresis
            } else {
                value < threshold - cfg.hysteresis
            }
        };
        if nominal {
            self.level = Level::Nominal;
            self.warn_samples = 0;
            self.crit_samples = 0;
            return;
        }
        self.warn_samples = if fires(cfg.warn) {
            self.warn_samples.saturating_add(1)
        } else {
            0
        };
        self.crit_samples = if fires(cfg.crit) {
            self.crit_samples.saturating_add(1)
        } else {
            0
        };
        let retained = if self.level == Level::Critical && !cleared(cfg.crit) {
            Level::Critical
        } else if self.level >= Level::Caution && !cleared(cfg.warn) {
            Level::Caution
        } else {
            Level::Nominal
        };
        let raised = if self.crit_samples >= sustain {
            Level::Critical
        } else if self.warn_samples >= sustain {
            Level::Caution
        } else {
            Level::Nominal
        };
        self.level = retained.max(raised);
    }
}

fn state_subject(rule: RuleId, subject: &str) -> String {
    if rule == RuleId::Disk {
        subject.to_owned()
    } else {
        String::new()
    }
}

/// "CPU TEMP k10temp Tctl 89.3°C": label, subject when there is one, value.
fn describe(r: &RuleStatus) -> String {
    let mut parts = vec![r.rule.label().to_owned()];
    if !r.subject.is_empty() {
        parts.push(r.subject.clone());
    }
    parts.push(r.value_text());
    parts.join(" ")
}

impl RuleStatus {
    /// The value with its unit, for example "89.3°C" or "0.21/core", or
    /// "unavailable".
    pub fn value_text(&self) -> String {
        self.value
            .map(|v| format_rule_value(self.rule, v))
            .unwrap_or_else(|| "unavailable".to_owned())
    }
}

/// A threshold or value of `rule` with its unit.
pub fn format_rule_value(rule: RuleId, v: f64) -> String {
    if rule == RuleId::Load {
        format!("{v:.2}{}", rule.unit())
    } else {
        format!("{v:.1}{}", rule.unit())
    }
}

type Input = (String, Option<f64>, Option<String>, bool);
fn inputs(s: &Snapshot, rule: RuleId) -> Vec<Input> {
    let single = |value: Option<f64>, reason: Option<&str>| {
        vec![(
            String::new(),
            value,
            value
                .is_none()
                .then(|| reason.unwrap_or("not reported").to_string()),
            false,
        )]
    };
    match rule {
        RuleId::CpuUsage => single(s.cpu.value().map(|c| c.total as f64), s.cpu.reason()),
        RuleId::Load => single(
            s.cpu
                .value()
                .filter(|c| !c.per_core.is_empty())
                .map(|c| c.load_avg[0] / c.per_core.len() as f64),
            s.cpu.reason().or(Some("no cpu cores")),
        ),
        RuleId::Memory => single(
            s.memory.value().and_then(|m| m.used_percent()),
            s.memory.reason().or(Some("zero memory total")),
        ),
        RuleId::Swap => {
            if s.memory.value().is_some_and(|m| m.swap_total_bytes == 0) {
                vec![]
            } else {
                single(
                    s.memory.value().and_then(|m| m.swap_used_percent()),
                    s.memory.reason(),
                )
            }
        }
        RuleId::Disk => match s.disks.value() {
            Some(disks) => {
                let mut out: Vec<_> = disks
                    .iter()
                    .map(|d| {
                        (
                            d.mount_point.clone(),
                            d.used_percent(),
                            (d.total_bytes == 0).then(|| "zero disk total".into()),
                            false,
                        )
                    })
                    .collect();
                out.sort_by(|a, b| a.0.cmp(&b.0));
                out
            }
            None => single(None, s.disks.reason()),
        },
        RuleId::Battery => match &s.battery {
            crate::model::Reading::Unavailable(r) if r == "no battery present" => vec![],
            crate::model::Reading::Unavailable(r) => single(None, Some(r)),
            crate::model::Reading::Available(b) => {
                // Only a discharging battery is judged. An unknown status
                // borrows the answer from the AC adapter when there is one.
                let discharging = match b.status {
                    BatteryStatus::Discharging => true,
                    BatteryStatus::Charging | BatteryStatus::Full | BatteryStatus::NotCharging => {
                        false
                    }
                    BatteryStatus::Unknown => match b.ac_online {
                        Some(online) => !online,
                        None => return single(None, Some("battery status unknown")),
                    },
                };
                vec![(String::new(), Some(b.percent), None, !discharging)]
            }
        },
        RuleId::CpuTemp | RuleId::StorageTemp | RuleId::GpuTemp => {
            let kind = match rule {
                RuleId::CpuTemp => SensorKind::Cpu,
                RuleId::StorageTemp => SensorKind::Storage,
                _ => SensorKind::Gpu,
            };
            let mut values: Vec<(String, f64)> = s
                .sensors
                .value()
                .into_iter()
                .flat_map(|s| &s.temps)
                .filter(|t| t.kind == kind)
                .map(|t| (format!("{} {}", t.chip, t.label), t.celsius))
                .collect();
            let mut reason = s.sensors.reason().unwrap_or("no matching sensors");
            if rule == RuleId::GpuTemp {
                reason = s.gpus.reason().unwrap_or(reason);
                if let Some(gpus) = s.gpus.value() {
                    for g in gpus {
                        if g.power_state == GpuPower::Suspended {
                            reason = "gpu suspended";
                            continue;
                        }
                        if let Some(v) = g.temperature_celsius.value() {
                            values.push((format!("{} {}", g.pci_slot, g.name), *v));
                        } else if let Some(r) = g.temperature_celsius.reason() {
                            reason = r;
                        }
                    }
                }
            }
            match values
                .into_iter()
                .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            {
                Some((subject, value)) => vec![(subject, Some(value), None, false)],
                None => single(None, Some(reason)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thermal_snapshot(temps: &[(&str, f64)], seq: u64) -> Snapshot {
        use crate::model::*;
        let mut s = crate::collect::fake::sample_snapshot();
        s.seq = seq;
        s.sensors = Reading::Available(SensorStats {
            temps: temps
                .iter()
                .map(|(label, c)| TempSensor {
                    chip: "k10temp".into(),
                    label: (*label).into(),
                    kind: SensorKind::Cpu,
                    celsius: *c,
                    crit_celsius: None,
                })
                .collect(),
            fans: vec![],
        });
        s
    }

    fn cpu_temp(eval: &Evaluation) -> &RuleStatus {
        eval.rules
            .iter()
            .find(|r| r.rule == RuleId::CpuTemp)
            .expect("cpu_temp evaluated")
    }

    #[test]
    fn hottest_sensor_changing_does_not_reset_sustain() {
        let config = crate::config::AlertConfig::default();
        let mut engine = AlertEngine::default();
        let samples = [
            [("Tctl", 90.0), ("Tccd1", 88.0)],
            [("Tctl", 88.0), ("Tccd1", 91.0)],
            [("Tctl", 92.0), ("Tccd1", 89.0)],
        ];
        let mut last = None;
        for (i, temps) in samples.iter().enumerate() {
            last = Some(engine.evaluate(&thermal_snapshot(temps, i as u64 + 1), &config));
        }
        let eval = last.unwrap();
        assert_eq!(cpu_temp(&eval).level, Level::Caution);
        assert_eq!(cpu_temp(&eval).subject, "k10temp Tctl");
    }

    #[test]
    fn consensus_reason_uses_one_decimal() {
        let config = crate::config::AlertConfig::default();
        let mut engine = AlertEngine::default();
        let mut eval = None;
        for seq in 1..=3 {
            eval = Some(engine.evaluate(&thermal_snapshot(&[("Tctl", 89.254_321)], seq), &config));
        }
        let eval = eval.unwrap();
        let thermal = &eval.consensus.cells[1];
        assert_eq!(thermal.vote, Level::Caution);
        assert_eq!(
            thermal.reason.as_deref(),
            Some("CPU TEMP k10temp Tctl 89.3°C")
        );
    }

    #[test]
    fn worst_skips_unknown_unless_all_unknown() {
        assert_eq!(
            Level::worst([Level::Unknown, Level::Nominal]),
            Level::Nominal
        );
        assert_eq!(
            Level::worst([Level::Caution, Level::Unknown, Level::Critical]),
            Level::Critical
        );
        assert_eq!(Level::worst([Level::Unknown]), Level::Unknown);
        assert_eq!(Level::worst([]), Level::Unknown);
    }

    #[test]
    fn every_rule_maps_to_a_cell_and_matches_config_keys() {
        let config = crate::config::AlertConfig::default();
        let keys: Vec<&str> = config.rules().iter().map(|(k, _, _)| *k).collect();
        let ids: Vec<&str> = RuleId::ALL.iter().map(|r| r.key()).collect();
        assert_eq!(keys, ids);
        for (rule, (_, _, below)) in RuleId::ALL.iter().zip(config.rules()) {
            assert_eq!(rule.fires_below(), below, "{}", rule.key());
        }
    }

    #[test]
    fn agreement_classifies_and_formats() {
        use Level::*;
        assert_eq!(
            Agreement::of([Nominal, Nominal, Nominal]),
            Agreement::Unanimous
        );
        assert_eq!(
            Agreement::of([Nominal, Caution, Nominal]),
            Agreement::Majority
        );
        assert_eq!(
            Agreement::of([Unknown, Caution, Critical]),
            Agreement::ThreeWay
        );
        assert_eq!(Agreement::Unanimous.to_string(), "UNANIMOUS");
        assert_eq!(Agreement::Majority.to_string(), "SPLIT 2-1");
        assert_eq!(Agreement::ThreeWay.to_string(), "SPLIT 1-1-1");
    }
}
