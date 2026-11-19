//! All UI state, with no terminal code.
//!
//! `ui` reads an `&App` and draws it. Key handling and snapshot application
//! live here so they can be tested without a terminal.

use crate::alert::{
    Agreement, AlertEngine, AlertEvent, CellId, CellVote, Consensus, Level, RuleStatus,
};
use crate::config::Config;
use crate::model::{ProcessInfo, Snapshot};
use crate::theme::{Palette, ThemeName};
use std::collections::{BTreeMap, VecDeque};

/// Samples of history kept per series; enough for a sparkline across a 4K terminal.
pub const HISTORY_LEN: usize = 512;
/// Alert log entries kept.
pub const LOG_LEN: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Processes,
    Diagnostics,
}

impl Screen {
    pub const ALL: [Screen; 3] = [Screen::Overview, Screen::Processes, Screen::Diagnostics];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Overview => "OVERVIEW",
            Screen::Processes => "PROCESSES",
            Screen::Diagnostics => "DIAGNOSTICS",
        }
    }

    /// The number key that selects this screen.
    pub fn key(self) -> char {
        match self {
            Screen::Overview => '1',
            Screen::Processes => '2',
            Screen::Diagnostics => '3',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Cpu,
    Memory,
    Pid,
    Name,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Cpu => "CPU",
            SortKey::Memory => "MEM",
            SortKey::Pid => "PID",
            SortKey::Name => "NAME",
        }
    }

    pub fn next(self) -> SortKey {
        match self {
            SortKey::Cpu => SortKey::Memory,
            SortKey::Memory => SortKey::Pid,
            SortKey::Pid => SortKey::Name,
            SortKey::Name => SortKey::Cpu,
        }
    }

    /// The natural direction: biggest first for CPU and memory, ascending for PID and name.
    pub fn default_descending(self) -> bool {
        matches!(self, SortKey::Cpu | SortKey::Memory)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    /// Typing a process filter after `/`.
    Filter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessView {
    pub sort: SortKey,
    pub descending: bool,
    pub filter: String,
    pub mode: InputMode,
    /// The selected process; kept across refreshes while that PID exists.
    pub selected_pid: Option<u32>,
    /// Index of the first visible row on the processes screen.
    pub offset: usize,
}

impl Default for ProcessView {
    fn default() -> Self {
        ProcessView {
            sort: SortKey::Cpu,
            descending: true,
            filter: String::new(),
            mode: InputMode::Normal,
            selected_pid: None,
            offset: 0,
        }
    }
}

/// A bounded series of samples, oldest first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Series {
    values: VecDeque<f64>,
}

impl Series {
    pub fn push(&mut self, value: f64) {
        if self.values.len() == HISTORY_LEN {
            self.values.pop_front();
        }
        self.values.push_back(value);
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn last(&self) -> Option<f64> {
        self.values.back().copied()
    }

    /// The newest `n` samples, oldest first.
    pub fn tail(&self, n: usize) -> impl Iterator<Item = f64> + '_ {
        self.values
            .iter()
            .skip(self.values.len().saturating_sub(n))
            .copied()
    }

    pub fn max(&self) -> Option<f64> {
        self.values.iter().copied().reduce(f64::max)
    }
}

/// History for sparklines. A series only receives a sample when its source was available.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct History {
    pub cpu_total: Series,
    pub memory_percent: Series,
    /// Sum over all non-loopback interfaces, bytes per second.
    pub net_rx_bps: Series,
    pub net_tx_bps: Series,
    /// Sum over all listed disks, bytes per second.
    pub disk_read_bps: Series,
    pub disk_write_bps: Series,
    /// GPU utilisation keyed by PCI slot.
    pub gpu_util: BTreeMap<String, Series>,
}

#[derive(Debug, Clone)]
pub struct App {
    pub engine: AlertEngine,
    pub page_size: usize,
    pub config: Config,
    pub theme: ThemeName,
    pub palette: Palette,
    pub snapshot: Option<Snapshot>,
    pub history: History,
    pub screen: Screen,
    pub show_help: bool,
    pub paused: bool,
    pub interval_ms: u64,
    pub procs: ProcessView,
    /// Shared row offset for diagnostics tables; each table clamps to its final page.
    pub diagnostics_offset: usize,
    /// Every rule status from the latest evaluation, in `RuleId::ALL` order.
    pub rules: Vec<RuleStatus>,
    pub consensus: Consensus,
    /// Newest last.
    pub log: VecDeque<AlertEvent>,
    /// True after `a` until a rule rises to a new, higher level.
    pub acknowledged: bool,
    pub should_quit: bool,
    /// Local UTC offset at the latest snapshot, for the header clock. `None`
    /// when the C library cannot report it, in which case no clock is shown.
    pub utc_offset_secs: Option<i32>,
}

impl App {
    /// A fresh app with no data yet. `palette` must be the config's palette,
    /// already validated by `Config::validate`.
    pub fn new(config: Config, palette: Palette) -> App {
        App {
            engine: AlertEngine::default(),
            page_size: 20,
            theme: config.theme.name,
            interval_ms: config.general.interval_ms,
            palette,
            config,
            snapshot: None,
            history: History::default(),
            screen: Screen::Overview,
            show_help: false,
            paused: false,
            procs: ProcessView::default(),
            diagnostics_offset: 0,
            rules: Vec::new(),
            consensus: Consensus::unknown(),
            log: VecDeque::new(),
            acknowledged: false,
            should_quit: false,
            utc_offset_secs: None,
        }
    }

    /// Number of rows in the longest diagnostics table.
    pub fn diagnostics_len(&self) -> usize {
        let mut lengths = vec![
            self.rules.len().max(crate::alert::RuleId::ALL.len()),
            self.log.len(),
        ];
        if let Some(s) = &self.snapshot {
            lengths.extend([
                s.sensors
                    .value()
                    .map_or(1, |s| s.temps.len() + s.fans.len()),
                s.gpus.value().map_or(1, Vec::len),
                if s.battery.is_available() { 8 } else { 1 },
                s.disks.value().map_or(1, Vec::len) + s.network.value().map_or(1, Vec::len),
            ]);
        }
        lengths.into_iter().max().unwrap_or(0)
    }

    /// Processes after the kernel-thread setting and the name filter, in the
    /// current sort order. Ties break by PID so the order is stable.
    pub fn visible_processes(&self) -> Vec<&ProcessInfo> {
        let Some(list) = self.snapshot.as_ref().and_then(|s| s.processes.value()) else {
            return Vec::new();
        };
        let needle = self.procs.filter.to_lowercase();
        let hide_kernel = self.config.general.hide_kernel_threads;
        let mut out: Vec<&ProcessInfo> = list
            .iter()
            .filter(|p| !(hide_kernel && p.kernel_thread))
            .filter(|p| needle.is_empty() || p.name.to_lowercase().contains(&needle))
            .collect();
        let key = self.procs.sort;
        out.sort_by(|a, b| {
            let ord = match key {
                SortKey::Cpu => a.cpu_percent.total_cmp(&b.cpu_percent),
                SortKey::Memory => a.memory_bytes.cmp(&b.memory_bytes),
                SortKey::Pid => a.pid.cmp(&b.pid),
                SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            };
            let ord = if self.procs.descending {
                ord.reverse()
            } else {
                ord
            };
            ord.then(a.pid.cmp(&b.pid))
        });
        out
    }

    /// The worst current rule level, for the banner.
    pub fn worst_level(&self) -> Level {
        self.consensus.verdict
    }
}

impl Consensus {
    /// The consensus before any data exists: every cell has no data.
    pub fn unknown() -> Consensus {
        Consensus {
            cells: CellId::ALL.map(|cell| CellVote {
                cell,
                vote: Level::Unknown,
                reason: None,
            }),
            verdict: Level::Unknown,
            agreement: Agreement::Unanimous,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Redraw,
    Quit,
    IntervalChanged(u64),
    PauseChanged(bool),
}

impl App {
    pub fn apply_snapshot(&mut self, s: Snapshot) {
        let old_index = self.selected_index();
        let eval = self.engine.evaluate(&s, &self.config.alerts);
        if eval
            .events
            .iter()
            .any(|e| e.to >= Level::Caution && e.to > e.from)
        {
            self.acknowledged = false;
        }
        self.rules = eval.rules;
        self.consensus = eval.consensus;
        self.log.extend(eval.events);
        while self.log.len() > LOG_LEN {
            self.log.pop_front();
        }
        if let Some(c) = s.cpu.value() {
            self.history.cpu_total.push(c.total as f64);
        }
        if let Some(v) = s.memory.value().and_then(|m| m.used_percent()) {
            self.history.memory_percent.push(v);
        }
        if let Some(nets) = s.network.value() {
            push_sum(
                &mut self.history.net_rx_bps,
                nets.iter().map(|n| n.rx_bps.value().copied()),
            );
            push_sum(
                &mut self.history.net_tx_bps,
                nets.iter().map(|n| n.tx_bps.value().copied()),
            );
        }
        if let Some(disks) = s.disks.value() {
            push_sum(
                &mut self.history.disk_read_bps,
                disks.iter().map(|d| d.read_bps.value().copied()),
            );
            push_sum(
                &mut self.history.disk_write_bps,
                disks.iter().map(|d| d.write_bps.value().copied()),
            );
        }
        if let Some(gpus) = s.gpus.value() {
            self.history
                .gpu_util
                .retain(|slot, _| gpus.iter().any(|g| &g.pci_slot == slot));
            for g in gpus {
                if let Some(v) = g.utilization_percent.value() {
                    self.history
                        .gpu_util
                        .entry(g.pci_slot.clone())
                        .or_default()
                        .push(*v as f64);
                }
            }
        }
        self.utc_offset_secs = crate::clock::local_offset_secs((s.taken_at_ms / 1000) as i64);
        self.snapshot = Some(s);
        self.diagnostics_offset = self
            .diagnostics_offset
            .min(self.diagnostics_len().saturating_sub(1));
        self.clamp_selection(old_index);
    }

    /// Updates the process viewport after a terminal resize.
    pub fn set_page_size(&mut self, rows: usize) {
        self.page_size = rows.max(1);
        self.clamp_selection(self.selected_index());
    }

    fn selected_index(&self) -> usize {
        self.visible_processes()
            .iter()
            .position(|p| Some(p.pid) == self.procs.selected_pid)
            .unwrap_or(0)
    }

    fn clamp_selection(&mut self, fallback: usize) {
        let list = self.visible_processes();
        let len = list.len();
        let index = list
            .iter()
            .position(|p| Some(p.pid) == self.procs.selected_pid)
            .unwrap_or(fallback.min(len.saturating_sub(1)));
        self.procs.selected_pid = list.get(index).map(|p| p.pid);
        self.procs.offset = self
            .procs
            .offset
            .min(len.saturating_sub(self.page_size.max(1)));
        if index < self.procs.offset {
            self.procs.offset = index;
        }
        if index >= self.procs.offset + self.page_size.max(1) {
            self.procs.offset = index + 1 - self.page_size.max(1);
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Effect {
        let before = (
            self.screen,
            self.show_help,
            self.procs.clone(),
            self.diagnostics_offset,
            self.acknowledged,
            self.theme,
        );
        let effect = self.dispatch_key(key);
        let after = (
            self.screen,
            self.show_help,
            self.procs.clone(),
            self.diagnostics_offset,
            self.acknowledged,
            self.theme,
        );
        if effect == Effect::Redraw && before == after {
            return Effect::None;
        }
        effect
    }

    fn dispatch_key(&mut self, key: crossterm::event::KeyEvent) -> Effect {
        use crossterm::event::{KeyCode as K, KeyEventKind, KeyModifiers};
        if key.kind == KeyEventKind::Release {
            return Effect::None;
        }
        if self.show_help {
            self.show_help = false;
            return Effect::Redraw;
        }
        if key.code == K::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return Effect::Quit;
        }
        if self.procs.mode == InputMode::Filter {
            match key.code {
                K::Esc => {
                    self.procs.filter.clear();
                    self.procs.mode = InputMode::Normal;
                }
                K::Enter => self.procs.mode = InputMode::Normal,
                K::Backspace => {
                    self.procs.filter.pop();
                }
                K::Char(c)
                    if !c.is_control()
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.procs.filter.push(c)
                }
                _ => return Effect::None,
            }
            self.clamp_selection(0);
            return Effect::Redraw;
        }
        match key.code {
            K::Char('q') => {
                self.should_quit = true;
                return Effect::Quit;
            }
            K::Char('?') | K::F(1) => self.show_help = true,
            K::Char('1') => self.screen = Screen::Overview,
            K::Char('2') => self.screen = Screen::Processes,
            K::Char('3') => self.screen = Screen::Diagnostics,
            K::Tab | K::BackTab => {
                let i = Screen::ALL
                    .iter()
                    .position(|s| *s == self.screen)
                    .unwrap_or(0);
                let back = key.code == K::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
                self.screen = Screen::ALL[(i + if back { 2 } else { 1 }) % 3];
            }
            K::Char(' ') => {
                self.paused = !self.paused;
                self.engine.reset_pending();
                return Effect::PauseChanged(self.paused);
            }
            K::Char(c @ ('+' | '-')) => {
                let ladder = [250, 500, 1000, 2000, 5000, 10000];
                let next = if c == '+' {
                    ladder
                        .into_iter()
                        .find(|v| *v > self.interval_ms)
                        .unwrap_or(10000)
                } else {
                    ladder
                        .into_iter()
                        .rev()
                        .find(|v| *v < self.interval_ms)
                        .unwrap_or(250)
                };
                if next == self.interval_ms {
                    return Effect::None;
                }
                self.interval_ms = next;
                return Effect::IntervalChanged(next);
            }
            K::Char('t') => {
                self.theme = self.theme.next();
                self.palette = if self.theme == self.config.theme.name {
                    crate::config::palette_for(self.theme, &self.config.theme.colors)
                        .expect("validated theme overrides")
                } else {
                    Palette::builtin(self.theme)
                };
            }
            K::Char('s') => {
                self.procs.sort = self.procs.sort.next();
                self.procs.descending = self.procs.sort.default_descending();
                self.clamp_selection(0);
            }
            K::Char('r') => {
                self.procs.descending = !self.procs.descending;
                self.clamp_selection(0);
            }
            K::Char('/') => {
                // The filter editor lives on the processes screen.
                self.screen = Screen::Processes;
                self.procs.mode = InputMode::Filter;
            }
            K::Char('a') => self.acknowledged = true,
            K::Esc => {
                // A filter kept with Enter stays until Esc clears it.
                self.procs.filter.clear();
                self.clamp_selection(0);
            }
            K::Char('j' | 'k' | 'g' | 'G') | K::Down | K::Up | K::PageDown | K::PageUp
                if self.screen == Screen::Diagnostics =>
            {
                let last = self.diagnostics_len().saturating_sub(1);
                self.diagnostics_offset = match key.code {
                    K::Char('j') | K::Down => self.diagnostics_offset.saturating_add(1),
                    K::Char('k') | K::Up => self.diagnostics_offset.saturating_sub(1),
                    K::PageDown => self
                        .diagnostics_offset
                        .saturating_add(self.page_size.max(1)),
                    K::PageUp => self
                        .diagnostics_offset
                        .saturating_sub(self.page_size.max(1)),
                    K::Char('g') => 0,
                    _ => last,
                }
                .min(last);
            }
            K::Char('j' | 'k' | 'g' | 'G') | K::Down | K::Up | K::PageDown | K::PageUp => {
                let len = self.visible_processes().len();
                let i = self.selected_index();
                let index = match key.code {
                    K::Char('j') | K::Down => i.saturating_add(1),
                    K::Char('k') | K::Up => i.saturating_sub(1),
                    K::PageDown => i.saturating_add(self.page_size.max(1)),
                    K::PageUp => i.saturating_sub(self.page_size.max(1)),
                    K::Char('g') => 0,
                    _ => len.saturating_sub(1),
                }
                .min(len.saturating_sub(1));
                self.procs.selected_pid = self.visible_processes().get(index).map(|p| p.pid);
                self.clamp_selection(index);
            }
            _ => return Effect::None,
        }
        Effect::Redraw
    }
}

/// Records the sum only when every source reported a value: a sum over
/// some sources is not a total.
fn push_sum(series: &mut Series, values: impl Iterator<Item = Option<f64>>) {
    let values: Option<Vec<f64>> = values.collect();
    if let Some(sum) = values.and_then(|v| v.into_iter().reduce(|a, b| a + b)) {
        series.push(sum);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn proc(pid: u32, name: &str, cpu: f32, mem: u64, kernel: bool) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.into(),
            user: "u".into(),
            cpu_percent: cpu,
            memory_bytes: mem,
            state: "sleep".into(),
            kernel_thread: kernel,
        }
    }

    fn app_with(procs: Vec<ProcessInfo>) -> App {
        let config = Config::default();
        let palette = config.palette().unwrap();
        let mut app = App::new(config, palette);
        app.snapshot = Some(Snapshot {
            taken_at_ms: 0,
            seq: 1,
            host: HostInfo {
                hostname: Reading::Available("h".into()),
                kernel: Reading::Available("k".into()),
                uptime_secs: Reading::Available(1),
            },
            cpu: Reading::unavailable("warming up"),
            memory: Reading::unavailable("x"),
            disks: Reading::unavailable("x"),
            network: Reading::unavailable("x"),
            processes: Reading::Available(procs),
            sensors: Reading::unavailable("x"),
            gpus: Reading::unavailable("x"),
            battery: Reading::unavailable("x"),
        });
        app
    }

    #[test]
    fn visible_processes_sort_filter_and_hide_kernel_threads() {
        let mut app = app_with(vec![
            proc(10, "cargo", 50.0, 10, false),
            proc(2, "kworker/0:1", 90.0, 0, true),
            proc(7, "Firefox", 50.0, 900, false),
            proc(3, "zsh", 1.0, 5, false),
        ]);
        let pids = |a: &App| {
            a.visible_processes()
                .iter()
                .map(|p| p.pid)
                .collect::<Vec<_>>()
        };
        assert_eq!(pids(&app), vec![7, 10, 3], "cpu desc, ties by pid");
        app.procs.sort = SortKey::Name;
        app.procs.descending = false;
        assert_eq!(pids(&app), vec![10, 7, 3], "case-insensitive name order");
        app.procs.filter = "FIRE".into();
        assert_eq!(pids(&app), vec![7]);
        app.procs.filter.clear();
        app.config.general.hide_kernel_threads = false;
        app.procs.sort = SortKey::Pid;
        assert_eq!(pids(&app), vec![2, 3, 7, 10]);
    }

    #[test]
    fn series_is_bounded_and_tail_is_oldest_first() {
        let mut s = Series::default();
        for i in 0..(HISTORY_LEN + 5) {
            s.push(i as f64);
        }
        assert_eq!(s.len(), HISTORY_LEN);
        assert_eq!(s.last(), Some((HISTORY_LEN + 4) as f64));
        let t: Vec<f64> = s.tail(3).collect();
        assert_eq!(
            t,
            vec![
                (HISTORY_LEN + 2) as f64,
                (HISTORY_LEN + 3) as f64,
                (HISTORY_LEN + 4) as f64
            ]
        );
    }
}
