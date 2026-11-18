use super::Source;
use crate::model::{CpuStats, Reading};
use sysinfo::{CpuRefreshKind, System};

pub struct Cpu {
    system: Option<System>,
    warmed: bool,
}
impl Default for Cpu {
    fn default() -> Self {
        Self {
            system: System::new().ok(),
            warmed: false,
        }
    }
}
impl Source for Cpu {
    type Output = CpuStats;
    fn sample(&mut self) -> Reading<CpuStats> {
        let Some(s) = &mut self.system else {
            return Reading::unavailable("cpu initialization failed");
        };
        s.refresh_cpu_specifics(CpuRefreshKind::everything());
        if !std::mem::replace(&mut self.warmed, true) {
            return Reading::unavailable("warming up");
        }
        if s.cpus().is_empty() {
            return Reading::unavailable("no cpu counters");
        }
        let frequencies: Vec<u64> = s
            .cpus()
            .iter()
            .map(|c| c.frequency())
            .filter(|f| *f > 0)
            .collect();
        let Ok(load) = System::load_average() else {
            return Reading::unavailable("load average not reported");
        };
        Reading::Available(CpuStats {
            per_core: s.cpus().iter().map(|c| c.usage()).collect(),
            total: s.global_cpu_usage(),
            load_avg: [load.one, load.five, load.fifteen],
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
