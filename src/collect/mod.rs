//! Metric sources and the collector that turns them into one `Snapshot`.
//!
//! Every source implements `Source` so tests can replace it with a fake.
//! Real sources read `sysinfo`, `/proc` and `/sys`; see docs/SPEC.md,
//! "Data sources and fallbacks", for what each one reads and when it reports
//! `Reading::Unavailable`.

use crate::model::{
    BatteryStats, CpuStats, DiskStats, GpuStats, HostInfo, MemoryStats, NetInterface, ProcessInfo,
    Reading, SensorStats, Snapshot,
};
use std::time::{SystemTime, UNIX_EPOCH};

/// One metric source. `sample` is called once per tick from the collector thread.
pub trait Source: Send {
    type Output;
    fn sample(&mut self) -> Reading<Self::Output>;
}

/// Host identity and uptime. Each field carries its own availability.
pub trait HostSource: Send {
    fn sample(&mut self) -> HostInfo;
}

/// Every source the collector polls.
pub struct Sources {
    pub host: Box<dyn HostSource>,
    pub cpu: Box<dyn Source<Output = CpuStats>>,
    pub memory: Box<dyn Source<Output = MemoryStats>>,
    pub disks: Box<dyn Source<Output = Vec<DiskStats>>>,
    pub network: Box<dyn Source<Output = Vec<NetInterface>>>,
    pub processes: Box<dyn Source<Output = Vec<ProcessInfo>>>,
    pub sensors: Box<dyn Source<Output = SensorStats>>,
    pub gpus: Box<dyn Source<Output = Vec<GpuStats>>>,
    pub battery: Box<dyn Source<Output = BatteryStats>>,
}

/// Polls every source and numbers the snapshots.
pub struct Collector {
    sources: Sources,
    seq: u64,
}

impl Collector {
    pub fn new(sources: Sources) -> Collector {
        Collector { sources, seq: 0 }
    }

    /// Takes one sample from every source.
    pub fn sample(&mut self) -> Snapshot {
        self.seq += 1;
        let taken_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let s = &mut self.sources;
        let mut snapshot = Snapshot {
            taken_at_ms,
            seq: self.seq,
            host: s.host.sample(),
            cpu: s.cpu.sample(),
            memory: s.memory.sample(),
            disks: s.disks.sample(),
            network: s.network.sample(),
            processes: s.processes.sample(),
            sensors: s.sensors.sample(),
            gpus: s.gpus.sample(),
            battery: s.battery.sample(),
        };
        sanitize(&mut snapshot);
        snapshot
    }
}

/// Replaces control characters in one string with `?`.
fn clean(text: &mut String) {
    if text.chars().any(char::is_control) {
        *text = text
            .chars()
            .map(|c| if c.is_control() { '?' } else { c })
            .collect();
    }
}

/// Removes control characters from every name in a snapshot. Process names
/// in particular are set by any unprivileged program, and an escape sequence
/// or newline in one must not reach a terminal through `--once` or forge a
/// row in a report.
fn sanitize(s: &mut Snapshot) {
    fn each<T>(r: &mut Reading<T>, f: impl FnOnce(&mut T)) {
        if let Reading::Available(v) = r {
            f(v);
        }
    }
    each(&mut s.host.hostname, clean);
    each(&mut s.host.kernel, clean);
    each(&mut s.cpu, |c| {
        if let Some(b) = &mut c.brand {
            clean(b);
        }
    });
    each(&mut s.disks, |ds| {
        for d in ds {
            clean(&mut d.mount_point);
            clean(&mut d.device);
            clean(&mut d.file_system);
        }
    });
    each(&mut s.network, |ns| {
        ns.iter_mut().for_each(|n| clean(&mut n.name))
    });
    each(&mut s.processes, |ps| {
        for p in ps {
            clean(&mut p.name);
            clean(&mut p.user);
            clean(&mut p.state);
        }
    });
    each(&mut s.sensors, |ss| {
        for t in &mut ss.temps {
            clean(&mut t.chip);
            clean(&mut t.label);
        }
        for f in &mut ss.fans {
            clean(&mut f.chip);
            clean(&mut f.label);
        }
    });
    each(&mut s.gpus, |gs| {
        for g in gs {
            clean(&mut g.name);
            clean(&mut g.pci_slot);
        }
    });
    each(&mut s.battery, |b| clean(&mut b.name));
}

pub mod battery;
pub mod cpu;
pub mod disk;
pub mod fake;
pub mod gpu;
pub mod host;
pub mod memory;
pub mod network;
pub mod process;
pub mod sensors;

impl Collector {
    /// Connects all sources to the running Linux system.
    pub fn system() -> Collector {
        Collector::new(Sources {
            host: Box::new(host::Host),
            cpu: Box::new(cpu::Cpu::default()),
            memory: Box::new(memory::Memory::default()),
            disks: Box::new(disk::Disks::new("/".into())),
            network: Box::new(network::Network::default()),
            processes: Box::new(process::Processes::default()),
            sensors: Box::new(sensors::Sensors::new("/".into())),
            gpus: Box::new(gpu::Gpus::new("/".into())),
            battery: Box::new(battery::Battery::new("/".into())),
        })
    }
}

use std::path::{Path, PathBuf};
fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}
fn number<T: std::str::FromStr>(path: impl AsRef<Path>) -> Option<T> {
    read(path)?.parse().ok()
}
fn entries(path: impl AsRef<Path>) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    paths.sort();
    paths
}

/// Counter delta per second. Backwards counters are resets, never huge spikes.
pub fn rate(previous: Option<u64>, current: u64, elapsed_secs: f64) -> Reading<f64> {
    let Some(previous) = previous else {
        return Reading::unavailable("warming up");
    };
    let Some(delta) = current.checked_sub(previous) else {
        return Reading::unavailable("counter reset");
    };
    if !elapsed_secs.is_finite() || elapsed_secs <= 0.0 {
        return Reading::unavailable("zero elapsed time");
    }
    Reading::Available(delta as f64 / elapsed_secs)
}

#[derive(Default)]
struct Rates {
    previous: std::collections::BTreeMap<String, (u64, u64)>,
    at: Option<std::time::Instant>,
}
impl Rates {
    fn sample(
        &mut self,
        counters: std::collections::BTreeMap<String, (u64, u64)>,
    ) -> std::collections::BTreeMap<String, (Reading<f64>, Reading<f64>)> {
        let now = std::time::Instant::now();
        let elapsed = self
            .at
            .map(|at| now.duration_since(at).as_secs_f64())
            .unwrap_or(0.0);
        let out = counters
            .iter()
            .map(|(name, (a, b))| {
                let prev = self.previous.get(name);
                (
                    name.clone(),
                    (
                        rate(prev.map(|p| p.0), *a, elapsed),
                        rate(prev.map(|p| p.1), *b, elapsed),
                    ),
                )
            })
            .collect();
        self.previous = counters;
        self.at = Some(now);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_characters_never_reach_any_output() {
        let mut snap = fake::sample_snapshot();
        if let Reading::Available(ps) = &mut snap.processes {
            ps[0].name = "\u{1b}[2Jowned".into();
        }
        if let Reading::Available(s) = &mut snap.sensors {
            s.temps[0].label = "fake\nCPU TEMP 20.0".into();
        }
        snap.host.hostname = Reading::Available("host\u{7}".into());
        let mut c = Collector::new(fake::sources_with(snap));
        let out = c.sample();
        assert_eq!(out.processes.value().unwrap()[0].name, "?[2Jowned");
        assert_eq!(
            out.sensors.value().unwrap().temps[0].label,
            "fake?CPU TEMP 20.0"
        );
        assert_eq!(out.host.hostname, Reading::Available("host?".into()));
        let json = serde_json::to_string(&out).unwrap();
        assert!(!json.contains("\\u001b") && !json.contains("\\n"), "{json}");
    }
    #[test]
    fn counter_rates() {
        assert_eq!(rate(Some(10), 30, 2.0), Reading::Available(10.0));
        assert_eq!(rate(Some(10), 10, 1.0), Reading::Available(0.0));
        assert_eq!(rate(None, 10, 1.0).reason(), Some("warming up"));
        for elapsed in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(!rate(Some(0), 10, elapsed).is_available());
        }
        assert_eq!(rate(Some(u64::MAX), 0, 1.0).reason(), Some("counter reset"));
    }
    #[test]
    #[ignore = "reads the live machine and waits for a second sample"]
    fn live_system() {
        let mut collector = Collector::system();
        let first = collector.sample();
        assert_eq!(first.cpu.reason(), Some("warming up"));
        std::thread::sleep(std::time::Duration::from_millis(500));
        let second = collector.sample();
        println!("{}", serde_json::to_string_pretty(&second).unwrap());
        assert_eq!(second.seq, 2);
        assert!(second.cpu.is_available());
        assert!(second.memory.is_available());
        assert!(second.disks.is_available());
        assert!(second.network.is_available());
        assert!(second.processes.is_available());
        assert!(
            second.sensors.is_available() || second.sensors.reason() == Some("no hwmon sensors")
        );
        assert!(
            second.gpus.is_available() || second.gpus.reason() == Some("no display adapters found")
        );
        assert!(
            second.battery.is_available() || second.battery.reason() == Some("no battery present")
        );
    }
}
