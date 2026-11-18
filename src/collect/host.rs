//! Host identity from sysinfo's independent host queries.
use super::HostSource;
use crate::model::{HostInfo, Reading};

pub struct Host;
impl HostSource for Host {
    fn sample(&mut self) -> HostInfo {
        HostInfo {
            hostname: sysinfo::System::host_name()
                .map(Reading::Available)
                .unwrap_or_else(|_| Reading::unavailable("hostname not reported")),
            kernel: sysinfo::System::kernel_version()
                .map(Reading::Available)
                .unwrap_or_else(|_| Reading::unavailable("kernel not reported")),
            uptime_secs: sysinfo::System::uptime()
                .map(Reading::Available)
                .unwrap_or_else(|_| Reading::unavailable("uptime not reported")),
        }
    }
}
