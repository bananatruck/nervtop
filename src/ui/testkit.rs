//! Test helpers that build realistic `App` values by hand and inspect rendered `TestBackend` buffers.

use crate::alert::{Agreement, AlertEvent, CellId, CellVote, Consensus, Level, RuleId, RuleStatus};
use crate::app::App;
use crate::config::Config;
use crate::model::{
    BatteryStats, BatteryStatus, CpuStats, DiskStats, Fan, GpuPower, GpuStats, GpuVendor, HostInfo,
    MemoryStats, NetInterface, ProcessInfo, Reading, SensorKind, SensorStats, Snapshot, TempSensor,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

/// Converts a ratatui `Buffer` into a `Vec<String>`, one entry per terminal row.
pub fn buffer_lines(buf: &Buffer) -> Vec<String> {
    let mut lines = Vec::with_capacity(buf.area.height as usize);
    for y in 0..buf.area.height {
        let mut line = String::with_capacity(buf.area.width as usize);
        for x in 0..buf.area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        lines.push(line);
    }
    lines
}

/// Joins all rows of `buf` with newlines.
pub fn buffer_text(buf: &Buffer) -> String {
    buffer_lines(buf).join("\n")
}

/// Renders `app` into a `TestBackend` of size `width` x `height` and returns the resulting buffer.
pub fn render(app: &App, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("TestBackend terminal creation succeeds");
    terminal
        .draw(|frame| super::draw(frame, app))
        .expect("TestBackend draw succeeds");
    terminal.backend().buffer().clone()
}

/// Builds a realistic nominal `App` with 16 cores, 2 GPUs (one active, one suspended),
/// sensors, fans, disks, network interfaces, battery, processes, history and all 9 alert rules.
pub fn nominal_app() -> App {
    let config = Config::default();
    let palette = config.palette().expect("default palette is valid");
    let mut app = App::new(config, palette);

    let per_core = vec![
        18.0, 24.5, 62.0, 12.0, 31.0, 44.0, 88.0, 15.0, 19.0, 22.0, 35.0, 41.0, 10.0, 9.5, 28.0,
        33.0,
    ];
    let snapshot = Snapshot {
        taken_at_ms: 1_763_452_319_000,
        seq: 42,
        host: HostInfo {
            hostname: Reading::Available("magi-01".into()),
            kernel: Reading::Available("6.12.4-arch1".into()),
            uptime_secs: Reading::Available(14_520),
        },
        cpu: Reading::Available(CpuStats {
            per_core,
            total: 30.8,
            load_avg: Reading::Available([1.42, 1.18, 0.95]),
            freq_mhz: Some(3850),
            brand: Some("AMD Ryzen 9 8945HS".into()),
        }),
        memory: Reading::Available(MemoryStats {
            total_bytes: 16 * 1024 * 1024 * 1024,
            available_bytes: 9 * 1024 * 1024 * 1024,
            swap_total_bytes: 4 * 1024 * 1024 * 1024,
            swap_used_bytes: 512 * 1024 * 1024,
        }),
        disks: Reading::Available(vec![
            DiskStats {
                mount_point: "/".into(),
                device: "nvme0n1p2".into(),
                file_system: "ext4".into(),
                total_bytes: 500 * 1024 * 1024 * 1024,
                available_bytes: 220 * 1024 * 1024 * 1024,
                read_bps: Reading::Available(1_572_864.0),
                write_bps: Reading::Available(524_288.0),
            },
            DiskStats {
                mount_point: "/home".into(),
                device: "nvme0n1p3".into(),
                file_system: "ext4".into(),
                total_bytes: 1024 * 1024 * 1024 * 1024,
                available_bytes: 600 * 1024 * 1024 * 1024,
                read_bps: Reading::Available(204_800.0),
                write_bps: Reading::Available(1_048_576.0),
            },
        ]),
        network: Reading::Available(vec![
            NetInterface {
                name: "wlan0".into(),
                rx_bps: Reading::Available(2_621_440.0),
                tx_bps: Reading::Available(393_216.0),
                rx_total_bytes: 8 * 1024 * 1024 * 1024,
                tx_total_bytes: 1024 * 1024 * 1024,
            },
            NetInterface {
                name: "eth0".into(),
                rx_bps: Reading::Available(0.0),
                tx_bps: Reading::Available(0.0),
                rx_total_bytes: 64 * 1024 * 1024,
                tx_total_bytes: 32 * 1024 * 1024,
            },
        ]),
        processes: Reading::Available(vec![
            ProcessInfo {
                pid: 4242,
                name: "nervtop".into(),
                user: "kesh".into(),
                cpu_percent: 1.8,
                memory_bytes: 24 * 1024 * 1024,
                state: "run".into(),
                kernel_thread: false,
            },
            ProcessInfo {
                pid: 1337,
                name: "rustc".into(),
                user: "kesh".into(),
                cpu_percent: 142.5,
                memory_bytes: 1_800 * 1024 * 1024,
                state: "run".into(),
                kernel_thread: false,
            },
            ProcessInfo {
                pid: 2048,
                name: "firefox".into(),
                user: "kesh".into(),
                cpu_percent: 14.2,
                memory_bytes: 920 * 1024 * 1024,
                state: "sleep".into(),
                kernel_thread: false,
            },
            ProcessInfo {
                pid: 99,
                name: "kworker/u32:1".into(),
                user: "root".into(),
                cpu_percent: 0.4,
                memory_bytes: 0,
                state: "idle".into(),
                kernel_thread: true,
            },
        ]),
        sensors: Reading::Available(SensorStats {
            temps: vec![
                TempSensor {
                    chip: "k10temp".into(),
                    label: "Tctl".into(),
                    kind: SensorKind::Cpu,
                    celsius: 92.0,
                    crit_celsius: Some(100.0),
                },
                TempSensor {
                    chip: "amdgpu".into(),
                    label: "edge".into(),
                    kind: SensorKind::Gpu,
                    celsius: 54.0,
                    crit_celsius: Some(100.0),
                },
                TempSensor {
                    chip: "nvme".into(),
                    label: "Composite".into(),
                    kind: SensorKind::Storage,
                    celsius: 43.0,
                    crit_celsius: Some(82.0),
                },
            ],
            fans: vec![
                Fan {
                    chip: "asus".into(),
                    label: "cpu_fan".into(),
                    rpm: 2800,
                },
                Fan {
                    chip: "asus".into(),
                    label: "gpu_fan".into(),
                    rpm: 2400,
                },
            ],
        }),
        gpus: Reading::Available(vec![
            GpuStats {
                pci_slot: "0000:65:00.0".into(),
                vendor: GpuVendor::Amd,
                name: "Radeon 780M".into(),
                power_state: GpuPower::Active,
                utilization_percent: Reading::Available(34.0),
                memory_used_bytes: Reading::Available(1024 * 1024 * 1024),
                memory_total_bytes: Reading::Available(4 * 1024 * 1024 * 1024),
                temperature_celsius: Reading::Available(54.0),
                power_watts: Reading::Available(18.5),
            },
            GpuStats {
                pci_slot: "0000:01:00.0".into(),
                vendor: GpuVendor::Nvidia,
                name: "GeForce RTX 4060".into(),
                power_state: GpuPower::Suspended,
                utilization_percent: Reading::unavailable("suspended"),
                memory_used_bytes: Reading::unavailable("suspended"),
                memory_total_bytes: Reading::unavailable("suspended"),
                temperature_celsius: Reading::unavailable("suspended"),
                power_watts: Reading::unavailable("suspended"),
            },
        ]),
        battery: Reading::Available(BatteryStats {
            name: "BAT1".into(),
            percent: 78.0,
            status: BatteryStatus::Discharging,
            ac_online: Some(false),
            power_watts: Reading::Available(14.2),
            time_remaining_secs: Reading::Available(11_400),
            health_percent: Reading::Available(94.5),
            charge_limit_percent: Some(80),
            cycle_count: Some(67),
        }),
    };

    app.snapshot = Some(snapshot);
    app.procs.selected_pid = Some(4242);

    for i in 0..40 {
        let f = (i as f64 * 2.0) % 90.0 + 10.0;
        app.history.cpu_total.push(f);
        app.history.memory_percent.push(43.75);
        app.history.net_rx_bps.push(100_000.0 + f * 1000.0);
        app.history.net_tx_bps.push(50_000.0 + f * 500.0);
        app.history.disk_read_bps.push(200_000.0);
        app.history.disk_write_bps.push(100_000.0);
    }

    app.rules = vec![
        RuleStatus {
            rule: RuleId::CpuUsage,
            subject: String::new(),
            value: Some(30.8),
            level: Level::Nominal,
            warn: 85.0,
            crit: 95.0,
            note: None,
        },
        RuleStatus {
            rule: RuleId::Load,
            subject: String::new(),
            value: Some(0.09),
            level: Level::Nominal,
            warn: 1.5,
            crit: 3.0,
            note: None,
        },
        RuleStatus {
            rule: RuleId::CpuTemp,
            subject: "k10temp:Tctl".into(),
            value: Some(68.0),
            level: Level::Nominal,
            warn: 85.0,
            crit: 95.0,
            note: None,
        },
        RuleStatus {
            rule: RuleId::GpuTemp,
            subject: "Radeon 780M".into(),
            value: Some(54.0),
            level: Level::Nominal,
            warn: 80.0,
            crit: 90.0,
            note: None,
        },
        RuleStatus {
            rule: RuleId::Memory,
            subject: String::new(),
            value: Some(43.75),
            level: Level::Nominal,
            warn: 85.0,
            crit: 95.0,
            note: None,
        },
        RuleStatus {
            rule: RuleId::Swap,
            subject: String::new(),
            value: Some(12.5),
            level: Level::Nominal,
            warn: 60.0,
            crit: 90.0,
            note: None,
        },
        RuleStatus {
            rule: RuleId::Disk,
            subject: "/".into(),
            value: Some(56.0),
            level: Level::Nominal,
            warn: 90.0,
            crit: 95.0,
            note: None,
        },
        RuleStatus {
            rule: RuleId::StorageTemp,
            subject: "nvme:Composite".into(),
            value: Some(43.0),
            level: Level::Nominal,
            warn: 65.0,
            crit: 75.0,
            note: None,
        },
        RuleStatus {
            rule: RuleId::Battery,
            subject: "BAT1".into(),
            value: Some(78.0),
            level: Level::Nominal,
            warn: 20.0,
            crit: 10.0,
            note: None,
        },
    ];

    app.consensus = Consensus {
        cells: [
            CellVote {
                cell: CellId::Compute,
                vote: Level::Nominal,
                reason: None,
            },
            CellVote {
                cell: CellId::Thermal,
                vote: Level::Nominal,
                reason: None,
            },
            CellVote {
                cell: CellId::Resource,
                vote: Level::Nominal,
                reason: None,
            },
        ],
        verdict: Level::Nominal,
        agreement: Agreement::Unanimous,
    };

    app
}

/// Builds an `App` with a Critical disk alert (`DISK / 97.0%`).
pub fn critical_disk_app() -> App {
    let mut app = nominal_app();
    if let Some(rule) = app.rules.iter_mut().find(|r| r.rule == RuleId::Disk) {
        rule.value = Some(97.0);
        rule.level = Level::Critical;
    }
    app.consensus = Consensus {
        cells: [
            CellVote {
                cell: CellId::Compute,
                vote: Level::Nominal,
                reason: None,
            },
            CellVote {
                cell: CellId::Thermal,
                vote: Level::Nominal,
                reason: None,
            },
            CellVote {
                cell: CellId::Resource,
                vote: Level::Critical,
                reason: Some("DISK / 97.0%".into()),
            },
        ],
        verdict: Level::Critical,
        agreement: Agreement::Majority,
    };
    app.log.push_back(AlertEvent {
        at_ms: 1_763_452_319_000,
        rule: RuleId::Disk,
        subject: "/".into(),
        from: Level::Caution,
        to: Level::Critical,
        value: Some(97.0),
    });
    app
}
