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
}
impl Default for Processes {
    fn default() -> Self {
        Self {
            system: System::new().ok(),
            users: Users::new_with_refreshed_list().ok(),
            users_at: Instant::now(),
            warmed: false,
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
        s.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_user(UpdateKind::OnlyIfNotSet),
        );
        if !std::mem::replace(&mut self.warmed, true) {
            return Reading::unavailable("warming up");
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
