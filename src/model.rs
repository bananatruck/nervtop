//! Plain data produced by the collectors and consumed by alerts and the UI.
//!
//! Nothing in this module reads the system. Every value here was measured by a
//! source in `collect`, and anything that could not be measured is carried as
//! `Reading::Unavailable` with a human readable reason.

use serde::Serialize;

/// A measured value, or the reason it could not be measured.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum Reading<T> {
    Available(T),
    Unavailable(String),
}

impl<T> Reading<T> {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Reading::Unavailable(reason.into())
    }

    pub fn as_ref(&self) -> Reading<&T> {
        match self {
            Reading::Available(v) => Reading::Available(v),
            Reading::Unavailable(r) => Reading::Unavailable(r.clone()),
        }
    }

    pub fn value(&self) -> Option<&T> {
        match self {
            Reading::Available(v) => Some(v),
            Reading::Unavailable(_) => None,
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Reading::Available(_) => None,
            Reading::Unavailable(r) => Some(r),
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Reading<U> {
        match self {
            Reading::Available(v) => Reading::Available(f(v)),
            Reading::Unavailable(r) => Reading::Unavailable(r),
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self, Reading::Available(_))
    }
}

impl<T> From<Option<T>> for Reading<T> {
    /// Converts `None` into a generic "not reported" reason. Prefer
    /// `Reading::unavailable` with a specific reason where one is known.
    fn from(value: Option<T>) -> Self {
        match value {
            Some(v) => Reading::Available(v),
            None => Reading::unavailable("not reported"),
        }
    }
}

/// One full sample of the machine.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot {
    /// Milliseconds since the Unix epoch when sampling started.
    pub taken_at_ms: u64,
    /// Sequence number, starting at 1 for the first sample.
    pub seq: u64,
    pub host: HostInfo,
    pub cpu: Reading<CpuStats>,
    pub memory: Reading<MemoryStats>,
    pub disks: Reading<Vec<DiskStats>>,
    pub network: Reading<Vec<NetInterface>>,
    pub processes: Reading<Vec<ProcessInfo>>,
    pub sensors: Reading<SensorStats>,
    pub gpus: Reading<Vec<GpuStats>>,
    pub battery: Reading<BatteryStats>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HostInfo {
    pub hostname: Reading<String>,
    pub kernel: Reading<String>,
    pub uptime_secs: Reading<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CpuStats {
    /// Usage of each logical core in percent, 0 to 100, in core order.
    pub per_core: Vec<f32>,
    /// Usage of all cores together in percent, 0 to 100.
    pub total: f32,
    /// One, five and fifteen minute load averages, read separately from usage
    /// so a failed `/proc/loadavg` read does not hide CPU usage.
    pub load_avg: Reading<[f64; 3]>,
    /// Average current frequency across cores in MHz, when the kernel reports it.
    pub freq_mhz: Option<u64>,
    /// CPU model name, for example "AMD Ryzen 9 8945HS".
    pub brand: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemoryStats {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

impl MemoryStats {
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.available_bytes)
    }

    pub fn used_percent(&self) -> Option<f64> {
        percent(self.used_bytes(), self.total_bytes)
    }

    pub fn swap_used_percent(&self) -> Option<f64> {
        percent(self.swap_used_bytes, self.swap_total_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiskStats {
    pub mount_point: String,
    /// Block device name without `/dev/`, for example `nvme0n1p8`.
    pub device: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    /// Read rate of the backing block device in bytes per second.
    pub read_bps: Reading<f64>,
    /// Write rate of the backing block device in bytes per second.
    pub write_bps: Reading<f64>,
}

impl DiskStats {
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.available_bytes)
    }

    pub fn used_percent(&self) -> Option<f64> {
        percent(self.used_bytes(), self.total_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NetInterface {
    pub name: String,
    pub rx_bps: Reading<f64>,
    pub tx_bps: Reading<f64>,
    /// Current counters, independent of the availability of rate baselines.
    pub rx_total_bytes: Reading<u64>,
    pub tx_total_bytes: Reading<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    /// User name, or the numeric UID when it cannot be resolved.
    pub user: String,
    /// CPU usage in percent of one core, so it can exceed 100 on multicore machines.
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    /// Short state, for example "run", "sleep", "idle", "zombie".
    pub state: String,
    /// True for kernel threads, which have no command line.
    pub kernel_thread: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SensorStats {
    pub temps: Vec<TempSensor>,
    pub fans: Vec<Fan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    /// CPU package or die sensor (k10temp, coretemp, zenpower).
    Cpu,
    /// GPU sensor (amdgpu, nouveau).
    Gpu,
    /// NVMe or drive sensor.
    Storage,
    /// Anything else (ACPI zones, wifi chips, battery).
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TempSensor {
    /// hwmon chip name, for example `k10temp`.
    pub chip: String,
    /// Sensor label, for example `Tctl`, or `temp1` when unlabeled.
    pub label: String,
    pub kind: SensorKind,
    pub celsius: f64,
    /// Critical temperature reported by the chip, if any.
    pub crit_celsius: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fan {
    pub chip: String,
    pub label: String,
    pub rpm: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuVendor {
    Amd,
    Nvidia,
    Intel,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuPower {
    /// The device is powered and its fields were queried.
    Active,
    /// The device is runtime-suspended; fields were not queried so it stays asleep.
    Suspended,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GpuStats {
    /// PCI address, for example `0000:01:00.0`.
    pub pci_slot: String,
    pub vendor: GpuVendor,
    pub name: String,
    pub power_state: GpuPower,
    pub utilization_percent: Reading<f32>,
    pub memory_used_bytes: Reading<u64>,
    pub memory_total_bytes: Reading<u64>,
    pub temperature_celsius: Reading<f64>,
    pub power_watts: Reading<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BatteryStats {
    /// Kernel name, for example `BAT1`.
    pub name: String,
    pub percent: f64,
    pub status: BatteryStatus,
    /// Whether external power is connected, when a mains supply is exposed.
    pub ac_online: Option<bool>,
    /// Charge or discharge power in watts, always positive.
    pub power_watts: Reading<f64>,
    /// Seconds until empty (discharging) or full (charging).
    pub time_remaining_secs: Reading<u64>,
    /// Full capacity as a percent of design capacity.
    pub health_percent: Reading<f64>,
    /// Charge limit set by the vendor driver, for example 80 on ASUS laptops.
    pub charge_limit_percent: Option<u8>,
    pub cycle_count: Option<u64>,
}

/// `part / whole * 100`, or `None` when `whole` is zero.
pub fn percent(part: u64, whole: u64) -> Option<f64> {
    if whole == 0 {
        None
    } else {
        Some(part as f64 / whole as f64 * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_handles_zero_whole() {
        assert_eq!(percent(5, 0), None);
        assert_eq!(percent(1, 4), Some(25.0));
    }

    #[test]
    fn memory_used_is_total_minus_available() {
        let m = MemoryStats {
            total_bytes: 100,
            available_bytes: 30,
            swap_total_bytes: 0,
            swap_used_bytes: 0,
        };
        assert_eq!(m.used_bytes(), 70);
        assert_eq!(m.used_percent(), Some(70.0));
        assert_eq!(m.swap_used_percent(), None);
    }

    #[test]
    fn reading_serializes_with_status_tag() {
        let a: Reading<u8> = Reading::Available(3);
        let u: Reading<u8> = Reading::unavailable("no sensor");
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            r#"{"status":"available","value":3}"#
        );
        assert_eq!(
            serde_json::to_string(&u).unwrap(),
            r#"{"status":"unavailable","value":"no sensor"}"#
        );
    }
}
