use super::Source;
use crate::model::{CpuStats, Reading};
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MINIMUM_CPU_UPDATE_INTERVAL, System};

pub struct Cpu {
    system: Option<System>,
    /// When the first populated counter read happened; usage is measured
    /// against it.
    baseline: Option<Instant>,
}
impl Default for Cpu {
    fn default() -> Self {
        Self {
            system: System::new().ok(),
            baseline: None,
        }
    }
}

/// Decides whether usage can be reported. Usage needs a populated baseline
/// read at least `min` earlier: sysinfo ignores a refresh that comes sooner
/// than its `MINIMUM_CPU_UPDATE_INTERVAL`, so a quicker second read would
/// report a delta over nothing. A failed read starts warm-up over.
fn warm_up(
    baseline: &mut Option<Instant>,
    now: Instant,
    populated: bool,
    min: Duration,
) -> Result<(), &'static str> {
    if !populated {
        *baseline = None;
        return Err("no cpu counters");
    }
    match baseline {
        None => {
            *baseline = Some(now);
            Err("warming up")
        }
        Some(t) if now.duration_since(*t) < min => Err("warming up"),
        Some(_) => Ok(()),
    }
}

impl Source for Cpu {
    type Output = CpuStats;
    fn sample(&mut self) -> Reading<CpuStats> {
        let Some(s) = &mut self.system else {
            return Reading::unavailable("cpu initialization failed");
        };
        s.refresh_cpu_specifics(CpuRefreshKind::everything());
        if let Err(reason) = warm_up(
            &mut self.baseline,
            Instant::now(),
            !s.cpus().is_empty(),
            MINIMUM_CPU_UPDATE_INTERVAL,
        ) {
            return Reading::unavailable(reason);
        }
        let frequencies: Vec<u64> = s
            .cpus()
            .iter()
            .map(|c| c.frequency())
            .filter(|f| *f > 0)
            .collect();
        let load_avg = match System::load_average() {
            Ok(load) => Reading::Available([load.one, load.five, load.fifteen]),
            Err(_) => Reading::unavailable("loadavg not readable"),
        };
        Reading::Available(CpuStats {
            per_core: s.cpus().iter().map(|c| c.usage()).collect(),
            total: s.global_cpu_usage(),
            load_avg,
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
    fn usage_needs_a_populated_baseline_old_enough() {
        let t0 = Instant::now();
        let mut baseline = None;
        assert_eq!(warm_up(&mut baseline, t0, true, MIN), Err("warming up"));
        // A second read inside sysinfo's minimum interval is not a new reading.
        assert_eq!(
            warm_up(&mut baseline, t0 + Duration::from_millis(50), true, MIN),
            Err("warming up")
        );
        assert_eq!(warm_up(&mut baseline, t0 + MIN, true, MIN), Ok(()));
    }

    #[test]
    fn a_failed_read_does_not_count_as_a_baseline() {
        let t0 = Instant::now();
        let mut baseline = None;
        assert_eq!(
            warm_up(&mut baseline, t0, false, MIN),
            Err("no cpu counters")
        );
        assert_eq!(
            warm_up(&mut baseline, t0 + Duration::from_secs(1), true, MIN),
            Err("warming up"),
            "the first good read only establishes the baseline"
        );
        // A failure after warm-up starts over.
        assert_eq!(
            warm_up(&mut baseline, t0 + Duration::from_secs(2), true, MIN),
            Ok(())
        );
        assert_eq!(
            warm_up(&mut baseline, t0 + Duration::from_secs(3), false, MIN),
            Err("no cpu counters")
        );
        assert_eq!(
            warm_up(&mut baseline, t0 + Duration::from_secs(4), true, MIN),
            Err("warming up")
        );
    }
}
