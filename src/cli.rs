//! Command-line parsing. Arguments exclude the executable name.
use crate::{
    app::Screen,
    config::{MAX_INTERVAL_MS, MIN_INTERVAL_MS},
    theme::ThemeName,
};
use std::{fmt, path::PathBuf};

pub const USAGE: &str = "Usage: nervtop [OPTIONS]\n\n  --config PATH       Read a config file\n  --interval MS       Sample every 250 to 10000 ms\n  --theme NAME        nerv, phosphor, ansi\n  --print-config      Print the complete documented default config\n  --once              Print a live report and exit\n  --json              JSON report, requires --once\n  --screenshot PATH   Render live UI to SVG\n  --size WxH          Screenshot size, 80x24 to 500x200 (default 120x40)\n  --screen NAME       overview, processes, diagnostics (screenshot only)\n  -h, --help          Print help\n  -V, --version       Print version\n\nUse --print-config to see all configuration keys and defaults.\n";
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run(Options),
    PrintConfig,
    Help,
    Version,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub config: Option<PathBuf>,
    pub interval_ms: Option<u64>,
    pub theme: Option<ThemeName>,
    pub once: bool,
    pub json: bool,
    pub screenshot: Option<PathBuf>,
    pub size: (u16, u16),
    pub screen: Screen,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            config: None,
            interval_ms: None,
            theme: None,
            once: false,
            json: false,
            screenshot: None,
            size: (120, 40),
            screen: Screen::Overview,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);
impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CliError {}
fn error(s: impl Into<String>) -> CliError {
    CliError(s.into().replace(['\n', '\r'], " "))
}
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, CliError> {
    let mut args = args.into_iter();
    let mut o = Options::default();
    let mut size_set = false;
    let mut screen_set = false;
    let mut action = None;
    while let Some(arg) = args.next() {
        let (flag, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
        if matches!(
            flag,
            "--config" | "--interval" | "--theme" | "--screenshot" | "--size" | "--screen"
        ) {
            let value = inline
                .map(str::to_owned)
                .or_else(|| args.next().filter(|s| !s.starts_with('-')))
                .filter(|s| !s.is_empty())
                .ok_or_else(|| error(format!("{flag} requires a value")))?;
            match flag {
                "--config" => o.config = Some(value.into()),
                "--screenshot" => o.screenshot = Some(value.into()),
                "--interval" => {
                    let v = value
                        .parse::<u64>()
                        .map_err(|_| error("--interval must be 250 to 10000 milliseconds"))?;
                    if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&v) {
                        return Err(error("--interval must be 250 to 10000 milliseconds"));
                    }
                    o.interval_ms = Some(v);
                }
                "--theme" => o.theme = Some(value.parse().map_err(error)?),
                "--size" => {
                    let (w, h) = value
                        .split_once('x')
                        .ok_or_else(|| error("--size must be WxH"))?;
                    let w = w
                        .parse::<u16>()
                        .map_err(|_| error("invalid --size width"))?;
                    let h = h
                        .parse::<u16>()
                        .map_err(|_| error("invalid --size height"))?;
                    if !(80..=500).contains(&w) || !(24..=200).contains(&h) {
                        return Err(error("--size must be between 80x24 and 500x200"));
                    }
                    o.size = (w, h);
                    size_set = true;
                }
                "--screen" => {
                    o.screen = match value.as_str() {
                        "overview" => Screen::Overview,
                        "processes" => Screen::Processes,
                        "diagnostics" => Screen::Diagnostics,
                        _ => {
                            return Err(error(
                                "--screen must be overview, processes or diagnostics",
                            ));
                        }
                    };
                    screen_set = true;
                }
                _ => unreachable!(),
            }
        } else {
            if inline.is_some() {
                return Err(error(format!("{flag} does not accept a value")));
            }
            match flag {
                "--once" => o.once = true,
                "--json" => o.json = true,
                "--print-config" => action = Some(Command::PrintConfig),
                "--help" | "-h" => action = Some(Command::Help),
                "--version" | "-V" => action = Some(Command::Version),
                _ => return Err(error(format!("unknown argument: {flag}"))),
            }
        }
    }
    if o.json && !o.once {
        return Err(error("--json requires --once"));
    }
    if (size_set || screen_set) && o.screenshot.is_none() {
        return Err(error("--size and --screen require --screenshot"));
    }
    Ok(action.unwrap_or(Command::Run(o)))
}
