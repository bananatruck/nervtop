//! The config file: sample interval, theme and alert thresholds.
//!
//! `DEFAULT_TOML` is the documented default file. A test proves it parses to
//! exactly `Config::default()`, so the documentation cannot drift from the code.

use crate::theme::{self, Palette, ThemeName};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// The documented default config, printed by `nervtop --print-config`.
pub const DEFAULT_TOML: &str = include_str!("../config/default.toml");

pub const MIN_INTERVAL_MS: u64 = 250;
pub const MAX_INTERVAL_MS: u64 = 10_000;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub general: General,
    pub theme: ThemeConfig,
    pub alerts: AlertConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct General {
    /// Milliseconds between samples.
    pub interval_ms: u64,
    /// Hide kernel threads (processes with no command line) from the process lists.
    pub hide_kernel_threads: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ThemeConfig {
    pub name: ThemeName,
    /// Slot name to colour string, applied on top of the named theme.
    pub colors: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuleConfig {
    pub enabled: bool,
    pub warn: f64,
    pub crit: f64,
    pub hysteresis: f64,
}

/// A rule table as written in the file; missing keys keep that rule's defaults.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialRule {
    enabled: Option<bool>,
    warn: Option<f64>,
    crit: Option<f64>,
    hysteresis: Option<f64>,
}

impl PartialRule {
    fn apply(self, base: RuleConfig) -> RuleConfig {
        RuleConfig {
            enabled: self.enabled.unwrap_or(base.enabled),
            warn: self.warn.unwrap_or(base.warn),
            crit: self.crit.unwrap_or(base.crit),
            hysteresis: self.hysteresis.unwrap_or(base.hysteresis),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct RawAlertConfig {
    sustain: Option<u32>,
    cpu_usage: PartialRule,
    load: PartialRule,
    cpu_temp: PartialRule,
    gpu_temp: PartialRule,
    memory: PartialRule,
    swap: PartialRule,
    disk: PartialRule,
    storage_temp: PartialRule,
    battery: PartialRule,
}

impl From<RawAlertConfig> for AlertConfig {
    fn from(raw: RawAlertConfig) -> Self {
        let d = AlertConfig::default();
        AlertConfig {
            sustain: raw.sustain.unwrap_or(d.sustain),
            cpu_usage: raw.cpu_usage.apply(d.cpu_usage),
            load: raw.load.apply(d.load),
            cpu_temp: raw.cpu_temp.apply(d.cpu_temp),
            gpu_temp: raw.gpu_temp.apply(d.gpu_temp),
            memory: raw.memory.apply(d.memory),
            swap: raw.swap.apply(d.swap),
            disk: raw.disk.apply(d.disk),
            storage_temp: raw.storage_temp.apply(d.storage_temp),
            battery: raw.battery.apply(d.battery),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "RawAlertConfig")]
pub struct AlertConfig {
    /// Consecutive samples a condition must hold before its level rises.
    pub sustain: u32,
    pub cpu_usage: RuleConfig,
    pub load: RuleConfig,
    pub cpu_temp: RuleConfig,
    pub gpu_temp: RuleConfig,
    pub memory: RuleConfig,
    pub swap: RuleConfig,
    pub disk: RuleConfig,
    pub storage_temp: RuleConfig,
    pub battery: RuleConfig,
}

impl Default for General {
    fn default() -> Self {
        General {
            interval_ms: 1000,
            hide_kernel_threads: true,
        }
    }
}

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            name: ThemeName::Nerv,
            colors: BTreeMap::new(),
        }
    }
}

impl RuleConfig {
    const fn new(warn: f64, crit: f64, hysteresis: f64) -> Self {
        RuleConfig {
            enabled: true,
            warn,
            crit,
            hysteresis,
        }
    }
}

impl Default for AlertConfig {
    fn default() -> Self {
        AlertConfig {
            sustain: 3,
            cpu_usage: RuleConfig::new(85.0, 95.0, 2.0),
            load: RuleConfig::new(1.5, 3.0, 0.1),
            cpu_temp: RuleConfig::new(85.0, 95.0, 3.0),
            gpu_temp: RuleConfig::new(80.0, 90.0, 3.0),
            memory: RuleConfig::new(85.0, 95.0, 2.0),
            swap: RuleConfig::new(60.0, 90.0, 2.0),
            disk: RuleConfig::new(90.0, 95.0, 1.0),
            storage_temp: RuleConfig::new(65.0, 75.0, 3.0),
            battery: RuleConfig::new(20.0, 10.0, 2.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        message: String,
    },
    Parse {
        path: Option<PathBuf>,
        message: String,
    },
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read { path, message } => {
                write!(f, "cannot read config {}: {message}", path.display())
            }
            ConfigError::Parse {
                path: Some(path),
                message,
            } => write!(f, "invalid config {}: {message}", path.display()),
            ConfigError::Parse {
                path: None,
                message,
            } => write!(f, "invalid config: {message}"),
            ConfigError::Invalid(message) => write!(f, "invalid config: {message}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Parses and validates config text.
    pub fn from_toml(text: &str) -> Result<Config, ConfigError> {
        let config: Config = toml::from_str(text).map_err(|e| ConfigError::Parse {
            path: None,
            message: e.to_string().trim_end().to_string(),
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Loads `explicit` if given (it must exist), otherwise the default path if
    /// it exists, otherwise the built-in defaults.
    pub fn load(explicit: Option<&Path>) -> Result<Config, ConfigError> {
        let path = match explicit {
            Some(p) => p.to_path_buf(),
            None => match default_path() {
                Some(p) if p.exists() => p,
                _ => return Ok(Config::default()),
            },
        };
        let text = std::fs::read_to_string(&path).map_err(|e| ConfigError::Read {
            path: path.clone(),
            message: e.to_string(),
        })?;
        Config::from_toml(&text).map_err(|e| match e {
            ConfigError::Parse { message, .. } => ConfigError::Parse {
                path: Some(path),
                message,
            },
            other => other,
        })
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let g = &self.general;
        if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&g.interval_ms) {
            return Err(ConfigError::Invalid(format!(
                "general.interval_ms must be between {MIN_INTERVAL_MS} and {MAX_INTERVAL_MS}, got {}",
                g.interval_ms
            )));
        }
        let a = &self.alerts;
        if !(1..=60).contains(&a.sustain) {
            return Err(ConfigError::Invalid(format!(
                "alerts.sustain must be between 1 and 60, got {}",
                a.sustain
            )));
        }
        for (name, rule, below) in a.rules() {
            check_rule(name, rule, below)?;
        }
        self.palette()?;
        Ok(())
    }

    /// The configured theme with its colour overrides applied.
    pub fn palette(&self) -> Result<Palette, ConfigError> {
        palette_for(self.theme.name, &self.theme.colors)
    }
}

/// Builds the palette for `name` with `overrides` applied.
pub fn palette_for(
    name: ThemeName,
    overrides: &BTreeMap<String, String>,
) -> Result<Palette, ConfigError> {
    let mut palette = Palette::builtin(name);
    for (slot, value) in overrides {
        let color = theme::parse_color(value)
            .map_err(|e| ConfigError::Invalid(format!("theme.colors.{slot}: {e}")))?;
        palette
            .set(slot, color)
            .map_err(|e| ConfigError::Invalid(format!("theme.colors: {e}")))?;
    }
    Ok(palette)
}

impl AlertConfig {
    /// Every rule with its config key and whether it fires below (true) or above (false).
    pub fn rules(&self) -> [(&'static str, &RuleConfig, bool); 9] {
        [
            ("cpu_usage", &self.cpu_usage, false),
            ("load", &self.load, false),
            ("cpu_temp", &self.cpu_temp, false),
            ("gpu_temp", &self.gpu_temp, false),
            ("memory", &self.memory, false),
            ("swap", &self.swap, false),
            ("disk", &self.disk, false),
            ("storage_temp", &self.storage_temp, false),
            ("battery", &self.battery, true),
        ]
    }
}

fn check_rule(name: &str, r: &RuleConfig, below: bool) -> Result<(), ConfigError> {
    let finite = r.warn.is_finite() && r.crit.is_finite() && r.hysteresis.is_finite();
    if !finite {
        return Err(ConfigError::Invalid(format!(
            "alerts.{name}: thresholds must be finite numbers"
        )));
    }
    if r.hysteresis < 0.0 {
        return Err(ConfigError::Invalid(format!(
            "alerts.{name}.hysteresis must not be negative"
        )));
    }
    let ordered = if below {
        r.warn > r.crit
    } else {
        r.warn < r.crit
    };
    if !ordered {
        let relation = if below { "above" } else { "below" };
        return Err(ConfigError::Invalid(format!(
            "alerts.{name}: warn ({}) must be {relation} crit ({})",
            r.warn, r.crit
        )));
    }
    Ok(())
}

/// `$XDG_CONFIG_HOME/nervtop/config.toml`, else `$HOME/.config/nervtop/config.toml`.
pub fn default_path() -> Option<PathBuf> {
    config_path_from(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn config_path_from(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = match xdg {
        Some(x) if x.is_absolute() => x,
        _ => home?.join(".config"),
    };
    Some(base.join("nervtop").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn documented_default_file_equals_builtin_defaults() {
        let parsed = Config::from_toml(DEFAULT_TOML).expect("default.toml parses");
        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn empty_file_means_defaults() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
    }

    #[test]
    fn partial_rule_keeps_that_rules_own_defaults() {
        let c = Config::from_toml("[alerts.cpu_temp]\nwarn = 70\ncrit = 80\n").unwrap();
        assert_eq!(c.alerts.cpu_temp.warn, 70.0);
        assert_eq!(c.alerts.cpu_temp.crit, 80.0);
        assert_eq!(c.alerts.cpu_temp.hysteresis, 3.0);
        assert!(c.alerts.cpu_temp.enabled);
        assert_eq!(c.alerts.memory, AlertConfig::default().memory);
        let b = Config::from_toml("[alerts.battery]\nwarn = 30\n").unwrap();
        assert_eq!(b.alerts.battery.warn, 30.0);
        assert_eq!(b.alerts.battery.crit, 10.0);
        let off = Config::from_toml("[alerts.swap]\nenabled = false\n").unwrap();
        assert!(!off.alerts.swap.enabled);
        assert_eq!(off.alerts.swap.warn, 60.0);
    }

    #[test]
    fn unknown_rule_keys_are_errors() {
        assert!(Config::from_toml("[alerts.memory]\nwarning = 70\n").is_err());
        assert!(Config::from_toml("[alerts.gpu_usage]\nwarn = 70\n").is_err());
    }

    #[test]
    fn unknown_keys_are_errors_naming_the_key() {
        let err = Config::from_toml("[general]\nintervl_ms = 500\n").unwrap_err();
        assert!(err.to_string().contains("intervl_ms"), "{err}");
    }

    #[test]
    fn interval_out_of_range_is_rejected() {
        let err = Config::from_toml("[general]\ninterval_ms = 50\n").unwrap_err();
        assert!(err.to_string().contains("interval_ms"), "{err}");
    }

    #[test]
    fn misordered_thresholds_are_rejected_in_both_directions() {
        let above = Config::from_toml("[alerts.memory]\nwarn = 95\ncrit = 85\n").unwrap_err();
        assert!(above.to_string().contains("alerts.memory"), "{above}");
        let below = Config::from_toml("[alerts.battery]\nwarn = 10\ncrit = 20\n").unwrap_err();
        assert!(below.to_string().contains("must be above"), "{below}");
    }

    #[test]
    fn colour_overrides_apply_and_bad_ones_fail() {
        let c = Config::from_toml(
            "[theme]\nname = \"ansi\"\n[theme.colors]\naccent = \"#010203\"\nbackground = \"reset\"\n",
        )
        .unwrap();
        let p = c.palette().unwrap();
        assert_eq!(p.accent, Color::Rgb(1, 2, 3));
        assert_eq!(p.background, Color::Reset);
        assert!(Config::from_toml("[theme.colors]\nglow = \"red\"\n").is_err());
        assert!(Config::from_toml("[theme.colors]\naccent = \"#zzzzzz\"\n").is_err());
    }

    #[test]
    fn config_path_prefers_absolute_xdg() {
        let p = config_path_from(Some("/x".into()), Some("/home/u".into()));
        assert_eq!(p, Some(PathBuf::from("/x/nervtop/config.toml")));
        let p = config_path_from(Some("relative".into()), Some("/home/u".into()));
        assert_eq!(
            p,
            Some(PathBuf::from("/home/u/.config/nervtop/config.toml"))
        );
        assert_eq!(config_path_from(None, None), None);
    }

    #[test]
    fn explicit_missing_file_is_an_error() {
        let err = Config::load(Some(Path::new("/nonexistent/nervtop.toml"))).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }));
    }
}
