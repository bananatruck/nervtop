use super::Source;
use crate::model::{CpuStats, Reading};
use std::time::Instant;
use sysinfo::{CpuRefreshKind, MINIMUM_CPU_UPDATE_INTERVAL, System};

pub struct Cpu {
    system: Option<System>,
    stat_path: std::path::PathBuf,
    /// The last accepted counter read and its time.
    baseline: Option<(Instant, Counters)>,
}
impl Default for Cpu {
    fn default() -> Self {
        Self {
            system: System::new().ok(),
            stat_path: "/proc/stat".into(),
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
        self.baseline = Some((now, current));
        s.refresh_cpu_specifics(CpuRefreshKind::nothing().with_frequency());
        let frequencies: Vec<u64> = s
            .cpus()
            .iter()
            .map(|c| c.frequency())
            .filter(|f| *f > 0)
            .collect();
        Reading::Available(CpuStats {
            per_core,
            total,
            load_avg: load_average(),
            freq_mhz: (!frequencies.is_empty())
                .then(|| frequencies.iter().sum::<u64>() / frequencies.len() as u64),
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
