//! Deterministic sources and entirely made-up test data. Never used by `Collector::system`.
use super::{HostSource, Source, Sources};
use crate::model::*;
use std::collections::VecDeque;

/// Returns queued readings in order, then repeats the last reading.
/// An empty queue reports `no fake readings`.
pub struct FakeSource<T: Clone> {
    queued: VecDeque<Reading<T>>,
    last: Reading<T>,
}
impl<T: Clone> FakeSource<T> {
    pub fn new(readings: impl IntoIterator<Item = Reading<T>>) -> Self {
        Self {
            queued: readings.into_iter().collect(),
            last: Reading::unavailable("no fake readings"),
        }
    }
    pub fn constant(reading: Reading<T>) -> Self {
        Self::new([reading])
    }
}
impl<T: Clone + Send> Source for FakeSource<T> {
    type Output = T;
    fn sample(&mut self) -> Reading<T> {
        if let Some(next) = self.queued.pop_front() {
            self.last = next;
        }
        self.last.clone()
    }
}
pub struct FakeHost(pub HostInfo);
impl HostSource for FakeHost {
    fn sample(&mut self) -> HostInfo {
        self.0.clone()
    }
}
/// Builds independent constant fakes from a caller's snapshot.
pub fn sources_with(snapshot: Snapshot) -> Sources {
    Sources {
        host: Box::new(FakeHost(snapshot.host)),
        cpu: Box::new(FakeSource::constant(snapshot.cpu)),
        memory: Box::new(FakeSource::constant(snapshot.memory)),
        disks: Box::new(FakeSource::constant(snapshot.disks)),
        network: Box::new(FakeSource::constant(snapshot.network)),
        processes: Box::new(FakeSource::constant(snapshot.processes)),
        sensors: Box::new(FakeSource::constant(snapshot.sensors)),
        gpus: Box::new(FakeSource::constant(snapshot.gpus)),
        battery: Box::new(FakeSource::constant(snapshot.battery)),
    }
}
/// A realistic, fully available laptop sample containing made-up test data.
pub fn sample_snapshot() -> Snapshot {
    use Reading::Available as A;
    Snapshot {
        taken_at_ms: 1_700_000_000_000,
        seq: 1,
        host: HostInfo {
            hostname: A("test-laptop".into()),
            kernel: A("6.12.0-test".into()),
            uptime_secs: A(12345),
        },
        cpu: A(CpuStats {
            per_core: vec![24.0; 16],
            total: 24.0,
            load_avg: [1.2, 1.4, 1.1],
            freq_mhz: Some(3200),
            brand: Some("Test 16-core CPU".into()),
        }),
        memory: A(MemoryStats {
            total_bytes: 16 << 30,
            available_bytes: 9 << 30,
            swap_total_bytes: 4 << 30,
            swap_used_bytes: 256 << 20,
        }),
        disks: A([
            ("/", "nvme0n1p2", 500_u64 << 30, 200_u64 << 30),
            ("/boot", "nvme0n1p1", 1 << 30, 512 << 20),
        ]
        .into_iter()
        .map(|(mount, device, total, available)| DiskStats {
            mount_point: mount.into(),
            device: device.into(),
            file_system: "ext4".into(),
            total_bytes: total,
            available_bytes: available,
            read_bps: A(2048.0),
            write_bps: A(4096.0),
        })
        .collect()),
        network: A(["wlan0", "enp3s0"]
            .into_iter()
            .map(|name| NetInterface {
                name: name.into(),
                rx_bps: A(125000.0),
                tx_bps: A(15000.0),
                rx_total_bytes: 123456789,
                tx_total_bytes: 23456789,
            })
            .collect()),
        processes: A([
            (1, "init", 0.1, 4_u64 << 20),
            (1000, "test-browser", 32.0, 800 << 20),
            (1001, "test-editor", 5.0, 250 << 20),
        ]
        .into_iter()
        .map(|(pid, name, cpu, memory)| ProcessInfo {
            pid,
            name: name.into(),
            user: "testuser".into(),
            cpu_percent: cpu,
            memory_bytes: memory,
            state: "sleep".into(),
            kernel_thread: false,
        })
        .collect()),
        sensors: A(SensorStats {
            temps: vec![
                TempSensor {
                    chip: "k10temp".into(),
                    label: "Tctl".into(),
                    kind: SensorKind::Cpu,
                    celsius: 62.5,
                    crit_celsius: Some(95.0),
                },
                TempSensor {
                    chip: "nvme".into(),
                    label: "Composite".into(),
                    kind: SensorKind::Storage,
                    celsius: 43.0,
                    crit_celsius: Some(85.0),
                },
            ],
            fans: vec![Fan {
                chip: "asus".into(),
                label: "cpu_fan".into(),
                rpm: 2200,
            }],
        }),
        gpus: A([
            (GpuVendor::Amd, "0000:05:00.0", "Test AMD GPU"),
            (GpuVendor::Nvidia, "0000:01:00.0", "Test NVIDIA GPU"),
        ]
        .into_iter()
        .map(|(vendor, slot, name)| GpuStats {
            pci_slot: slot.into(),
            vendor,
            name: name.into(),
            power_state: GpuPower::Active,
            utilization_percent: A(12.0),
            memory_used_bytes: A(512 << 20),
            memory_total_bytes: A(8 << 30),
            temperature_celsius: A(48.0),
            power_watts: A(8.5),
        })
        .collect()),
        battery: A(BatteryStats {
            name: "BAT1".into(),
            percent: 75.0,
            status: BatteryStatus::Discharging,
            ac_online: Some(false),
            power_watts: A(10.0),
            time_remaining_secs: A(10800),
            health_percent: A(90.0),
            charge_limit_percent: Some(80),
            cycle_count: Some(120),
        }),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_source_repeats_last_and_handles_empty() {
        let mut f = FakeSource::new([Reading::Available(1), Reading::Available(2)]);
        assert_eq!(f.sample(), Reading::Available(1));
        assert_eq!(f.sample(), Reading::Available(2));
        assert_eq!(f.sample(), Reading::Available(2));
        assert_eq!(
            FakeSource::<u8>::new([]).sample().reason(),
            Some("no fake readings")
        );
    }
    #[test]
    fn collector_numbers_fake_snapshots() {
        let snapshot = sample_snapshot();
        let mut c = super::super::Collector::new(sources_with(snapshot.clone()));
        let first = c.sample();
        let second = c.sample();
        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);
        assert!(first.taken_at_ms > 0);
        assert_eq!(second.cpu, snapshot.cpu);
        assert_eq!(second.gpus, snapshot.gpus);
        assert_eq!(second.battery, snapshot.battery);
    }
}
