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

#[cfg(test)]
mod tests {
    use super::*;

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
