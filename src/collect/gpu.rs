use super::{Source, entries, number, read};
use crate::model::{GpuPower, GpuStats, GpuVendor, Reading};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// External command boundary, replaceable without executing a GPU query in tests.
pub trait CommandRunner: Send {
    /// Shares an outstanding child cleanup with the collector, when supported.
    fn pending_reaper(&self) -> Option<Arc<AtomicBool>> {
        None
    }

    fn run(&mut self, program: &str, args: &[&str], timeout: Duration) -> Result<String, String>;
}
#[derive(Default)]
pub struct SystemCommandRunner {
    pending: Arc<AtomicBool>,
}

const MAX_STDOUT: usize = 64 * 1024;

/// Reads `file` until end of file, `deadline`, or more than `limit` bytes,
/// whichever comes first, waiting with poll(2) so it never blocks past the
/// deadline. The descriptor is closed on return: a descendant of nvidia-smi
/// that keeps the pipe open can no longer pin a thread or a file descriptor.
fn read_bounded(
    mut file: std::fs::File,
    deadline: Instant,
    limit: usize,
) -> Result<String, String> {
    use std::os::fd::AsRawFd;
    let failed = || "nvidia-smi output failed".to_owned();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("nvidia-smi timeout".into());
        }
        let mut fd = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = libc::c_int::try_from(left.as_millis().max(1)).unwrap_or(libc::c_int::MAX);
        // SAFETY: `fd` is one valid pollfd for an open descriptor owned by `file`.
        let ready = unsafe { libc::poll(&mut fd, 1, ms) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(failed());
        }
        if ready == 0 {
            continue;
        }
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.len() > limit {
                    return Err("nvidia-smi output too large".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(failed()),
        }
    }
    String::from_utf8(bytes).map_err(|_| failed())
}

/// The first readable PCI ID database under `root`.
fn load_pci_ids(root: &Path) -> Option<String> {
    ["usr/share/hwdata/pci.ids", "usr/share/misc/pci.ids"]
        .into_iter()
        .find_map(|p| read(root.join(p)))
}

fn background_reaper(
    pending: Arc<AtomicBool>,
    reap: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    pending.store(true, Ordering::SeqCst);
    std::thread::spawn(move || {
        reap();
        pending.store(false, Ordering::SeqCst);
    })
}

/// How long a killed nvidia-smi gets to be reaped before it is handed to a
/// background reaper. A process in uninterruptible sleep ignores SIGKILL until
/// the kernel call returns, so this must stay bounded.
const KILL_GRACE: Duration = Duration::from_millis(200);

/// Polls `try_wait` until the child reports an exit or `grace` has passed on
/// the `now` clock. Returns whether the child exited; never blocks for more
/// than `grace` plus one poll step.
fn reaped_within(
    mut try_wait: impl FnMut() -> std::io::Result<Option<std::process::ExitStatus>>,
    now: impl Fn() -> Instant,
    grace: Duration,
) -> bool {
    let deadline = now() + grace;
    loop {
        if let Ok(Some(_)) = try_wait() {
            return true;
        }
        if now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

impl CommandRunner for SystemCommandRunner {
    fn pending_reaper(&self) -> Option<Arc<AtomicBool>> {
        Some(self.pending.clone())
    }

    fn run(&mut self, program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
        if self.pending.load(Ordering::SeqCst) {
            return Err("previous nvidia-smi still running".into());
        }
        let mut child = Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    "nvidia-smi not found".to_owned()
                } else {
                    "nvidia-smi spawn failed".to_owned()
                }
            })?;
        let stdout = child.stdout.take().ok_or("nvidia-smi stdout unavailable")?;
        let stdout = std::fs::File::from(std::os::fd::OwnedFd::from(stdout));
        let start = Instant::now();
        let deadline = start + timeout;
        let result = read_bounded(stdout, deadline, MAX_STDOUT).and_then(|text| {
            loop {
                match child.try_wait() {
                    Ok(Some(status)) if status.success() => break Ok(text),
                    Ok(Some(_)) => break Err("nvidia-smi non-zero exit".to_owned()),
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Ok(None) => break Err("nvidia-smi timeout".to_owned()),
                    Err(_) => break Err("nvidia-smi wait failed".to_owned()),
                }
            }
        });
        if result.is_err() {
            let _ = child.kill();
            if !reaped_within(|| child.try_wait(), Instant::now, KILL_GRACE) {
                background_reaper(self.pending.clone(), move || {
                    let _ = child.wait();
                });
            }
        }
        result
    }
}

pub struct Gpus {
    root: PathBuf,
    runner: Box<dyn CommandRunner>,
    pending_reaper: Arc<AtomicBool>,
    names: Option<String>,
    name_cache: BTreeMap<(u16, u16), Option<String>>,
    cached: Result<BTreeMap<String, GpuStats>, String>,
    /// Device ID of each NVIDIA slot when `cached` was queried, so a cached
    /// row is never applied to a different card at the same slot.
    cached_ids: BTreeMap<String, Option<u16>>,
    queried_at: Option<Instant>,
}
impl Gpus {
    pub fn new(root: PathBuf) -> Self {
        Self::with_runner(root, Box::new(SystemCommandRunner::default()))
    }
    pub fn with_runner(root: PathBuf, runner: Box<dyn CommandRunner>) -> Self {
        // Only retain the first readable database; do not probe it every tick.
        let names = load_pci_ids(&root);
        Self {
            root,
            pending_reaper: runner.pending_reaper().unwrap_or_default(),
            runner,
            names,
            name_cache: BTreeMap::new(),
            cached: Err("warming up".into()),
            cached_ids: BTreeMap::new(),
            queried_at: None,
        }
    }
    fn query(&mut self, nvidia_ids: &BTreeMap<String, Option<u16>>) {
        // The throttle keeps counting from the last command while its
        // cleanup is pending, so the next command runs only after cleanup
        // has finished and two seconds have passed since the last one.
        if self.pending_reaper.load(Ordering::SeqCst) {
            self.cached = Err("previous nvidia-smi still running".into());
            return;
        }
        if self
            .queried_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.cached_ids = nvidia_ids.clone();
        self.cached = self.runner.run("nvidia-smi", &["--query-gpu=pci.bus_id,name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw", "--format=csv,noheader,nounits"], Duration::from_secs(2)).map(|text| parse_csv(&text));
        self.queried_at = Some(Instant::now());
    }
}
fn unavailable(
    slot: String,
    vendor: GpuVendor,
    name: String,
    power_state: GpuPower,
    reason: &str,
) -> GpuStats {
    GpuStats {
        pci_slot: slot,
        vendor,
        name,
        power_state,
        utilization_percent: Reading::unavailable(reason),
        memory_used_bytes: Reading::unavailable(reason),
        memory_total_bytes: Reading::unavailable(reason),
        temperature_celsius: Reading::unavailable(reason),
        power_watts: Reading::unavailable(reason),
    }
}
fn normalize_slot(slot: &str) -> Option<String> {
    let (domain, rest) = slot.trim().split_once(':')?;
    let (bus, tail) = rest.split_once(':')?;
    let (device, function) = tail.split_once('.')?;
    Some(format!(
        "{:04x}:{:02x}:{:02x}.{:x}",
        u32::from_str_radix(domain, 16).ok()?,
        u8::from_str_radix(bus, 16).ok()?,
        u8::from_str_radix(device, 16).ok()?,
        u8::from_str_radix(function, 16).ok()?
    ))
}
fn valid_number(value: f64, min: f64, max: f64) -> Reading<f64> {
    if value.is_finite() && (min..=max).contains(&value) {
        Reading::Available(value)
    } else {
        Reading::unavailable("invalid value")
    }
}
fn csv_number(text: &str, min: f64, max: f64) -> Reading<f64> {
    text.trim()
        .parse::<f64>()
        .map(|v| valid_number(v, min, max))
        .unwrap_or_else(|_| Reading::unavailable("not reported"))
}
fn csv_memory(text: &str) -> Reading<u64> {
    match text
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|v| v.checked_mul(1048576))
    {
        Some(bytes) => Reading::Available(bytes),
        None if text.trim().parse::<f64>().is_ok() => Reading::unavailable("invalid value"),
        None => Reading::unavailable("not reported"),
    }
}
fn sys_value(dir: &Path, file: &str, scale: f64, min: f64, max: f64) -> Reading<f64> {
    match read(dir.join(file)) {
        Some(text) => csv_number(&text, min * scale, max * scale).map(|v| v / scale),
        None => Reading::unavailable(format!("{file} not reported")),
    }
}
fn parse_csv(text: &str) -> BTreeMap<String, GpuStats> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            if fields.len() < 7 {
                return None;
            }
            let slot = normalize_slot(fields[0])?;
            let numeric = &fields[fields.len() - 5..];
            Some((
                slot.clone(),
                GpuStats {
                    pci_slot: slot,
                    vendor: GpuVendor::Nvidia,
                    name: fields[1..fields.len() - 5].join(","),
                    power_state: GpuPower::Active,
                    utilization_percent: csv_number(numeric[0], 0.0, 100.0).map(|v| v as f32),
                    memory_used_bytes: csv_memory(numeric[1]),
                    memory_total_bytes: csv_memory(numeric[2]),
                    temperature_celsius: csv_number(numeric[3], -50.0, 200.0),
                    power_watts: csv_number(numeric[4], 0.0, 1000.0),
                },
            ))
        })
        .collect()
}
fn pci_name(text: &str, vendor: u16, device: u16) -> Option<String> {
    let mut in_vendor = false;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !line.starts_with('\t') {
            in_vendor = line
                .split_whitespace()
                .next()
                .and_then(|s| u16::from_str_radix(s, 16).ok())
                == Some(vendor);
        } else if in_vendor && !line.starts_with("\t\t") {
            let line = line.trim_start();
            if line.get(..4).and_then(|s| u16::from_str_radix(s, 16).ok()) == Some(device) {
                return line
                    .get(4..)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
            }
        }
    }
    None
}
fn sys_number<T: std::str::FromStr>(dir: &Path, file: &str) -> Reading<T> {
    number(dir.join(file))
        .map(Reading::Available)
        .unwrap_or_else(|| Reading::unavailable(format!("{file} not reported")))
}
/// Whether a runtime PM status means the device is powered and staying so:
/// "active", "unsupported" (runtime PM disabled, so always powered) or no
/// status file at all. Anything else ("suspended", "suspending", "resuming",
/// "error" or an unrecognized value) means nvidia-smi must not run.
fn runtime_settled_awake(runtime: &std::io::Result<Option<String>>) -> bool {
    matches!(runtime, Ok(None))
        || matches!(runtime, Ok(Some(status)) if matches!(status.as_str(), "active" | "unsupported"))
}
impl Source for Gpus {
    type Output = Vec<GpuStats>;
    fn sample(&mut self) -> Reading<Self::Output> {
        let mut out = Vec::new();
        // Read every display adapter's IDs and runtime PM status once, before
        // deciding anything: `nvidia-smi --query-gpu` touches every NVIDIA
        // device, so one asleep NVIDIA adapter rules out the query for all.
        let adapters: Vec<_> = entries(self.root.join("sys/bus/pci/devices"))
            .into_iter()
            .filter(|dir| read(dir.join("class")).is_some_and(|c| c.starts_with("0x03")))
            .map(|dir| {
                let hex = |file: &str| {
                    read(dir.join(file))
                        .and_then(|v| u16::from_str_radix(v.trim_start_matches("0x"), 16).ok())
                };
                let ids = (hex("vendor"), hex("device"));
                let runtime = match std::fs::read_to_string(dir.join("power/runtime_status")) {
                    Ok(status) => Ok(Some(status.trim().to_owned())),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(e),
                };
                (dir, ids, runtime)
            })
            .collect();
        let nvidia_asleep = adapters.iter().any(|(_, (vendor, _), runtime)| {
            *vendor == Some(0x10de) && !runtime_settled_awake(runtime)
        });
        let nvidia_ids: BTreeMap<String, Option<u16>> = adapters
            .iter()
            .filter(|(_, (vendor, _), _)| *vendor == Some(0x10de))
            .filter_map(|(dir, (_, device), _)| {
                normalize_slot(&dir.file_name()?.to_string_lossy()).map(|slot| (slot, *device))
            })
            .collect();
        let nvidia_unreadable = adapters
            .iter()
            .any(|(_, (vendor, _), runtime)| *vendor == Some(0x10de) && runtime.is_err());
        for (dir, (vendor_id, device_id), runtime) in adapters {
            let status = runtime.as_ref().ok().and_then(|s| s.as_deref());
            let (vendor, label) = match vendor_id {
                Some(0x1002) => (GpuVendor::Amd, "AMD"),
                Some(0x10de) => (GpuVendor::Nvidia, "NVIDIA"),
                Some(0x8086) => (GpuVendor::Intel, "Intel"),
                _ => (GpuVendor::Other, "Unknown"),
            };
            let root = &self.root;
            let names = &self.names;
            let name = vendor_id
                .zip(device_id)
                .and_then(|(v, d)| {
                    self.name_cache
                        .entry((v, d))
                        .or_insert_with(|| {
                            // The database text is dropped after the first
                            // scan; a device first seen later (a hot-plugged
                            // eGPU) re-reads it for this one lookup.
                            let reloaded;
                            let text = match names.as_deref() {
                                Some(t) => Some(t),
                                None => {
                                    reloaded = load_pci_ids(root);
                                    reloaded.as_deref()
                                }
                            };
                            text.and_then(|t| pci_name(t, v, d))
                                .map(|n| format!("{label} {n}"))
                        })
                        .clone()
                })
                .unwrap_or_else(|| match (vendor_id, device_id) {
                    (Some(v), Some(d)) => format!("{label} GPU [{v:04x}:{d:04x}]"),
                    _ => format!("{label} GPU [pci id not reported]"),
                });
            let slot = dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if matches!(status, Some("suspended" | "suspending")) {
                out.push(unavailable(
                    slot,
                    vendor,
                    name,
                    GpuPower::Suspended,
                    "gpu suspended",
                ));
                continue;
            }
            let mut gpu = unavailable(
                slot.clone(),
                vendor,
                name,
                GpuPower::Active,
                "gpu metrics not supported",
            );
            match vendor {
                GpuVendor::Nvidia if runtime.is_err() => {
                    gpu = unavailable(
                        slot,
                        vendor,
                        gpu.name,
                        GpuPower::Unknown,
                        "gpu power state unreadable",
                    );
                }
                GpuVendor::Nvidia if !runtime_settled_awake(&runtime) => {
                    let reason = match status {
                        Some("resuming") => "gpu resuming",
                        Some("error") => "gpu runtime pm error",
                        _ => "gpu runtime pm status unrecognized",
                    };
                    gpu = unavailable(slot, vendor, gpu.name, GpuPower::Active, reason);
                }
                // This adapter is awake; another one's state rules out the
                // query, which touches every NVIDIA device.
                GpuVendor::Nvidia if nvidia_unreadable => {
                    gpu = unavailable(
                        slot,
                        vendor,
                        gpu.name,
                        GpuPower::Active,
                        "not queried: another nvidia gpu's power state is unreadable",
                    );
                }
                GpuVendor::Nvidia if nvidia_asleep => {
                    gpu = unavailable(
                        slot,
                        vendor,
                        gpu.name,
                        GpuPower::Active,
                        "not queried: another nvidia gpu is asleep",
                    );
                }
                GpuVendor::Nvidia => {
                    self.query(&nvidia_ids);
                    match &self.cached {
                        Ok(rows) => {
                            let key = normalize_slot(&slot);
                            let same_card = key
                                .as_ref()
                                .and_then(|k| self.cached_ids.get(k))
                                .is_some_and(|id| id.is_some() && *id == device_id);
                            if !same_card {
                                gpu = unavailable(
                                    slot,
                                    vendor,
                                    gpu.name,
                                    GpuPower::Active,
                                    "gpu changed since last query",
                                );
                            } else if let Some(row) = key.and_then(|slot| rows.get(&slot)) {
                                let fallback_name = gpu.name;
                                gpu = row.clone();
                                if gpu.name.is_empty() || gpu.name.starts_with('[') {
                                    gpu.name = fallback_name;
                                }
                                gpu.pci_slot = slot;
                            } else {
                                gpu = unavailable(
                                    slot,
                                    vendor,
                                    gpu.name,
                                    GpuPower::Active,
                                    "nvidia-smi device not found",
                                );
                            }
                        }
                        Err(reason) => {
                            gpu = unavailable(slot, vendor, gpu.name, GpuPower::Active, reason)
                        }
                    }
                }
                GpuVendor::Amd => {
                    gpu.utilization_percent =
                        sys_value(&dir, "gpu_busy_percent", 1.0, 0.0, 100.0).map(|v| v as f32);
                    gpu.memory_used_bytes = sys_number(&dir, "mem_info_vram_used");
                    gpu.memory_total_bytes = sys_number(&dir, "mem_info_vram_total");
                    gpu.temperature_celsius = Reading::unavailable("gpu temperature not reported");
                    gpu.power_watts = Reading::unavailable("gpu power not reported");
                    for hwmon in entries(dir.join("hwmon")) {
                        if hwmon.join("temp1_input").exists() {
                            gpu.temperature_celsius =
                                sys_value(&hwmon, "temp1_input", 1000.0, -50.0, 200.0);
                        }
                        let power_file = if hwmon.join("power1_average").exists() {
                            "power1_average"
                        } else {
                            "power1_input"
                        };
                        if hwmon.join(power_file).exists() {
                            gpu.power_watts = sys_value(&hwmon, power_file, 1e6, 0.0, 1000.0);
                        }
                    }
                }
                GpuVendor::Intel | GpuVendor::Other => {}
            }
            out.push(gpu);
        }
        // Every present adapter's name is cached now; release the database
        // text (over a megabyte) instead of holding it for the whole session.
        self.names = None;
        if out.is_empty() {
            Reading::unavailable("no display adapters found")
        } else {
            Reading::Available(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    #[test]
    fn timed_out_runner_closes_the_descendants_output_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let pipe_file = dir.path().join("pipe");
        let result = SystemCommandRunner::default().run(
            "sh",
            &[
                "-c",
                "readlink /proc/$$/fd/1 > \"$1\"; sleep 1 &",
                "sh",
                pipe_file.to_str().unwrap(),
            ],
            Duration::from_millis(200),
        );
        let pipe = std::fs::read_to_string(&pipe_file).unwrap();
        let held_open = std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .flatten()
            .any(|fd| {
                std::fs::read_link(fd.path()).is_ok_and(|target| target.as_os_str() == pipe.trim())
            });
        // Let the short-lived fixture descendant exit before this test ends.
        std::thread::sleep(Duration::from_secs(1));
        assert!(result.is_err());
        assert!(!held_open, "runner left {} open", pipe.trim());
    }

    fn pipe() -> (std::fs::File, std::fs::File) {
        use std::os::fd::FromRawFd;
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        // SAFETY: pipe2(2) just returned these two new descriptors.
        unsafe {
            (
                std::fs::File::from_raw_fd(fds[0]),
                std::fs::File::from_raw_fd(fds[1]),
            )
        }
    }

    #[test]
    fn fixture_pipe_reaches_eof_while_an_unrelated_child_is_running() {
        use std::io::Write;
        let (reader, mut writer) = pipe();
        writer.write_all(b"row\n").unwrap();
        let mut child = Command::new("sleep").arg("3").spawn().unwrap();
        drop(writer);
        let result = read_bounded(reader, Instant::now() + Duration::from_millis(200), 4);
        let _ = child.kill();
        child.wait().unwrap();
        assert_eq!(result, Ok("row\n".to_owned()));
    }

    #[test]
    fn bounded_read_returns_at_the_deadline_while_a_writer_holds_the_pipe() {
        use std::io::Write;
        let (reader, mut writer) = pipe();
        writer.write_all(b"partial").unwrap();
        let start = Instant::now();
        let out = read_bounded(reader, start + Duration::from_millis(150), MAX_STDOUT);
        assert!(
            start.elapsed() < Duration::from_millis(600),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(out, Err("nvidia-smi timeout".to_owned()));
        drop(writer);
    }

    #[test]
    fn bounded_read_returns_all_output_at_eof_and_enforces_the_limit() {
        use std::io::Write;
        let (reader, mut writer) = pipe();
        writer.write_all(b"row 1\nrow 2\n").unwrap();
        drop(writer);
        let deadline = Instant::now() + Duration::from_secs(2);
        assert_eq!(
            read_bounded(reader, deadline, MAX_STDOUT),
            Ok("row 1\nrow 2\n".to_owned())
        );
        let (reader, mut writer) = pipe();
        writer.write_all(&[b'x'; 64]).unwrap();
        drop(writer);
        assert_eq!(
            read_bounded(reader, deadline, 10),
            Err("nvidia-smi output too large".to_owned())
        );
    }

    #[test]
    fn a_new_device_id_is_named_from_pci_ids_even_after_the_first_scan() {
        let t = tempfile::tempdir().unwrap();
        put(
            t.path(),
            "usr/share/hwdata/pci.ids",
            "1002  Advanced Micro Devices\n\t15bf  Phoenix1\n\t7480  Navi 33\n",
        );
        let (mut g, _) = source(t.path(), Ok(CSV.into()));
        g.sample();
        device(t.path(), "0000:03:00.0", "0x1002", "0x7480", "active");
        let gpus = g.sample();
        assert_eq!(gpus.value().unwrap()[0].name, "AMD Navi 33");
        std::fs::remove_file(t.path().join("usr/share/hwdata/pci.ids")).unwrap();
        assert_eq!(g.sample().value().unwrap()[0].name, "AMD Navi 33");
    }

    #[test]
    fn a_cached_row_is_not_applied_to_a_replacement_device() {
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", "active");
        let (mut g, calls) = source(t.path(), Ok(CSV.into()));
        assert!(
            g.sample().value().unwrap()[0]
                .utilization_percent
                .is_available()
        );
        // A different card appears at the same slot inside the 2 s cache window.
        device(t.path(), "0000:01:00.0", "0x10de", "0x2860", "active");
        let gpu = g.sample().value().unwrap()[0].clone();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "still inside the rate limit"
        );
        assert_eq!(
            gpu.utilization_percent.reason(),
            Some("gpu changed since last query")
        );
        assert_ne!(gpu.name, "NVIDIA GeForce RTX 4060 Laptop GPU");
        g.queried_at = Some(Instant::now() - Duration::from_secs(3));
        assert!(
            g.sample().value().unwrap()[0]
                .utilization_percent
                .is_available()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        std::fs::remove_file(t.path().join("sys/bus/pci/devices/0000:01:00.0/device")).unwrap();
        assert!(
            !g.sample().value().unwrap()[0]
                .utilization_percent
                .is_available()
        );
    }
    use crate::collect::sensors::tests::put;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    const CSV: &str = "00000000:01:00.0, NVIDIA GeForce RTX 4060 Laptop GPU, 17, 321, 8188, 48, 12.51\n00000000:02:00.0, NVIDIA GPU, [N/A], [Not Supported], 4096, , [N/A]\nmalformed,line\n";
    struct Runner {
        calls: Arc<AtomicUsize>,
        result: Result<String, String>,
    }
    impl CommandRunner for Runner {
        fn run(
            &mut self,
            program: &str,
            args: &[&str],
            timeout: Duration,
        ) -> Result<String, String> {
            assert_eq!(program, "nvidia-smi");
            assert_eq!(args.len(), 2);
            assert_eq!(timeout, Duration::from_secs(2));
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result.clone()
        }
    }
    fn device(root: &Path, slot: &str, vendor: &str, id: &str, runtime: &str) {
        for (file, text) in [
            ("class", "0x030000"),
            ("vendor", vendor),
            ("device", id),
            ("power/runtime_status", runtime),
        ] {
            put(root, &format!("sys/bus/pci/devices/{slot}/{file}"), text);
        }
    }
    fn source(root: &Path, result: Result<String, String>) -> (Gpus, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Gpus::with_runner(
                root.into(),
                Box::new(Runner {
                    calls: calls.clone(),
                    result,
                }),
            ),
            calls,
        )
    }
    #[test]
    fn unreadable_runtime_status_blocks_all_nvidia_queries() {
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", "active");
        device(t.path(), "0000:02:00.0", "0x10de", "0x28e0", "active");
        std::fs::write(
            t.path()
                .join("sys/bus/pci/devices/0000:02:00.0/power/runtime_status"),
            [0xff],
        )
        .unwrap();
        let (mut g, calls) = source(t.path(), Ok(CSV.into()));
        let reading = g.sample();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let rows = reading.value().unwrap();
        // The awake peer keeps its known state; only why it was not queried
        // is about the other adapter.
        assert_eq!(rows[0].power_state, GpuPower::Active);
        assert_eq!(
            rows[0].power_watts.reason(),
            Some("not queried: another nvidia gpu's power state is unreadable")
        );
        // The adapter whose status could not be read is not called active.
        assert_eq!(rows[1].power_state, GpuPower::Unknown);
        assert_eq!(
            rows[1].power_watts.reason(),
            Some("gpu power state unreadable")
        );
    }
    #[test]
    fn invalid_gpu_values_are_unavailable() {
        for value in ["NaN", "inf", "-1", "1e100", "101"] {
            let rows = parse_csv(&format!("0000:01:00.0, GPU, {value}, 0, 0, 0, 0"));
            assert_eq!(
                rows["0000:01:00.0"].utilization_percent.reason(),
                Some("invalid value"),
                "{value}"
            );
            let t = tempfile::tempdir().unwrap();
            device(t.path(), "0000:05:00.0", "0x1002", "0x15bf", "active");
            put(
                t.path(),
                "sys/bus/pci/devices/0000:05:00.0/gpu_busy_percent",
                value,
            );
            assert_eq!(
                Gpus::new(t.path().into()).sample().value().unwrap()[0]
                    .utilization_percent
                    .reason(),
                Some("invalid value"),
                "{value}"
            );
        }
        for value in ["NaN", "inf", "-51", "1e100", "201"] {
            let rows = parse_csv(&format!("0000:01:00.0, GPU, 0, 0, 0, {value}, 0"));
            assert_eq!(
                rows["0000:01:00.0"].temperature_celsius.reason(),
                Some("invalid value"),
                "{value}"
            );
        }
        for value in ["NaN", "inf", "-1", "1e100", "1001"] {
            let rows = parse_csv(&format!("0000:01:00.0, GPU, 0, 0, 0, 0, {value}"));
            assert_eq!(
                rows["0000:01:00.0"].power_watts.reason(),
                Some("invalid value"),
                "{value}"
            );
        }
        for value in [
            "NaN",
            "inf",
            "-1",
            "1e100",
            "17592186044416",
            "18446744073709551616",
        ] {
            let rows = parse_csv(&format!("0000:01:00.0, GPU, 0, {value}, {value}, 0, 0"));
            let row = &rows["0000:01:00.0"];
            assert_eq!(
                row.memory_used_bytes.reason(),
                Some("invalid value"),
                "{value}"
            );
            assert_eq!(
                row.memory_total_bytes.reason(),
                Some("invalid value"),
                "{value}"
            );
        }
        let rows = parse_csv("0000:01:00.0, GPU, 100, 17592186044415, 0, -50, 1000");
        let row = &rows["0000:01:00.0"];
        assert_eq!(row.temperature_celsius, Reading::Available(-50.0));
        assert_eq!(
            row.memory_used_bytes,
            Reading::Available(18446744073708503040)
        );
    }
    #[test]
    fn csv_names_with_commas_and_short_rows() {
        let rows = parse_csv(
            "0000:01:00.0, NVIDIA GPU, Special Edition, 17, 321, 8188, 48, 12.51\n0000:02:00.0, GPU, 1, 2, 3, 4",
        );
        assert_eq!(rows.len(), 1);
        let row = &rows["0000:01:00.0"];
        assert_eq!(row.name, "NVIDIA GPU,Special Edition");
        assert_eq!(row.utilization_percent, Reading::Available(17.0));
        assert_eq!(row.power_watts, Reading::Available(12.51));
    }
    #[test]
    fn command_uses_c_locale() {
        assert_eq!(
            SystemCommandRunner::default()
                .run(
                    "sh",
                    &["-c", "printf %s \"$LC_ALL\""],
                    Duration::from_secs(2)
                )
                .unwrap(),
            "C"
        );
    }
    #[test]
    fn pending_reaper_blocks_queries_until_it_finishes() {
        struct DeferredRunner {
            pending: Arc<AtomicBool>,
            calls: Arc<AtomicUsize>,
        }
        impl CommandRunner for DeferredRunner {
            fn pending_reaper(&self) -> Option<Arc<AtomicBool>> {
                Some(self.pending.clone())
            }
            fn run(&mut self, _: &str, _: &[&str], _: Duration) -> Result<String, String> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.pending.store(true, Ordering::SeqCst);
                Err("nvidia-smi timeout".into())
            }
        }
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", "active");
        let pending = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut g = Gpus::with_runner(
            t.path().into(),
            Box::new(DeferredRunner {
                pending: pending.clone(),
                calls: calls.clone(),
            }),
        );
        assert_eq!(
            g.sample().value().unwrap()[0].power_watts.reason(),
            Some("nvidia-smi timeout")
        );
        assert_eq!(
            g.sample().value().unwrap()[0].power_watts.reason(),
            Some("previous nvidia-smi still running")
        );
        g.queried_at = Some(Instant::now() - Duration::from_secs(3));
        assert_eq!(
            g.sample().value().unwrap()[0].power_watts.reason(),
            Some("previous nvidia-smi still running")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        pending.store(false, Ordering::SeqCst);
        g.sample();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    /// A reaper finishing early does not let a query start before the
    /// throttle since the last one has expired.
    #[test]
    fn a_finished_reaper_still_waits_for_the_query_throttle() {
        struct DeferredRunner {
            pending: Arc<AtomicBool>,
            calls: Arc<AtomicUsize>,
        }
        impl CommandRunner for DeferredRunner {
            fn pending_reaper(&self) -> Option<Arc<AtomicBool>> {
                Some(self.pending.clone())
            }
            fn run(&mut self, _: &str, _: &[&str], _: Duration) -> Result<String, String> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.pending.store(true, Ordering::SeqCst);
                Err("nvidia-smi timeout".into())
            }
        }
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", "active");
        let pending = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut g = Gpus::with_runner(
            t.path().into(),
            Box::new(DeferredRunner {
                pending: pending.clone(),
                calls: calls.clone(),
            }),
        );
        g.sample();
        g.sample();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Cleanup finishes within the throttle: no new command yet.
        pending.store(false, Ordering::SeqCst);
        g.sample();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Once the throttle has expired, the next query runs.
        g.queried_at = Some(Instant::now() - Duration::from_secs(3));
        g.sample();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn oversized_command_output_is_rejected() {
        let mut runner = SystemCommandRunner::default();
        assert_eq!(
            runner
                .run(
                    "head",
                    &["-c", "65537", "/dev/zero"],
                    Duration::from_secs(2)
                )
                .err()
                .as_deref(),
            Some("nvidia-smi output too large")
        );
    }
    #[test]
    fn streaming_output_is_bounded() {
        let start = Instant::now();
        assert_eq!(
            SystemCommandRunner::default()
                .run("yes", &[], Duration::from_secs(2))
                .err()
                .as_deref(),
            Some("nvidia-smi output too large")
        );
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn reap_polling_stops_at_its_deadline() {
        let start = Instant::now();
        let ticks = std::cell::Cell::new(0);
        assert!(!reaped_within(
            || Ok(None),
            || {
                let tick = ticks.get();
                ticks.set(tick + 1);
                start + Duration::from_millis(tick * 100)
            },
            Duration::from_millis(200)
        ));
        assert_eq!(ticks.get(), 3);
        assert!(reaped_within(
            || Ok(Some(std::os::unix::process::ExitStatusExt::from_raw(0))),
            Instant::now,
            Duration::ZERO
        ));
    }
    #[test]
    fn output_reader_enforces_the_exact_limit() {
        use std::io::Write;
        let feed = |bytes: Vec<u8>| {
            let (reader, mut writer) = pipe();
            // A writer thread, because the pipe buffer is smaller than the input.
            let t = std::thread::spawn(move || {
                let _ = writer.write_all(&bytes);
            });
            let out = read_bounded(reader, Instant::now() + Duration::from_secs(2), MAX_STDOUT);
            let _ = t.join();
            out
        };
        assert_eq!(feed(vec![b'x'; MAX_STDOUT]).unwrap().len(), MAX_STDOUT);
        assert_eq!(
            feed(vec![b'x'; MAX_STDOUT + 1]).err().as_deref(),
            Some("nvidia-smi output too large")
        );
        assert_eq!(
            feed(vec![0xff]).err().as_deref(),
            Some("nvidia-smi output failed")
        );
    }
    #[test]
    fn background_reaper_marks_pending_until_cleanup_finishes() {
        let pending = Arc::new(AtomicBool::new(false));
        let (release, wait) = mpsc::channel();
        let thread = background_reaper(pending.clone(), move || {
            wait.recv().unwrap();
        });
        assert!(pending.load(Ordering::SeqCst));
        let mut runner = SystemCommandRunner {
            pending: pending.clone(),
        };
        assert_eq!(
            runner
                .run("/nonexistent/nvidia-smi", &[], Duration::from_secs(1))
                .err()
                .as_deref(),
            Some("previous nvidia-smi still running")
        );
        release.send(()).unwrap();
        thread.join().unwrap();
        assert!(!pending.load(Ordering::SeqCst));
        assert_eq!(
            runner
                .run("/nonexistent/nvidia-smi", &[], Duration::from_secs(1))
                .err()
                .as_deref(),
            Some("nvidia-smi not found")
        );
    }
    #[test]
    fn amd_hwmon_values_have_physical_bounds() {
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:05:00.0", "0x1002", "0x15bf", "active");
        let mut g = Gpus::new(t.path().into());
        for value in ["NaN", "inf", "-51000", "1e100", "200001"] {
            put(
                t.path(),
                "sys/bus/pci/devices/0000:05:00.0/hwmon/hwmon0/temp1_input",
                value,
            );
            assert_eq!(
                g.sample().value().unwrap()[0].temperature_celsius.reason(),
                Some("invalid value"),
                "{value}"
            );
        }
        for file in ["power1_input", "power1_average"] {
            for value in ["NaN", "inf", "-1", "1e100", "1000000001"] {
                put(
                    t.path(),
                    &format!("sys/bus/pci/devices/0000:05:00.0/hwmon/hwmon0/{file}"),
                    value,
                );
                assert_eq!(
                    g.sample().value().unwrap()[0].power_watts.reason(),
                    Some("invalid value"),
                    "{value}"
                );
            }
        }
        put(
            t.path(),
            "sys/bus/pci/devices/0000:05:00.0/hwmon/hwmon0/temp1_input",
            "-50000",
        );
        put(
            t.path(),
            "sys/bus/pci/devices/0000:05:00.0/hwmon/hwmon0/power1_average",
            "1000000000",
        );
        let reading = g.sample();
        let row = &reading.value().unwrap()[0];
        assert_eq!(row.temperature_celsius, Reading::Available(-50.0));
        assert_eq!(row.power_watts, Reading::Available(1000.0));
    }
    #[test]
    fn parses_nvidia_csv_per_field() {
        let rows = parse_csv(CSV);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows["0000:01:00.0"].memory_used_bytes,
            Reading::Available(321 * 1048576)
        );
        let partial = &rows["0000:02:00.0"];
        assert!(!partial.utilization_percent.is_available());
        assert!(!partial.memory_used_bytes.is_available());
        assert!(!partial.temperature_celsius.is_available());
        assert!(!partial.power_watts.is_available());
        assert_eq!(
            partial.memory_total_bytes,
            Reading::Available(4096 * 1048576)
        );
    }
    #[test]
    fn suspended_gpu_never_executes_command() {
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", "suspended");
        let (mut g, calls) = source(t.path(), Ok(CSV.into()));
        for status in ["suspended", "suspending"] {
            put(
                t.path(),
                "sys/bus/pci/devices/0000:01:00.0/power/runtime_status",
                status,
            );
            let reading = g.sample();
            let row = &reading.value().unwrap()[0];
            assert_eq!(row.power_state, GpuPower::Suspended);
            assert_eq!(row.power_watts.reason(), Some("gpu suspended"));
            assert_eq!(row.utilization_percent.reason(), Some("gpu suspended"));
            assert_eq!(row.memory_used_bytes.reason(), Some("gpu suspended"));
            assert_eq!(row.memory_total_bytes.reason(), Some("gpu suspended"));
            assert_eq!(row.temperature_celsius.reason(), Some("gpu suspended"));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn pci_scan_amd_and_nvidia_rate_limit() {
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", "active");
        device(t.path(), "0000:05:00.0", "0x1002", "0x15bf", "active");
        for (file, text) in [
            ("gpu_busy_percent", "32"),
            ("mem_info_vram_used", "1048576"),
            ("mem_info_vram_total", "4294967296"),
            ("hwmon/hwmon3/temp1_input", "48000"),
            ("hwmon/hwmon3/power1_input", "5500000"),
        ] {
            put(
                t.path(),
                &format!("sys/bus/pci/devices/0000:05:00.0/{file}"),
                text,
            );
        }
        put(
            t.path(),
            "usr/share/hwdata/pci.ids",
            "1002  Advanced Micro Devices, Inc.\n\t15bf  Phoenix1\n\t\t1043 0000  Subsystem\n10de  NVIDIA Corporation\n\t28e0  AD107M\n",
        );
        let (mut g, calls) = source(t.path(), Ok(CSV.into()));
        for _ in 0..2 {
            let reading = g.sample();
            let rows = reading.value().unwrap();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].vendor, GpuVendor::Nvidia);
            assert_eq!(rows[1].vendor, GpuVendor::Amd);
            assert_eq!(rows[1].name, "AMD Phoenix1");
            assert_eq!(rows[1].utilization_percent, Reading::Available(32.0));
            assert_eq!(rows[1].temperature_celsius, Reading::Available(48.0));
            assert_eq!(rows[1].power_watts, Reading::Available(5.5));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        g.queried_at = Some(Instant::now() - Duration::from_secs(2));
        g.sample();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn blank_nvidia_name_keeps_pci_fallback() {
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", "active");
        let (mut g, _) = source(
            t.path(),
            Ok("00000000:01:00.0, , 0, 0, 8188, 48, 10".into()),
        );
        assert_eq!(
            g.sample().value().unwrap()[0].name,
            "NVIDIA GPU [10de:28e0]"
        );
    }
    #[test]
    fn pci_lookup_and_missing_fields() {
        assert_eq!(
            pci_name(
                "1002  AMD\n\t15bf  Phoenix\n10de  NVIDIA\n\t15bf  Other\n",
                0x1002,
                0x15bf
            ),
            Some("Phoenix".into())
        );
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:05:00.0", "0x1002", "0x15bf", "active");
        let mut g = Gpus::new(t.path().into());
        let r = g.sample();
        let row = &r.value().unwrap()[0];
        assert_eq!(row.name, "AMD GPU [1002:15bf]");
        assert!(!row.memory_used_bytes.is_available());
    }
    #[test]
    fn failures_and_unknown_power_do_not_query() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(
            Gpus::new(t.path().into()).sample().reason(),
            Some("no display adapters found")
        );
        device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", "active");
        for reason in [
            "nvidia-smi not found",
            "nvidia-smi non-zero exit",
            "nvidia-smi timeout",
        ] {
            let (mut g, _) = source(t.path(), Err(reason.into()));
            assert_eq!(
                g.sample().value().unwrap()[0].power_watts.reason(),
                Some(reason)
            );
        }
    }
    #[test]
    fn asleep_nvidia_gpu_stops_every_nvidia_query() {
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", "active");
        device(t.path(), "0000:02:00.0", "0x10de", "0x28e0", "suspended");
        let (mut g, calls) = source(t.path(), Ok(CSV.into()));
        let reading = g.sample();
        let rows = reading.value().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(rows[0].power_state, GpuPower::Active);
        let reason = Some("not queried: another nvidia gpu is asleep");
        assert_eq!(rows[0].utilization_percent.reason(), reason);
        assert_eq!(rows[0].memory_used_bytes.reason(), reason);
        assert_eq!(rows[0].memory_total_bytes.reason(), reason);
        assert_eq!(rows[0].temperature_celsius.reason(), reason);
        assert_eq!(rows[0].power_watts.reason(), reason);
        assert_eq!(rows[1].power_state, GpuPower::Suspended);
        assert_eq!(rows[1].power_watts.reason(), Some("gpu suspended"));
    }
    #[test]
    fn nvidia_gpu_is_queried_only_when_its_runtime_pm_state_is_settled_awake() {
        let status = "sys/bus/pci/devices/0000:01:00.0/power/runtime_status";
        for (runtime, not_queried) in [
            ("active", None),
            ("unsupported", None),
            ("missing", None),
            ("resuming", Some("gpu resuming")),
            ("error", Some("gpu runtime pm error")),
            ("bogus", Some("gpu runtime pm status unrecognized")),
        ] {
            let t = tempfile::tempdir().unwrap();
            device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", runtime);
            if runtime == "missing" {
                std::fs::remove_file(t.path().join(status)).unwrap();
            }
            let (mut g, calls) = source(t.path(), Ok(CSV.into()));
            let gpus = g.sample();
            let gpu = &gpus.value().unwrap()[0];
            assert_eq!(gpu.power_state, GpuPower::Active, "{runtime}");
            match not_queried {
                None => {
                    assert_eq!(calls.load(Ordering::SeqCst), 1, "{runtime}");
                    assert_eq!(
                        gpu.temperature_celsius,
                        Reading::Available(48.0),
                        "{runtime}"
                    );
                }
                Some(reason) => {
                    assert_eq!(calls.load(Ordering::SeqCst), 0, "{runtime}");
                    assert_eq!(gpu.utilization_percent.reason(), Some(reason));
                    assert_eq!(gpu.memory_used_bytes.reason(), Some(reason));
                    assert_eq!(gpu.memory_total_bytes.reason(), Some(reason));
                    assert_eq!(gpu.temperature_celsius.reason(), Some(reason));
                    assert_eq!(gpu.power_watts.reason(), Some(reason));
                }
            }
        }
    }
    #[test]
    fn resuming_nvidia_gpu_stops_the_query_for_its_siblings() {
        let t = tempfile::tempdir().unwrap();
        device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", "active");
        device(t.path(), "0000:02:00.0", "0x10de", "0x28e0", "resuming");
        let (mut g, calls) = source(t.path(), Ok(CSV.into()));
        let reading = g.sample();
        let rows = reading.value().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            rows[0].power_watts.reason(),
            Some("not queried: another nvidia gpu is asleep")
        );
        assert_eq!(rows[1].power_watts.reason(), Some("gpu resuming"));
    }
    #[test]
    fn real_runner_handles_success_failure_missing_and_timeout() {
        let mut r = SystemCommandRunner::default();
        assert_eq!(
            r.run("echo", &["hello"], Duration::from_secs(2)).unwrap(),
            "hello\n"
        );
        assert_eq!(
            r.run("false", &[], Duration::from_secs(2)).unwrap_err(),
            "nvidia-smi non-zero exit"
        );
        assert_eq!(
            r.run("/nonexistent/nvidia-smi", &[], Duration::from_secs(2))
                .unwrap_err(),
            "nvidia-smi not found"
        );
        assert_eq!(
            r.run("sleep", &["2"], Duration::from_millis(20))
                .unwrap_err(),
            "nvidia-smi timeout"
        );
        // A descendant that keeps stdout open must not stall the sample.
        let start = Instant::now();
        assert_eq!(
            r.run(
                "sh",
                &["-c", "sleep 3 & echo hi"],
                Duration::from_millis(100)
            )
            .unwrap_err(),
            "nvidia-smi timeout"
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
