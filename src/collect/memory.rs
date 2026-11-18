use super::Source;
use crate::model::{MemoryStats, Reading};
use sysinfo::{MemoryRefreshKind, System};
pub struct Memory {
    system: Option<System>,
}
impl Default for Memory {
    fn default() -> Self {
        Self {
            system: System::new().ok(),
        }
    }
}
impl Source for Memory {
    type Output = MemoryStats;
    fn sample(&mut self) -> Reading<MemoryStats> {
        let Some(s) = &mut self.system else {
            return Reading::unavailable("memory initialization failed");
        };
        s.refresh_memory_specifics(MemoryRefreshKind::everything());
        if s.total_memory() == 0 {
            return Reading::unavailable("memory not reported");
        }
        Reading::Available(MemoryStats {
            total_bytes: s.total_memory(),
            available_bytes: s.available_memory(),
            swap_total_bytes: s.total_swap(),
            swap_used_bytes: s.used_swap(),
        })
    }
}
