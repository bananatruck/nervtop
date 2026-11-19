use super::Source;
use crate::model::{ProcessInfo, Reading};
use std::time::{Duration, Instant};
use sysinfo::{
    ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, ThreadKind, UpdateKind, Users,
};
pub struct Processes {
    system: Option<System>,
    users: Option<Users>,
    users_at: Instant,
    warmed: bool,
    samples: u64,
}
impl Default for Processes {
    fn default() -> Self {
        Self {
            system: System::new().ok(),
            users: Users::new_with_refreshed_list().ok(),
            users_at: Instant::now(),
            warmed: false,
            samples: 0,
        }
    }
}
impl Source for Processes {
    type Output = Vec<ProcessInfo>;
    fn sample(&mut self) -> Reading<Self::Output> {
        let Some(s) = &mut self.system else {
            return Reading::unavailable("process initialization failed");
        };
        // UID is fetched once for new processes. No command, environment, disk or path refresh.
        let kind = refresh_kind(self.samples);
        self.samples = self.samples.wrapping_add(1);
        let updated = s.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        if let Err(reason) = scan_outcome(updated, &mut self.warmed) {
            return Reading::unavailable(reason);
        }
        let missed = s
            .processes()
            .values()
            .filter_map(|p| p.user_id())
            .any(|id| {
                self.users
                    .as_ref()
                    .and_then(|u| u.get_user_by_id(id))
                    .is_none()
            });
        if missed && self.users_at.elapsed() >= Duration::from_secs(60) {
            if let Some(users) = &mut self.users {
                users.refresh()
            } else {
                self.users = Users::new_with_refreshed_list().ok()
            }
            self.users_at = Instant::now();
        }
        let mut out: Vec<_> = s
            .processes()
            .values()
            .filter(|p| p.thread_kind() != Some(ThreadKind::Userland))
            .map(|p| ProcessInfo {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                user: p
                    .user_id()
                    .map(|id| {
                        self.users
                            .as_ref()
                            .and_then(|u| u.get_user_by_id(id))
                            .map(|u| u.name().to_owned())
                            .unwrap_or_else(|| id.to_string())
                    })
                    .unwrap_or_else(|| "unknown".into()),
                cpu_percent: p.cpu_usage(),
                memory_bytes: p.memory(),
                state: state(p.status()).into(),
                kernel_thread: p.thread_kind() == Some(ThreadKind::Kernel),
            })
            .collect();
        out.sort_by_key(|p| p.pid);
        Reading::Available(out)
    }
}
/// Every this many samples the UID of every process is read again, so a
/// daemon that changes its UID after start shows its new user within about
/// ten seconds at the default interval, without reading /proc/<pid>/status
/// for every process on every sample.
const USER_REFRESH_EVERY: u64 = 10;

/// CPU and memory for every process; the UID for new processes, and for all
/// of them every `USER_REFRESH_EVERY` samples.
fn refresh_kind(sample: u64) -> ProcessRefreshKind {
    let user = if sample.is_multiple_of(USER_REFRESH_EVERY) {
        UpdateKind::Always
    } else {
        UpdateKind::OnlyIfNotSet
    };
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_user(user)
        .without_tasks()
}

/// A scan that updated no process means /proc could not be read: report it
/// and start warm-up over, since CPU usage needs two good scans.
fn scan_outcome(updated: usize, warmed: &mut bool) -> Result<(), &'static str> {
    if updated == 0 {
        *warmed = false;
        return Err("process scan failed");
    }
    if !std::mem::replace(warmed, true) {
        return Err("warming up");
    }
    Ok(())
}

fn state(s: ProcessStatus) -> &'static str {
    match s {
        ProcessStatus::Idle => "idle",
        ProcessStatus::Run => "run",
        ProcessStatus::Sleep => "sleep",
        ProcessStatus::Stop => "stop",
        ProcessStatus::Zombie => "zombie",
        ProcessStatus::Tracing => "trace",
        ProcessStatus::Dead => "dead",
        ProcessStatus::Wakekill => "wakekill",
        ProcessStatus::Waking => "waking",
        ProcessStatus::Parked => "park",
        ProcessStatus::LockBlocked => "lock",
        ProcessStatus::UninterruptibleDiskSleep => "disk",
        ProcessStatus::Suspended => "suspend",
        ProcessStatus::Unknown(_) => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_ids_are_re_read_periodically() {
        // A daemon that drops privileges keeps its PID; its UID must be
        // re-read eventually, without paying for it on every sample.
        let always = (0..30)
            .filter(|n| refresh_kind(*n).user() == UpdateKind::Always)
            .count();
        assert_eq!(always, 3, "every {USER_REFRESH_EVERY}th sample");
        assert_eq!(refresh_kind(1).user(), UpdateKind::OnlyIfNotSet);
    }

    #[test]
    fn a_failed_scan_is_unavailable_and_restarts_warm_up() {
        let mut warmed = true;
        assert_eq!(scan_outcome(0, &mut warmed), Err("process scan failed"));
        assert!(!warmed);
        assert_eq!(scan_outcome(500, &mut warmed), Err("warming up"));
        assert_eq!(scan_outcome(500, &mut warmed), Ok(()));
    }

    #[test]
    fn refresh_skips_per_thread_task_scans() {
        // Walking /proc/<pid>/task/* for every process cost about 90 ms per
        // sample on a desktop with ~550 processes; threads are not shown.
        let kind = refresh_kind(0);
        assert!(!kind.tasks());
        assert!(kind.cpu() && kind.memory());
    }
}
