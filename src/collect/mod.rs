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
        Snapshot {
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
        }
    }
}
