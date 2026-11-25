use super::Source;
use crate::model::{CpuStats, Reading};
use std::time::Instant;
use sysinfo::{CpuRefreshKind, MINIMUM_CPU_UPDATE_INTERVAL, System};

pub struct Cpu {
    system: Option<System>,
    stat_path: std::path::PathBuf,
    /// `/sys/devices/system/cpu`, holding `cpuN/cpufreq/scaling_cur_freq`.
    cpufreq_root: std::path::PathBuf,
    cpuinfo_path: std::path::PathBuf,
    /// The last accepted counter read and its time.
    baseline: Option<(Instant, Counters)>,
}
impl Default for Cpu {
    fn default() -> Self {
        Self {
            system: System::new().ok(),
            stat_path: "/proc/stat".into(),
            cpufreq_root: "/sys/devices/system/cpu".into(),
            cpuinfo_path: "/proc/cpuinfo".into(),
            baseline: None,
        }
    }
}

/// Linux includes guest time in user/nice already. Only the first eight
/// counters contribute to elapsed time, and idle includes iowait.
type Times = [u64; 8];

#[derive(Debug)]
struct Counters {
    total: Times,
    cores: std::collections::BTreeMap<u32, Times>,
}

fn parse_counters(text: &str) -> Option<Counters> {
    fn times(mut fields: std::str::SplitWhitespace<'_>) -> Option<Times> {
        let mut counters = [0; 8];
        for (i, counter) in counters.iter_mut().enumerate() {
            *counter = match fields.next() {
                Some(value) => value.parse().ok()?,
                None if i >= 4 => 0,
                None => return None,
            };
        }
        Some(counters)
    }
    let mut lines = text.lines();
    let mut global = lines.next()?.split_whitespace();
    if global.next()? != "cpu" {
        return None;
    }
    let total = times(global)?;
    let mut cores = std::collections::BTreeMap::new();
    for line in lines {
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next().and_then(|name| name.strip_prefix("cpu")) else {
            break;
        };
        if cores.insert(name.parse().ok()?, times(fields)?).is_some() {
            return None;
        }
    }
    (!cores.is_empty()).then_some(Counters { total, cores })
}

/// Clock frequency in MHz per logical CPU from `/proc/cpuinfo`, from the
/// fields that report one: `cpu MHz` (x86) and `clock` (POWER). BogoMIPS is a
/// delay-loop calibration, not a frequency, and is never read as one.
fn cpuinfo_mhz(text: &str) -> std::collections::BTreeMap<u32, u64> {
    let mut out = std::collections::BTreeMap::new();
    let mut cpu = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "processor" => cpu = value.parse::<u32>().ok(),
            "cpu MHz" | "clock" => {
                let mhz = value.trim_end_matches("MHz").trim().parse::<f64>().ok();
                if let (Some(id), Some(mhz)) = (cpu, mhz.filter(|m| m.is_finite() && *m >= 1.0)) {
                    out.insert(id, mhz as u64);
                }
            }
            _ => {}
        }
    }
    out
}

/// The mean clock frequency of the given logical CPUs, in MHz. Each CPU
/// is read by its own ID (not by its position in a list, which differs
/// when some CPUs are offline): `scaling_cur_freq` in kHz first, then its
/// own `/proc/cpuinfo` block. CPUs with neither are left out; with none at
/// all the frequency is unknown.
fn frequency_mhz(
    cpufreq_root: &std::path::Path,
    cpuinfo_path: &std::path::Path,
    ids: impl Iterator<Item = u32>,
) -> Option<u64> {
    let mut cpuinfo = None;
    let readings: Vec<u64> = ids
        .filter_map(|id| {
            let sysfs = std::fs::read_to_string(
                cpufreq_root.join(format!("cpu{id}/cpufreq/scaling_cur_freq")),
            )
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
            .map(|khz| khz / 1000)
            .filter(|mhz| *mhz > 0);
            sysfs.or_else(|| {
                cpuinfo
                    .get_or_insert_with(|| {
                        cpuinfo_mhz(&std::fs::read_to_string(cpuinfo_path).unwrap_or_default())
                    })
                    .get(&id)
                    .copied()
            })
        })
        .collect();
    (!readings.is_empty()).then(|| readings.iter().sum::<u64>() / readings.len() as u64)
}

fn usage(previous: &Times, current: &Times) -> f32 {
    // iowait may decrease on Linux. Saturation also avoids spikes on resets.
    let delta: Times = std::array::from_fn(|i| current[i].saturating_sub(previous[i]));
    let total: u128 = delta.iter().map(|v| u128::from(*v)).sum();
    let idle = u128::from(delta[3]) + u128::from(delta[4]);
    if total == 0 {
        0.0
    } else {
        ((total - idle) as f64 / total as f64 * 100.0) as f32
    }
}

impl Source for Cpu {
    type Output = CpuStats;
    fn sample(&mut self) -> Reading<CpuStats> {
        self.sample_at(Instant::now())
    }
}

impl Cpu {
    fn sample_at(&mut self, now: Instant) -> Reading<CpuStats> {
        self.sample_with_load(now, || match System::load_average() {
            Ok(load) => Reading::Available([load.one, load.five, load.fifteen]),
            Err(_) => Reading::unavailable("loadavg not readable"),
        })
    }

    fn sample_with_load(
        &mut self,
        now: Instant,
        load_average: impl FnOnce() -> Reading<[f64; 3]>,
    ) -> Reading<CpuStats> {
        let Some(s) = &mut self.system else {
            return Reading::unavailable("cpu initialization failed");
        };
        let Some(current) = std::fs::read_to_string(&self.stat_path)
            .ok()
            .and_then(|text| parse_counters(&text))
        else {
            self.baseline = None;
            return Reading::unavailable("no cpu counters");
        };
        let Some((at, previous)) = &self.baseline else {
            self.baseline = Some((now, current));
            return Reading::unavailable("warming up");
        };
        if !previous.cores.keys().eq(current.cores.keys()) {
            self.baseline = Some((now, current));
            return Reading::unavailable("warming up");
        }
        if now.duration_since(*at) < MINIMUM_CPU_UPDATE_INTERVAL {
            return Reading::unavailable("warming up");
        }
        let total = usage(&previous.total, &current.total);
        let per_core = previous
            .cores
            .values()
            .zip(current.cores.values())
            .map(|(p, c)| usage(p, c))
            .collect();
        let freq_mhz = frequency_mhz(
            &self.cpufreq_root,
            &self.cpuinfo_path,
            current.cores.keys().copied(),
        );
        self.baseline = Some((now, current));
        // The CPU list, and with it the brand, is filled on any refresh.
        s.refresh_cpu_specifics(CpuRefreshKind::nothing());
        Reading::Available(CpuStats {
            per_core,
            total,
            load_avg: load_average(),
            freq_mhz,
            brand: s
                .cpus()
                .first()
                .map(|c| c.brand().trim())
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const MIN: Duration = Duration::from_millis(200);

    #[test]
    fn failed_usage_read_after_warmup_restarts_the_baseline() {
        let root = tempfile::tempdir().unwrap();
        let stat_path = root.path().join("stat");
        let counters = "cpu 100 0 0 100 0 0 0 0 0 0\ncpu0 100 0 0 100 0 0 0 0 0 0\n";
        std::fs::write(&stat_path, counters).unwrap();
        let mut cpu = Cpu {
            stat_path: stat_path.clone(),
            ..Cpu::default()
        };
        let start = Instant::now();
        assert_eq!(cpu.sample_at(start).reason(), Some("warming up"));
        assert!(cpu.sample_at(start + MIN).is_available());
        std::fs::remove_file(&stat_path).unwrap();
        assert_eq!(
            cpu.sample_at(start + MIN * 2).reason(),
            Some("no cpu counters")
        );
        std::fs::write(&stat_path, counters).unwrap();
        assert_eq!(cpu.sample_at(start + MIN * 3).reason(), Some("warming up"));
        assert!(cpu.sample_at(start + MIN * 4).is_available());
    }

    #[test]
    fn usage_comes_from_current_counters_and_respects_the_minimum_interval() {
        let root = tempfile::tempdir().unwrap();
        let stat_path = root.path().join("stat");
        let mut cpu = Cpu {
            stat_path: stat_path.clone(),
            ..Cpu::default()
        };
        let start = Instant::now();
        std::fs::write(
            &stat_path,
            "cpu 100 0 0 100 0 0 0 0\ncpu0 100 0 0 100 0 0 0 0\n",
        )
        .unwrap();
        assert_eq!(cpu.sample_at(start).reason(), Some("warming up"));
        std::fs::write(
            &stat_path,
            "cpu 150 0 0 150 0 0 0 0\ncpu0 150 0 0 150 0 0 0 0\n",
        )
        .unwrap();
        assert_eq!(cpu.sample_at(start + MIN / 4).reason(), Some("warming up"));
        let reading =
            cpu.sample_with_load(start + MIN, || Reading::unavailable("loadavg not readable"));
        let stats = reading.value().unwrap();
        assert_eq!(stats.total, 50.0);
        assert_eq!(stats.per_core, [50.0]);
        assert_eq!(stats.load_avg.reason(), Some("loadavg not readable"));
        assert_eq!(
            cpu.sample_at(start + MIN + MIN / 4).reason(),
            Some("warming up")
        );
        assert!(cpu.sample_at(start + MIN * 2).is_available());
    }

    #[test]
    fn malformed_counters_restart_warmup_and_topology_changes_need_a_baseline() {
        let root = tempfile::tempdir().unwrap();
        let stat_path = root.path().join("stat");
        let mut cpu = Cpu {
            stat_path: stat_path.clone(),
            ..Cpu::default()
        };
        let start = Instant::now();
        for text in [
            "",
            "cpu 1 2\ncpu0 1 2\n",
            "cpu 1 2 3 4\ncpu0 bad 2 3 4\n",
            "cpu 1 2 3 4\n",
        ] {
            std::fs::write(&stat_path, text).unwrap();
            assert_eq!(cpu.sample_at(start).reason(), Some("no cpu counters"));
        }
        std::fs::write(&stat_path, "cpu 1 2 3 4\ncpu0 1 2 3 4\n").unwrap();
        assert_eq!(cpu.sample_at(start).reason(), Some("warming up"));
        assert!(cpu.sample_at(start + MIN).is_available());
        std::fs::write(&stat_path, "cpu 1 2 3 4\ncpu1 1 2 3 4\n").unwrap();
        assert_eq!(cpu.sample_at(start + MIN * 2).reason(), Some("warming up"));
        assert!(cpu.sample_at(start + MIN * 3).is_available());
    }

    /// A Cpu reading counters, cpufreq and cpuinfo from a temporary tree.
    fn fixture(stat: &str, cpufreq: &[(u32, &str)], cpuinfo: &str) -> (tempfile::TempDir, Cpu) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("stat"), stat).unwrap();
        std::fs::write(root.path().join("cpuinfo"), cpuinfo).unwrap();
        for (id, khz) in cpufreq {
            let dir = root.path().join(format!("cpu/cpu{id}/cpufreq"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("scaling_cur_freq"), khz).unwrap();
        }
        let cpu = Cpu {
            stat_path: root.path().join("stat"),
            cpufreq_root: root.path().join("cpu"),
            cpuinfo_path: root.path().join("cpuinfo"),
            ..Cpu::default()
        };
        (root, cpu)
    }

    fn frequency(cpu: &mut Cpu) -> Option<u64> {
        let start = Instant::now();
        assert_eq!(cpu.sample_at(start).reason(), Some("warming up"));
        cpu.sample_at(start + MIN).value().unwrap().freq_mhz
    }

    const CPU0_CPU2: &str = "cpu 1 0 0 1 0 0 0 0\ncpu0 1 0 0 1 0 0 0 0\ncpu2 1 0 0 1 0 0 0 0\n";

    #[test]
    fn frequency_is_read_by_logical_cpu_id() {
        // cpu1 is offline: it has no counters, so its stale cpufreq value is
        // not read, and cpu2's is.
        let (_root, mut cpu) = fixture(
            CPU0_CPU2,
            &[(0, "2000000\n"), (1, "9000000\n"), (2, "3000000\n")],
            "",
        );
        assert_eq!(frequency(&mut cpu), Some(2500));
    }

    #[test]
    fn cpuinfo_fills_in_per_cpu_and_bogomips_is_not_a_frequency() {
        let cpuinfo = "processor\t: 0\ncpu MHz\t\t: 1000.000\n\nprocessor\t: 1\ncpu MHz\t\t: 9000.000\n\nprocessor\t: 2\ncpu MHz\t\t: 3000.000\n";
        let (_root, mut cpu) = fixture(CPU0_CPU2, &[(0, "2000000\n")], cpuinfo);
        assert_eq!(
            frequency(&mut cpu),
            Some(2500),
            "cpu0 from sysfs, cpu2 from cpuinfo"
        );

        let arm = "processor\t: 0\nBogoMIPS\t: 38.40\n\nprocessor\t: 2\nBogoMIPS\t: 38.40\n";
        let (_root, mut cpu) = fixture(CPU0_CPU2, &[], arm);
        assert_eq!(frequency(&mut cpu), None);

        let power = "processor\t: 0\nclock\t\t: 3000.000000MHz\n\nprocessor\t: 2\nclock\t\t: 2000.000000MHz\n";
        let (_root, mut cpu) = fixture(CPU0_CPU2, &[], power);
        assert_eq!(frequency(&mut cpu), Some(2500));
    }

    #[test]
    fn guest_time_is_not_counted_twice_and_idle_includes_iowait() {
        let counters =
            parse_counters("cpu 10 20 30 40 50 60 70 80 9 8\ncpu0 10 20 30 40 50 60 70 80 9 8\n")
                .unwrap();
        assert_eq!(counters.total, [10, 20, 30, 40, 50, 60, 70, 80]);
        assert_eq!(usage(&[0; 8], &counters.total), 75.0);
        assert_eq!(usage(&counters.total, &counters.total), 0.0);
        assert_eq!(usage(&counters.total, &[0; 8]), 0.0);
    }
}
