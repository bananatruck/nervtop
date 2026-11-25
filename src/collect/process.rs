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
    /// (PID, start time) of every process in the previous successful scan:
    /// those, and only those, have a CPU baseline in this one.
    measured: std::collections::HashSet<(u32, u64)>,
}
impl Default for Processes {
    fn default() -> Self {
        Self {
            system: System::new().ok(),
            users: Users::new_with_refreshed_list().ok(),
            users_at: Instant::now(),
            warmed: false,
            samples: 0,
            measured: std::collections::HashSet::new(),
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
        let previous = std::mem::take(&mut self.measured);
        if updated > 0 {
            self.measured = s
                .processes()
                .values()
                .map(|p| (p.pid().as_u32(), p.start_time()))
                .collect();
        }
        if let Err(reason) = scan_outcome(updated, &mut self.warmed) {
            return Reading::unavailable(reason);
        }
        let boot_time = System::boot_time().ok();
        // SAFETY: sysconf has no preconditions and only reads a constant.
        let ticks_per_sec = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) })
            .ok()
            .filter(|t| *t > 0);
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
                name: ticks_per_sec
                    .zip(boot_time)
                    .and_then(|(ticks, boot)| {
                        current_name(p.pid().as_u32(), p.start_time(), boot, ticks)
                    })
                    .unwrap_or_else(|| p.name().to_string_lossy().into_owned()),
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
                // sysinfo leaves 0 until a process has two scans of CPU
                // time; that 0 is not a measurement.
                cpu_percent: if previous.contains(&(p.pid().as_u32(), p.start_time())) {
                    Reading::Available(p.cpu_usage())
                } else {
                    Reading::unavailable("warming up")
                },
                memory_bytes: p.memory(),
                state: state(p.status()).into(),
                kernel_thread: p.thread_kind() == Some(ThreadKind::Kernel),
            })
            .collect();
        out.sort_by_key(|p| p.pid);
        Reading::Available(out)
    }
}
/// The command name in `/proc/<pid>/stat`, when that file still describes
/// the sampled process (the same start time, in seconds since the epoch as
/// sysinfo computes it). sysinfo keeps the name it read when it first saw a
/// PID, so after `exec` its name is the old program's; a PID reused since
/// the scan gives `None` here, and the caller keeps sysinfo's name.
fn current_name(pid: u32, start_time: u64, boot_time: u64, ticks_per_sec: u64) -> Option<String> {
    let stat = std::fs::read(format!("/proc/{pid}/stat")).ok()?;
    stat_name(&stat, start_time, boot_time, ticks_per_sec)
}

/// The name in a `/proc/<pid>/stat` record whose start time matches. The
/// name sits between the first `(` and the last `)`, and can hold spaces and
/// parentheses; start time is field 22, the 20th after the name.
fn stat_name(stat: &[u8], start_time: u64, boot_time: u64, ticks_per_sec: u64) -> Option<String> {
    let open = stat.iter().position(|b| *b == b'(')?;
    let close = stat.iter().rposition(|b| *b == b')')?;
    let name = String::from_utf8_lossy(stat.get(open + 1..close)?).into_owned();
    let rest = std::str::from_utf8(stat.get(close + 1..)?).ok()?;
    let ticks: u64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    (ticks / ticks_per_sec + boot_time == start_time).then_some(name)
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

    fn row(p: &mut Processes, pid: u32) -> ProcessInfo {
        let reading = p.sample();
        let list = reading.value().expect("process scan");
        list.iter()
            .find(|r| r.pid == pid)
            .expect("child listed")
            .clone()
    }

    fn comm(pid: u32) -> String {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .unwrap_or_default()
            .trim()
            .to_owned()
    }

    /// A process that calls exec keeps its PID and start time; its listed
    /// name must follow the new program.
    #[test]
    fn a_process_name_follows_exec() {
        use std::io::Write;
        let mut child = std::process::Command::new("sh")
            .args(["-c", "read line; exec sleep 30"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut p = Processes::default();
        p.sample();
        assert_eq!(row(&mut p, pid).name, "sh");
        child.stdin.as_mut().unwrap().write_all(b"go\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while comm(pid) != "sleep" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let name = row(&mut p, pid).name;
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(name, "sleep");
    }

    /// A busy process that appears after warm-up has no CPU baseline yet;
    /// its first sample must not be published as measured idle time.
    #[test]
    fn a_new_process_is_not_reported_idle_before_its_cpu_is_measured() {
        let mut p = Processes::default();
        p.sample();
        p.sample();
        let mut child = std::process::Command::new("sh")
            .args(["-c", "while :; do :; done"])
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let first = row(&mut p, child.id());
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(first.cpu_percent.reason(), Some("warming up"));
    }

    #[test]
    fn a_stat_name_is_used_only_for_the_same_process() {
        // Field 22 (start time) is 500 ticks: 5 s after boot at 100 ticks/s.
        let stat = b"42 (my (odd) name) S 1 42 42 0 -1 4194560 0 0 0 0 0 0 0 0 20 0 1 0 500 0 0";
        assert_eq!(
            stat_name(stat, 1_005, 1_000, 100).as_deref(),
            Some("my (odd) name")
        );
        assert_eq!(stat_name(stat, 1_006, 1_000, 100), None, "a reused PID");
        assert_eq!(stat_name(b"42 garbage", 1_005, 1_000, 100), None);
    }

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
