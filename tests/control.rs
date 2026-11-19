use crossterm::event::{KeyCode as K, KeyEvent, KeyModifiers as M};
use nervtop::{
    alert::*,
    app::*,
    cli::{self, Command},
    config::*,
    format::*,
    model::*,
    report,
    runtime::{self, AppEvent, Step},
    theme::*,
};
fn snapshot() -> Snapshot {
    Snapshot {
        taken_at_ms: 123,
        seq: 1,
        host: HostInfo {
            hostname: Reading::Available("host".into()),
            kernel: Reading::unavailable("kernel missing"),
            uptime_secs: Reading::Available(3600),
        },
        cpu: Reading::Available(CpuStats {
            total: 10.,
            per_core: vec![10., 10.],
            load_avg: Reading::Available([1., 2., 3.]),
            freq_mhz: None,
            brand: None,
        }),
        memory: Reading::Available(MemoryStats {
            total_bytes: 100,
            available_bytes: 50,
            swap_total_bytes: 0,
            swap_used_bytes: 0,
        }),
        disks: Reading::Available(vec![]),
        network: Reading::unavailable("net missing"),
        processes: Reading::Available(vec![]),
        sensors: Reading::Available(SensorStats {
            temps: vec![],
            fans: vec![],
        }),
        gpus: Reading::Available(vec![]),
        battery: Reading::unavailable("no battery present"),
    }
}
fn app() -> App {
    let c = Config::default();
    let p = c.palette().unwrap();
    App::new(c, p)
}
fn key(a: &mut App, k: K) -> Effect {
    a.handle_key(KeyEvent::new(k, M::NONE))
}
fn parse(s: &str) -> Result<Command, cli::CliError> {
    cli::parse(s.split_whitespace().map(str::to_owned))
}
fn opts(s: &str) -> cli::Options {
    match parse(s).unwrap() {
        Command::Run(o) => o,
        _ => panic!("expected options"),
    }
}
fn cpu(engine: &mut AlertEngine, value: f32, c: &AlertConfig) -> Evaluation {
    let mut s = snapshot();
    if let Reading::Available(ref mut cpu) = s.cpu {
        cpu.total = value;
    }
    engine.evaluate(&s, c)
}
fn status(e: &Evaluation, id: RuleId) -> &RuleStatus {
    e.rules.iter().find(|r| r.rule == id).unwrap()
}
fn instant() -> AlertConfig {
    AlertConfig {
        sustain: 1,
        ..AlertConfig::default()
    }
}
fn battery(percent: f64, status: BatteryStatus) -> BatteryStats {
    BatteryStats {
        name: "BAT0".into(),
        percent,
        status,
        ac_online: None,
        power_watts: Reading::unavailable("no power"),
        time_remaining_secs: Reading::unavailable("no rate"),
        health_percent: Reading::unavailable("no design"),
        charge_limit_percent: None,
        cycle_count: None,
    }
}
fn processes(ids: &[u32]) -> Snapshot {
    let mut s = snapshot();
    s.processes = Reading::Available(
        ids.iter()
            .map(|id| ProcessInfo {
                pid: *id,
                name: format!("p{id}"),
                user: "u".into(),
                cpu_percent: *id as f32,
                memory_bytes: *id as u64,
                state: "run".into(),
                kernel_thread: false,
            })
            .collect(),
    );
    s
}
#[test]
fn bytes_zero() {
    assert_eq!(fmt_bytes(0), "0.0 B");
}
#[test]
fn bytes_kib() {
    assert_eq!(fmt_bytes(1024), "1.0 KiB");
}
#[test]
fn bytes_mib() {
    assert_eq!(fmt_bytes(1048576), "1.0 MiB");
}
#[test]
fn bytes_gib() {
    assert_eq!(fmt_bytes(1073741824), "1.0 GiB");
}
#[test]
fn bytes_max() {
    assert!(fmt_bytes(u64::MAX).ends_with("EiB"));
}
#[test]
fn rate_fraction() {
    assert_eq!(fmt_rate(512.), "512.0 B/s");
}
#[test]
fn percent_fraction() {
    assert_eq!(fmt_percent(12.34), "12.3%");
}
#[test]
fn duration_zero() {
    assert_eq!(fmt_duration(0), "00:00:00");
}
#[test]
fn duration_hour() {
    assert_eq!(fmt_duration(3661), "01:01:01");
}
#[test]
fn duration_days() {
    assert_eq!(fmt_duration(90061), "1d 01:01:01");
}
#[test]
fn cli_defaults() {
    assert_eq!(opts("").size, (120, 40));
}
#[test]
fn cli_config() {
    assert_eq!(
        opts("--config /tmp/a").config.unwrap().to_str(),
        Some("/tmp/a")
    );
}
#[test]
fn cli_config_equals() {
    assert_eq!(
        opts("--config=/tmp/a").config.unwrap().to_str(),
        Some("/tmp/a")
    );
}
#[test]
fn cli_interval() {
    assert_eq!(opts("--interval 250").interval_ms, Some(250));
}
#[test]
fn cli_interval_equals() {
    assert_eq!(opts("--interval=10000").interval_ms, Some(10000));
}
#[test]
fn cli_theme() {
    assert_eq!(opts("--theme phosphor").theme, Some(ThemeName::Phosphor));
}
#[test]
fn cli_theme_equals() {
    assert_eq!(opts("--theme=ansi").theme, Some(ThemeName::Ansi));
}
#[test]
fn cli_once_json() {
    let o = opts("--once --json");
    assert!(o.once && o.json);
}
#[test]
fn cli_screenshot() {
    let o = opts("--screenshot a.svg --size 80x24 --screen diagnostics");
    assert_eq!(o.size, (80, 24));
    assert_eq!(o.screen, Screen::Diagnostics);
}
#[test]
fn cli_screenshot_equals() {
    let o = opts("--screenshot=a.svg --size=500x200 --screen=processes");
    assert_eq!(o.size, (500, 200));
    assert_eq!(o.screen, Screen::Processes);
}
#[test]
fn cli_help() {
    for s in ["-h", "--help"] {
        assert_eq!(parse(s).unwrap(), Command::Help);
    }
}
#[test]
fn cli_version() {
    for s in ["-V", "--version"] {
        assert_eq!(parse(s).unwrap(), Command::Version);
    }
}
#[test]
fn cli_print() {
    assert_eq!(parse("--print-config").unwrap(), Command::PrintConfig);
}
#[test]
fn cli_unknown() {
    assert!(parse("--wat").is_err());
}
#[test]
fn cli_missing() {
    for f in [
        "--config",
        "--interval",
        "--theme",
        "--size",
        "--screen",
        "--screenshot",
    ] {
        assert!(parse(f).is_err());
    }
}
#[test]
fn cli_empty() {
    assert!(parse("--config=").is_err());
}
#[test]
fn cli_interval_invalid() {
    for v in ["249", "10001", "x", "-1"] {
        assert!(parse(&format!("--interval={v}")).is_err());
    }
}
#[test]
fn cli_size_invalid() {
    for v in [
        "79x24",
        "80x23",
        "501x24",
        "80x201",
        "axb",
        "80",
        "999999x24",
    ] {
        assert!(parse(&format!("--screenshot a --size={v}")).is_err());
    }
}
#[test]
fn cli_requires_screenshot() {
    for s in ["--size=80x24", "--screen overview", "--samples 5"] {
        assert!(parse(s).is_err());
    }
}
#[test]
fn cli_samples() {
    assert_eq!(opts("--screenshot a.svg").samples, 4);
    assert_eq!(opts("--screenshot a.svg --samples 60").samples, 60);
    assert_eq!(opts("--screenshot=a.svg --samples=1").samples, 1);
    for v in ["0", "601", "x", "-1"] {
        assert!(
            parse(&format!("--screenshot a --samples={v}")).is_err(),
            "{v}"
        );
    }
}
#[test]
fn cli_redact_needs_once_or_screenshot() {
    assert!(opts("--once --redact").redact);
    assert!(opts("--screenshot a.svg --redact").redact);
    assert!(parse("--redact").is_err());
}
#[test]
fn cli_once_and_screenshot_are_exclusive() {
    assert!(parse("--once --screenshot a.svg").is_err());
}
#[test]
fn cli_requires_once() {
    assert!(parse("--json").is_err());
}
#[test]
fn cli_bad_names() {
    assert!(parse("--theme purple").is_err());
    assert!(parse("--screenshot a --screen x").is_err());
}
#[test]
fn cli_bool_value() {
    assert!(parse("--once=true").is_err());
}
#[test]
fn above_warn_boundary() {
    assert_eq!(
        status(
            &cpu(&mut AlertEngine::default(), 85., &instant()),
            RuleId::CpuUsage
        )
        .level,
        Level::Caution
    );
}
#[test]
fn above_critical_boundary() {
    assert_eq!(
        status(
            &cpu(&mut AlertEngine::default(), 95., &instant()),
            RuleId::CpuUsage
        )
        .level,
        Level::Critical
    );
}
#[test]
fn above_nominal() {
    assert_eq!(
        status(
            &cpu(&mut AlertEngine::default(), 84., &instant()),
            RuleId::CpuUsage
        )
        .level,
        Level::Nominal
    );
}
#[test]
fn sustain_three() {
    let mut e = AlertEngine::default();
    let c = AlertConfig::default();
    for _ in 0..2 {
        assert_eq!(
            status(&cpu(&mut e, 99., &c), RuleId::CpuUsage).level,
            Level::Nominal
        );
    }
    assert_eq!(
        status(&cpu(&mut e, 99., &c), RuleId::CpuUsage).level,
        Level::Critical
    );
}
#[test]
fn sustain_crit_independent() {
    let mut e = AlertEngine::default();
    let c = AlertConfig::default();
    cpu(&mut e, 90., &c);
    cpu(&mut e, 90., &c);
    assert_eq!(
        status(&cpu(&mut e, 99., &c), RuleId::CpuUsage).level,
        Level::Caution
    );
}
#[test]
fn hysteresis_above() {
    let mut e = AlertEngine::default();
    let c = instant();
    cpu(&mut e, 95., &c);
    for v in [94., 93., 94., 93.] {
        assert_eq!(
            status(&cpu(&mut e, v, &c), RuleId::CpuUsage).level,
            Level::Critical
        );
    }
    assert_eq!(
        status(&cpu(&mut e, 92., &c), RuleId::CpuUsage).level,
        Level::Caution
    );
}
#[test]
fn immediate_fall() {
    let mut e = AlertEngine::default();
    let c = AlertConfig::default();
    for _ in 0..3 {
        cpu(&mut e, 99., &c);
    }
    assert_eq!(
        status(&cpu(&mut e, 20., &c), RuleId::CpuUsage).level,
        Level::Nominal
    );
}
#[test]
fn unknown_resets() {
    let mut e = AlertEngine::default();
    let c = AlertConfig::default();
    cpu(&mut e, 99., &c);
    cpu(&mut e, 99., &c);
    let mut s = snapshot();
    s.cpu = Reading::unavailable("warming up");
    assert_eq!(
        status(&e.evaluate(&s, &c), RuleId::CpuUsage).level,
        Level::Unknown
    );
    assert_eq!(
        status(&cpu(&mut e, 99., &c), RuleId::CpuUsage).level,
        Level::Nominal
    );
}
#[test]
fn startup_events_suppressed() {
    let mut e = AlertEngine::default();
    let c = instant();
    assert!(cpu(&mut e, 10., &c).events.is_empty());
    let mut s = snapshot();
    s.cpu = Reading::unavailable("missing");
    assert!(e.evaluate(&s, &c).events.is_empty());
}
#[test]
fn escalation_event() {
    let e = cpu(&mut AlertEngine::default(), 99., &instant());
    let event = e
        .events
        .iter()
        .find(|e| e.rule == RuleId::CpuUsage)
        .unwrap();
    assert_eq!(event.at_ms, 123);
    assert_eq!(event.from, Level::Unknown);
    assert_eq!(event.to, Level::Critical);
}
#[test]
fn swap_zero_omitted() {
    assert!(
        !AlertEngine::default()
            .evaluate(&snapshot(), &instant())
            .rules
            .iter()
            .any(|r| r.rule == RuleId::Swap)
    );
}
#[test]
fn disabled_omitted() {
    let mut c = instant();
    c.cpu_usage.enabled = false;
    assert!(
        !cpu(&mut AlertEngine::default(), 99., &c)
            .rules
            .iter()
            .any(|r| r.rule == RuleId::CpuUsage)
    );
}
#[test]
fn battery_below_boundaries() {
    for (v, l) in [
        (20., Level::Caution),
        (10., Level::Critical),
        (21., Level::Nominal),
    ] {
        let mut s = snapshot();
        s.battery = Reading::Available(battery(v, BatteryStatus::Discharging));
        assert_eq!(
            status(
                &AlertEngine::default().evaluate(&s, &instant()),
                RuleId::Battery
            )
            .level,
            l
        );
    }
}
#[test]
fn battery_not_discharging() {
    for state in [
        BatteryStatus::Charging,
        BatteryStatus::Full,
        BatteryStatus::NotCharging,
    ] {
        let mut s = snapshot();
        s.battery = Reading::Available(battery(1., state));
        let e = AlertEngine::default().evaluate(&s, &instant());
        assert_eq!(status(&e, RuleId::Battery).level, Level::Nominal);
        assert_eq!(status(&e, RuleId::Battery).value, Some(1.));
    }
}
#[test]
fn battery_unknown_status_follows_ac_power() {
    let judge = |ac_online: Option<bool>| {
        let mut b = battery(5., BatteryStatus::Unknown);
        b.ac_online = ac_online;
        let mut s = snapshot();
        s.battery = Reading::Available(b);
        status(
            &AlertEngine::default().evaluate(&s, &instant()),
            RuleId::Battery,
        )
        .clone()
    };
    // On battery power an unknown status is judged as discharging.
    let off_ac = judge(Some(false));
    assert_eq!(off_ac.level, Level::Critical);
    assert_eq!(off_ac.value, Some(5.));
    // On AC it is nominal, like charging.
    let on_ac = judge(Some(true));
    assert_eq!(on_ac.level, Level::Nominal);
    assert_eq!(on_ac.value, Some(5.));
    // Without AC information nothing can be said.
    let unknown = judge(None);
    assert_eq!(unknown.level, Level::Unknown);
    assert_eq!(unknown.value, None);
    assert_eq!(unknown.note.as_deref(), Some("battery status unknown"));
}
#[test]
fn battery_absent() {
    assert!(
        !AlertEngine::default()
            .evaluate(&snapshot(), &instant())
            .rules
            .iter()
            .any(|r| r.rule == RuleId::Battery)
    );
}
#[test]
fn battery_error() {
    let mut s = snapshot();
    s.battery = Reading::unavailable("permission denied");
    assert_eq!(
        status(
            &AlertEngine::default().evaluate(&s, &instant()),
            RuleId::Battery
        )
        .note
        .as_deref(),
        Some("permission denied")
    );
}
#[test]
fn below_hysteresis() {
    let mut s = snapshot();
    let mut e = AlertEngine::default();
    for (v, l) in [
        (10., Level::Critical),
        (12., Level::Critical),
        (13., Level::Caution),
        (22., Level::Caution),
        (23., Level::Nominal),
    ] {
        s.battery = Reading::Available(battery(v, BatteryStatus::Discharging));
        assert_eq!(
            status(&e.evaluate(&s, &instant()), RuleId::Battery).level,
            l
        );
    }
}
#[test]
fn disk_subjects_sorted_and_removed() {
    let mut s = snapshot();
    let disk = |mount: &str| DiskStats {
        mount_point: mount.into(),
        device: "dev".into(),
        file_system: "ext4".into(),
        total_bytes: 100,
        available_bytes: 1,
        read_bps: Reading::unavailable("warm"),
        write_bps: Reading::unavailable("warm"),
    };
    s.disks = Reading::Available(vec![disk("/z"), disk("/")]);
    let mut e = AlertEngine::default();
    let result = e.evaluate(&s, &instant());
    assert_eq!(
        result
            .rules
            .iter()
            .filter(|r| r.rule == RuleId::Disk)
            .map(|r| r.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["/", "/z"]
    );
    s.disks = Reading::Available(vec![]);
    assert!(
        !e.evaluate(&s, &instant())
            .events
            .iter()
            .any(|e| e.rule == RuleId::Disk)
    );
}
#[test]
fn hottest_sensor() {
    let mut s = snapshot();
    s.sensors = Reading::Available(SensorStats {
        temps: vec![TempSensor {
            chip: "cpu".into(),
            label: "hot".into(),
            kind: SensorKind::Cpu,
            celsius: 96.,
            crit_celsius: None,
        }],
        fans: vec![],
    });
    let e = AlertEngine::default().evaluate(&s, &instant());
    assert_eq!(status(&e, RuleId::CpuTemp).subject, "cpu hot");
    assert_eq!(e.consensus.cells[1].vote, Level::Critical);
}
#[test]
fn consensus_reason() {
    let e = cpu(&mut AlertEngine::default(), 99., &instant());
    assert_eq!(e.consensus.verdict, Level::Critical);
    assert!(
        e.consensus.cells[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("CPU LOAD")
    );
    assert_eq!(e.consensus.agreement, Agreement::ThreeWay);
}
#[test]
fn quit_keys() {
    let mut a = app();
    assert_eq!(key(&mut a, K::Char('q')), Effect::Quit);
    assert!(a.should_quit);
    let mut a = app();
    a.procs.mode = InputMode::Filter;
    assert_eq!(
        a.handle_key(KeyEvent::new(K::Char('c'), M::CONTROL)),
        Effect::Quit
    );
}
#[test]
fn help_swallows() {
    for help in [K::Char('?'), K::F(1)] {
        let mut a = app();
        key(&mut a, help);
        assert!(a.show_help);
        assert_eq!(key(&mut a, K::Char('q')), Effect::Redraw);
        assert!(!a.should_quit);
        assert!(!a.show_help);
    }
}
#[test]
fn screens_direct() {
    let mut a = app();
    for (k, s) in [
        ('2', Screen::Processes),
        ('3', Screen::Diagnostics),
        ('1', Screen::Overview),
    ] {
        key(&mut a, K::Char(k));
        assert_eq!(a.screen, s);
    }
}
#[test]
fn screens_cycle() {
    let mut a = app();
    key(&mut a, K::Tab);
    assert_eq!(a.screen, Screen::Processes);
    key(&mut a, K::BackTab);
    assert_eq!(a.screen, Screen::Overview);
    key(&mut a, K::BackTab);
    assert_eq!(a.screen, Screen::Diagnostics);
    key(&mut a, K::Tab);
    assert_eq!(a.screen, Screen::Overview);
}
#[test]
fn pause_resume() {
    let mut a = app();
    assert_eq!(key(&mut a, K::Char(' ')), Effect::PauseChanged(true));
    assert_eq!(key(&mut a, K::Char(' ')), Effect::PauseChanged(false));
}
#[test]
fn intervals_clamped() {
    let mut a = app();
    for _ in 0..10 {
        key(&mut a, K::Char('+'));
    }
    assert_eq!(a.interval_ms, 10000);
    assert_eq!(key(&mut a, K::Char('+')), Effect::None);
    for _ in 0..10 {
        key(&mut a, K::Char('-'));
    }
    assert_eq!(a.interval_ms, 250);
    assert_eq!(key(&mut a, K::Char('-')), Effect::None);
}
#[test]
fn intervals_between_steps() {
    let mut a = app();
    a.interval_ms = 750;
    assert_eq!(key(&mut a, K::Char('+')), Effect::IntervalChanged(1000));
    a.interval_ms = 750;
    assert_eq!(key(&mut a, K::Char('-')), Effect::IntervalChanged(500));
}
#[test]
fn theme_overrides_return() {
    let mut c = Config::default();
    c.theme.colors.insert("accent".into(), "#010203".into());
    let p = c.palette().unwrap();
    let mut a = App::new(c, p);
    key(&mut a, K::Char('t'));
    assert_eq!(a.palette, Palette::builtin(ThemeName::Phosphor));
    key(&mut a, K::Char('t'));
    key(&mut a, K::Char('t'));
    assert_eq!(a.palette, p);
}
#[test]
fn sort_cycle_defaults() {
    let mut a = app();
    for (s, d) in [
        (SortKey::Memory, true),
        (SortKey::Pid, false),
        (SortKey::Name, false),
        (SortKey::Cpu, true),
    ] {
        key(&mut a, K::Char('s'));
        assert_eq!(a.procs.sort, s);
        assert_eq!(a.procs.descending, d);
    }
}
#[test]
fn reverse_sort() {
    let mut a = app();
    key(&mut a, K::Char('r'));
    assert!(!a.procs.descending);
    key(&mut a, K::Char('r'));
    assert!(a.procs.descending);
}
#[test]
fn filter_edit_keep() {
    let mut a = app();
    key(&mut a, K::Char('/'));
    key(&mut a, K::Char('q'));
    key(&mut a, K::Char('é'));
    key(&mut a, K::Backspace);
    key(&mut a, K::Enter);
    assert_eq!(a.procs.filter, "q");
    assert_eq!(a.procs.mode, InputMode::Normal);
    assert!(!a.should_quit);
}
#[test]
fn filter_escape() {
    let mut a = app();
    key(&mut a, K::Char('/'));
    key(&mut a, K::Char('a'));
    key(&mut a, K::Esc);
    assert!(a.procs.filter.is_empty());
    assert_eq!(a.procs.mode, InputMode::Normal);
}
#[test]
fn escape_clears_a_kept_filter() {
    let mut a = app();
    for k in [K::Char('/'), K::Char('a'), K::Enter] {
        key(&mut a, k);
    }
    assert_eq!(a.procs.filter, "a");
    assert_eq!(key(&mut a, K::Esc), Effect::Redraw);
    assert!(a.procs.filter.is_empty());
    assert_eq!(key(&mut a, K::Esc), Effect::None, "nothing left to clear");
}
#[test]
fn release_ignored() {
    let mut a = app();
    let mut k = KeyEvent::new(K::Char('q'), M::NONE);
    k.kind = crossterm::event::KeyEventKind::Release;
    assert_eq!(a.handle_key(k), Effect::None);
}
#[test]
fn unknown_key_ignored() {
    assert_eq!(key(&mut app(), K::F(12)), Effect::None);
}
#[test]
fn selection_movement() {
    let mut a = app();
    a.apply_snapshot(processes(&[1, 2, 3]));
    assert_eq!(a.procs.selected_pid, Some(3));
    for (k, pid) in [
        (K::Char('j'), 2),
        (K::Down, 1),
        (K::Char('k'), 2),
        (K::Up, 3),
        (K::Char('G'), 1),
        (K::Char('g'), 3),
    ] {
        key(&mut a, k);
        assert_eq!(a.procs.selected_pid, Some(pid));
    }
}
#[test]
fn selection_pages_offset() {
    let mut a = app();
    a.page_size = 2;
    a.apply_snapshot(processes(&[1, 2, 3, 4, 5]));
    key(&mut a, K::PageDown);
    assert_eq!(a.procs.selected_pid, Some(3));
    assert_eq!(a.procs.offset, 1);
    key(&mut a, K::Char('G'));
    assert_eq!(a.procs.offset, 3);
    key(&mut a, K::PageUp);
    assert_eq!(a.procs.selected_pid, Some(3));
    key(&mut a, K::Char('g'));
    assert_eq!(a.procs.offset, 0);
}
#[test]
fn selection_follows_pid() {
    let mut a = app();
    a.apply_snapshot(processes(&[1, 2, 3]));
    key(&mut a, K::Char('j'));
    a.apply_snapshot(processes(&[2, 4, 5]));
    assert_eq!(a.procs.selected_pid, Some(2));
    assert_eq!(a.procs.offset, 0);
}
#[test]
fn selection_vanished_index() {
    let mut a = app();
    a.apply_snapshot(processes(&[1, 2, 3]));
    key(&mut a, K::Char('j'));
    a.apply_snapshot(processes(&[1, 4, 5]));
    assert_eq!(a.procs.selected_pid, Some(4));
    a.apply_snapshot(processes(&[]));
    assert_eq!(a.procs.selected_pid, None);
    assert_eq!(a.procs.offset, 0);
}
#[test]
fn acknowledge_resets_escalation() {
    let mut a = app();
    a.config.alerts.sustain = 1;
    a.apply_snapshot(snapshot());
    key(&mut a, K::Char('a'));
    assert!(a.acknowledged);
    let mut s = snapshot();
    if let Reading::Available(ref mut c) = s.cpu {
        c.total = 99.;
    }
    a.apply_snapshot(s);
    assert!(!a.acknowledged);
}
#[test]
fn history_available_only() {
    let mut a = app();
    a.apply_snapshot(snapshot());
    assert_eq!(a.history.cpu_total.last(), Some(10.));
    assert_eq!(a.history.memory_percent.last(), Some(50.));
    assert!(a.history.net_rx_bps.is_empty());
    let mut s = snapshot();
    s.cpu = Reading::unavailable("missing");
    s.network = Reading::Available(vec![NetInterface {
        name: "eth0".into(),
        rx_bps: Reading::Available(5.),
        tx_bps: Reading::unavailable("warm"),
        rx_total_bytes: 0,
        tx_total_bytes: 0,
    }]);
    a.apply_snapshot(s);
    assert_eq!(a.history.cpu_total.len(), 1);
    assert_eq!(a.history.net_rx_bps.last(), Some(5.));
    assert!(a.history.net_tx_bps.is_empty());
}
#[test]
fn log_bounded() {
    let mut a = app();
    a.config.alerts.sustain = 1;
    for i in 0..(LOG_LEN + 10) {
        let mut s = snapshot();
        if let Reading::Available(ref mut c) = s.cpu {
            c.total = if i % 2 == 0 { 99. } else { 1. };
        }
        a.apply_snapshot(s);
    }
    assert_eq!(a.log.len(), LOG_LEN);
}
#[test]
fn runtime_step_quit() {
    let mut a = app();
    assert_eq!(runtime::step(&mut a, AppEvent::Quit).unwrap(), Step::Quit);
    assert!(a.should_quit);
}
#[test]
fn runtime_step_resize() {
    assert_eq!(
        runtime::step(&mut app(), AppEvent::Resize).unwrap(),
        Step::Draw
    );
}
#[test]
fn runtime_step_snapshot() {
    let mut a = app();
    assert_eq!(
        runtime::step(&mut a, AppEvent::Snapshot(Box::new(snapshot()))).unwrap(),
        Step::Draw
    );
    assert_eq!(a.snapshot.unwrap().seq, 1);
}
#[test]
fn runtime_paused_discards() {
    let mut a = app();
    a.paused = true;
    assert_eq!(
        runtime::step(&mut a, AppEvent::Snapshot(Box::new(snapshot()))).unwrap(),
        Step::Idle
    );
    assert!(a.snapshot.is_none());
}
#[test]
fn runtime_step_effects() {
    let mut a = app();
    for (k, e) in [
        (K::Char('+'), Step::Interval(2000)),
        (K::Char(' '), Step::Pause(true)),
        (K::F(12), Step::Idle),
    ] {
        assert_eq!(
            runtime::step(&mut a, AppEvent::Key(KeyEvent::new(k, M::NONE))).unwrap(),
            e
        );
    }
}
#[test]
fn runtime_dead_collector() {
    assert!(
        runtime::step(&mut app(), AppEvent::CollectorStopped)
            .unwrap_err()
            .to_string()
            .contains("collector stopped")
    );
}
#[test]
fn runtime_input_error() {
    let e = runtime::step(
        &mut app(),
        AppEvent::InputError(std::io::Error::other("input failed")),
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "input failed");
}
#[test]
fn report_text_missing_and_available() {
    let s = snapshot();
    let e = AlertEngine::default().evaluate(&s, &instant());
    let out = report::text(&s, &e);
    assert!(
        out.contains("kernel          unavailable (kernel missing)"),
        "{out}"
    );
    assert!(out.contains("total           10.0%"), "{out}");
    assert!(
        out.contains("network         unavailable (net missing)"),
        "{out}"
    );
    assert!(out.contains("CONSENSUS:"));
}
#[test]
fn report_json_missing_no_number() {
    let s = snapshot();
    let e = AlertEngine::default().evaluate(&s, &instant());
    let v: serde_json::Value = serde_json::from_str(&report::json(&s, &e)).unwrap();
    assert_eq!(v["snapshot"]["network"]["status"], "unavailable");
    assert_eq!(v["snapshot"]["network"]["value"], "net missing");
    assert!(v["snapshot"]["network"]["value"].as_f64().is_none());
    assert_eq!(v["snapshot"]["cpu"]["value"]["total"], 10.);
}
fn gpu(slot: &str, power_state: GpuPower, temp: f64) -> GpuStats {
    GpuStats {
        pci_slot: slot.into(),
        vendor: GpuVendor::Nvidia,
        name: "adapter".into(),
        power_state,
        utilization_percent: Reading::Available(20.),
        memory_used_bytes: Reading::unavailable("memory missing"),
        memory_total_bytes: Reading::Available(1024),
        temperature_celsius: Reading::Available(temp),
        power_watts: Reading::unavailable("power missing"),
    }
}
#[test]
fn gpu_suspended_excluded() {
    let mut s = snapshot();
    s.gpus = Reading::Available(vec![gpu("a", GpuPower::Suspended, 100.)]);
    let e = AlertEngine::default().evaluate(&s, &instant());
    let r = status(&e, RuleId::GpuTemp);
    assert_eq!(r.level, Level::Unknown);
    assert_eq!(r.note.as_deref(), Some("gpu suspended"));
}
#[test]
fn gpu_hottest_active_and_sensor() {
    let mut s = snapshot();
    s.gpus = Reading::Available(vec![
        gpu("sleep", GpuPower::Suspended, 120.),
        gpu("active", GpuPower::Active, 91.),
    ]);
    let e = AlertEngine::default().evaluate(&s, &instant());
    assert_eq!(status(&e, RuleId::GpuTemp).subject, "active adapter");
    if let Reading::Available(ref mut sensors) = s.sensors {
        sensors.temps.push(TempSensor {
            chip: "amdgpu".into(),
            label: "edge".into(),
            kind: SensorKind::Gpu,
            celsius: 99.,
            crit_celsius: None,
        });
    }
    let e = AlertEngine::default().evaluate(&s, &instant());
    assert_eq!(status(&e, RuleId::GpuTemp).subject, "amdgpu edge");
}
#[test]
fn gpu_history_removed_slots() {
    let mut a = app();
    let mut s = snapshot();
    s.gpus = Reading::Available(vec![gpu("a", GpuPower::Active, 10.)]);
    a.apply_snapshot(s);
    assert_eq!(a.history.gpu_util["a"].last(), Some(20.));
    a.apply_snapshot(snapshot());
    assert!(a.history.gpu_util.is_empty());
}
#[test]
fn resize_clamps_offset() {
    let mut a = app();
    a.apply_snapshot(processes(&[1, 2, 3, 4, 5]));
    key(&mut a, K::Char('G'));
    a.set_page_size(1);
    assert_eq!(a.procs.offset, 4);
    a.set_page_size(10);
    assert_eq!(a.procs.offset, 0);
    a.set_page_size(0);
    assert_eq!(a.page_size, 1);
}
#[test]
fn disk_history_sums_available() {
    let mut a = app();
    let mut s = snapshot();
    s.disks = Reading::Available(vec![DiskStats {
        mount_point: "/".into(),
        device: "dev".into(),
        file_system: "ext4".into(),
        total_bytes: 100,
        available_bytes: 50,
        read_bps: Reading::Available(20.),
        write_bps: Reading::unavailable("warming up"),
    }]);
    a.apply_snapshot(s);
    assert_eq!(a.history.disk_read_bps.last(), Some(20.));
    assert!(a.history.disk_write_bps.is_empty());
}
#[test]
fn full_report_nested_fields() {
    let mut s = snapshot();
    s.gpus = Reading::Available(vec![gpu("a", GpuPower::Active, 91.)]);
    s.battery = Reading::Available(battery(10., BatteryStatus::Discharging));
    let e = AlertEngine::default().evaluate(&s, &instant());
    let out = report::text(&s, &e);
    // Look a key up only inside its own section: GPU and battery both have
    // a "power" line.
    let line = |section: &str, key: &str| {
        let start = out
            .find(&format!("\n{section}\n"))
            .unwrap_or_else(|| panic!("no {section} section in:\n{out}"));
        let body = &out[start + section.len() + 2..];
        let body = &body[..body.find("\n\n").unwrap_or(body.len())];
        body.lines()
            .find(|l| l.trim_start().starts_with(key))
            .unwrap_or_else(|| panic!("no {key} line in {section}:\n{out}"))
            .to_owned()
    };
    assert!(line("GPU", "memory used").ends_with("unavailable (memory missing)"));
    assert!(line("GPU", "power").ends_with("unavailable (power missing)"));
    assert!(line("BATTERY", "power").ends_with("unavailable (no power)"));
    assert!(line("BATTERY", "time remaining").ends_with("unavailable (no rate)"));
    assert!(line("BATTERY", "health").ends_with("unavailable (no design)"));
}
#[test]
fn ineffective_keys_do_not_redraw() {
    let mut a = app();
    assert_eq!(key(&mut a, K::Char('1')), Effect::None);
    assert_eq!(key(&mut a, K::Char('a')), Effect::Redraw);
    assert_eq!(key(&mut a, K::Char('a')), Effect::None);
    key(&mut a, K::Char('/'));
    assert_eq!(key(&mut a, K::Backspace), Effect::None);
}
#[test]
fn load_per_core() {
    let e = AlertEngine::default().evaluate(&snapshot(), &instant());
    assert_eq!(status(&e, RuleId::Load).value, Some(0.5));
}
#[test]
fn load_zero_cores_unknown() {
    let mut s = snapshot();
    if let Reading::Available(ref mut c) = s.cpu {
        c.per_core.clear();
    }
    let e = AlertEngine::default().evaluate(&s, &instant());
    assert_eq!(status(&e, RuleId::Load).level, Level::Unknown);
    assert_eq!(
        status(&e, RuleId::Load).note.as_deref(),
        Some("no cpu cores")
    );
}
#[test]
fn swap_available_percent() {
    let mut s = snapshot();
    if let Reading::Available(ref mut m) = s.memory {
        m.swap_total_bytes = 100;
        m.swap_used_bytes = 90;
    }
    let e = AlertEngine::default().evaluate(&s, &instant());
    assert_eq!(status(&e, RuleId::Swap).level, Level::Critical);
    assert_eq!(status(&e, RuleId::Swap).value, Some(90.));
}
#[test]
fn storage_temperature() {
    let mut s = snapshot();
    s.sensors = Reading::Available(SensorStats {
        temps: vec![TempSensor {
            chip: "nvme".into(),
            label: "Composite".into(),
            kind: SensorKind::Storage,
            celsius: 75.,
            crit_celsius: Some(90.),
        }],
        fans: vec![],
    });
    let e = AlertEngine::default().evaluate(&s, &instant());
    assert_eq!(status(&e, RuleId::StorageTemp).level, Level::Critical);
    assert_eq!(status(&e, RuleId::StorageTemp).subject, "nvme Composite");
}
#[test]
fn alert_to_unknown_event() {
    let mut e = AlertEngine::default();
    cpu(&mut e, 99., &instant());
    let mut s = snapshot();
    s.cpu = Reading::unavailable("cpu missing");
    let ev = e.evaluate(&s, &instant());
    let event = ev
        .events
        .iter()
        .find(|e| e.rule == RuleId::CpuUsage)
        .unwrap();
    assert_eq!(event.from, Level::Critical);
    assert_eq!(event.to, Level::Unknown);
    assert_eq!(event.value, None);
}
#[test]
fn cli_errors_stay_on_one_line() {
    let error = cli::parse(["--bad\nflag".to_owned()]).unwrap_err();
    assert!(!error.to_string().contains('\n'));
    let error = cli::parse(["--theme=bad\r\nname".to_owned()]).unwrap_err();
    assert!(!error.to_string().contains(['\n', '\r']));
}

fn iface(name: &str, rx: Reading<f64>) -> NetInterface {
    NetInterface {
        name: name.into(),
        rx_bps: rx,
        tx_bps: Reading::Available(1.),
        rx_total_bytes: 10,
        tx_total_bytes: 10,
    }
}
#[test]
fn partial_throughput_is_not_recorded_as_a_total() {
    let mut a = app();
    let mut s = snapshot();
    s.network = Reading::Available(vec![
        iface("eth0", Reading::Available(5.)),
        iface("wlan0", Reading::unavailable("warming up")),
    ]);
    a.apply_snapshot(s.clone());
    assert!(a.history.net_rx_bps.is_empty(), "5 B/s is not the total");
    assert_eq!(a.history.net_tx_bps.last(), Some(2.), "tx is complete");
    s.network = Reading::Available(vec![
        iface("eth0", Reading::Available(5.)),
        iface("wlan0", Reading::Available(0.)),
    ]);
    a.apply_snapshot(s);
    assert_eq!(a.history.net_rx_bps.last(), Some(5.));
}
#[test]
fn slash_opens_the_processes_screen_with_the_filter_editor() {
    let mut a = app();
    assert_eq!(a.screen, Screen::Overview);
    key(&mut a, K::Char('/'));
    assert_eq!(a.screen, Screen::Processes);
    assert_eq!(a.procs.mode, InputMode::Filter);
}
#[test]
fn a_pause_restarts_pending_sustain() {
    let mut a = app();
    let hot = |seq| {
        let mut s = snapshot();
        s.seq = seq;
        if let Reading::Available(c) = &mut s.cpu {
            c.total = 99.;
        }
        s
    };
    a.apply_snapshot(hot(1));
    a.apply_snapshot(hot(2));
    key(&mut a, K::Char(' '));
    key(&mut a, K::Char(' '));
    a.apply_snapshot(hot(3));
    let cpu = a.rules.iter().find(|r| r.rule == RuleId::CpuUsage).unwrap();
    assert_eq!(
        cpu.level,
        Level::Nominal,
        "three samples across a pause are not consecutive"
    );
    a.apply_snapshot(hot(4));
    a.apply_snapshot(hot(5));
    let cpu = a.rules.iter().find(|r| r.rule == RuleId::CpuUsage).unwrap();
    assert_eq!(cpu.level, Level::Critical);
}

#[test]
fn unavailable_process_scan_keeps_selection_until_a_successful_scan() {
    let mut a = app();
    a.apply_snapshot(processes(&[1, 2, 3]));
    key(&mut a, K::Char('G'));
    let selected = a.procs.selected_pid;
    let mut failed = snapshot();
    failed.processes = Reading::unavailable("process scan failed");
    a.apply_snapshot(failed);
    assert_eq!(a.procs.selected_pid, selected);
    a.set_page_size(1);
    key(&mut a, K::Down);
    assert_eq!(a.procs.selected_pid, selected);
    a.apply_snapshot(processes(&[1, 2, 3]));
    assert_eq!(a.procs.selected_pid, selected);
    a.apply_snapshot(processes(&[]));
    assert_eq!(a.procs.selected_pid, None);
}

#[test]
fn name_sort_keeps_pid_ties_ascending_in_both_directions() {
    let mut a = app();
    let mut s = processes(&[3, 2, 1]);
    if let Reading::Available(list) = &mut s.processes {
        for p in list {
            p.name = match p.pid {
                1 => "Alpha",
                2 => "alpha",
                _ => "Beta",
            }
            .into();
        }
    }
    a.apply_snapshot(s);
    a.procs.sort = SortKey::Name;
    a.procs.descending = false;
    assert_eq!(
        a.visible_processes()
            .iter()
            .map(|p| p.pid)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    key(&mut a, K::Char('r'));
    assert_eq!(
        a.visible_processes()
            .iter()
            .map(|p| p.pid)
            .collect::<Vec<_>>(),
        [3, 1, 2]
    );
}
