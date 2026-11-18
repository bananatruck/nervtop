use super::{Source, entries, number, read};
use crate::model::{GpuPower, GpuStats, GpuVendor, Reading};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// External command boundary, replaceable without executing a GPU query in tests.
pub trait CommandRunner: Send {
    fn run(&mut self, program: &str, args: &[&str], timeout: Duration) -> Result<String, String>;
}
pub struct SystemCommandRunner;

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
    fn run(&mut self, program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
        let mut child = Command::new(program)
            .args(args)
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
        let mut stdout = child.stdout.take().ok_or("nvidia-smi stdout unavailable")?;
        // Drain concurrently so even unexpectedly large output cannot fill the pipe.
        // The text comes back over a channel so collecting it is bounded too.
        let (text_tx, text_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = text_tx.send(stdout.read_to_string(&mut text).map(|_| text));
        });
        let start = Instant::now();
        let result: Result<(), String> = loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    break if status.success() {
                        Ok(())
                    } else {
                        Err("nvidia-smi non-zero exit".into())
                    };
                }
                Ok(None) if start.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                outcome => {
                    let _ = child.kill();
                    if !reaped_within(|| child.try_wait(), Instant::now, KILL_GRACE) {
                        // Still not reaped (uninterruptible sleep): reap it in the
                        // background so this sample is not blocked by it.
                        std::thread::spawn(move || {
                            let _ = child.wait();
                            let _ = reader.join();
                        });
                    }
                    break Err(if outcome.is_ok() {
                        "nvidia-smi timeout".into()
                    } else {
                        "nvidia-smi wait failed".into()
                    });
                }
            }
        };
        result?;
        // The child has exited, so its stdout normally closes at once; a
        // descendant still holding the pipe must not stall the sample either.
        text_rx
            .recv_timeout((start + timeout + KILL_GRACE).saturating_duration_since(Instant::now()))
            .map_err(|_| "nvidia-smi output failed".to_owned())?
            .map_err(|_| "nvidia-smi output failed".to_owned())
    }
}

pub struct Gpus {
    root: PathBuf,
    runner: Box<dyn CommandRunner>,
    names: Option<String>,
    name_cache: BTreeMap<(u16, u16), Option<String>>,
    cached: Result<BTreeMap<String, GpuStats>, String>,
    queried_at: Option<Instant>,
}
impl Gpus {
    pub fn new(root: PathBuf) -> Self {
        Self::with_runner(root, Box::new(SystemCommandRunner))
    }
    pub fn with_runner(root: PathBuf, runner: Box<dyn CommandRunner>) -> Self {
        // Only retain the first readable database; do not probe it every tick.
        let names = ["usr/share/hwdata/pci.ids", "usr/share/misc/pci.ids"]
            .into_iter()
            .find_map(|p| read(root.join(p)));
        Self {
            root,
            runner,
            names,
            name_cache: BTreeMap::new(),
            cached: Err("warming up".into()),
            queried_at: None,
        }
    }
    fn query(&mut self) {
        if self
            .queried_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
        {
            return;
        }
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
fn csv_number(text: &str) -> Reading<f64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(Reading::Available)
        .unwrap_or_else(|| Reading::unavailable("not reported"))
}
fn parse_csv(text: &str) -> BTreeMap<String, GpuStats> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            if fields.len() != 7 {
                return None;
            }
            let slot = normalize_slot(fields[0])?;
            Some((
                slot.clone(),
                GpuStats {
                    pci_slot: slot,
                    vendor: GpuVendor::Nvidia,
                    name: fields[1].into(),
                    power_state: GpuPower::Active,
                    utilization_percent: csv_number(fields[2]).map(|v| v as f32),
                    memory_used_bytes: csv_number(fields[3]).map(|v| (v * 1048576.0) as u64),
                    memory_total_bytes: csv_number(fields[4]).map(|v| (v * 1048576.0) as u64),
                    temperature_celsius: csv_number(fields[5]),
                    power_watts: csv_number(fields[6]),
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
impl Source for Gpus {
    type Output = Vec<GpuStats>;
    fn sample(&mut self) -> Reading<Self::Output> {
        let mut out = Vec::new();
        for dir in entries(self.root.join("sys/bus/pci/devices")) {
            if !read(dir.join("class")).is_some_and(|c| c.starts_with("0x03")) {
                continue;
            }
            let hex = |file: &str| {
                read(dir.join(file))
                    .and_then(|v| u16::from_str_radix(v.trim_start_matches("0x"), 16).ok())
            };
            let vendor_id = hex("vendor");
            let device_id = hex("device");
            let (vendor, label) = match vendor_id {
                Some(0x1002) => (GpuVendor::Amd, "AMD"),
                Some(0x10de) => (GpuVendor::Nvidia, "NVIDIA"),
                Some(0x8086) => (GpuVendor::Intel, "Intel"),
                _ => (GpuVendor::Other, "Unknown"),
            };
            let name = vendor_id
                .zip(device_id)
                .and_then(|(v, d)| {
                    self.name_cache
                        .entry((v, d))
                        .or_insert_with(|| {
                            self.names
                                .as_deref()
                                .and_then(|text| pci_name(text, v, d))
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
            let runtime = read(dir.join("power/runtime_status"));
            if matches!(runtime.as_deref(), Some("suspended" | "suspending")) {
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
                // Only a suspended device is left alone. "active", "resuming",
                // "unsupported" (runtime PM disabled, so always powered) and a
                // missing status file all mean the GPU is already awake.
                GpuVendor::Nvidia => {
                    self.query();
                    match &self.cached {
                        Ok(rows) => {
                            if let Some(row) =
                                normalize_slot(&slot).and_then(|slot| rows.get(&slot))
                            {
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
                    gpu.utilization_percent = sys_number(&dir, "gpu_busy_percent");
                    gpu.memory_used_bytes = sys_number(&dir, "mem_info_vram_used");
                    gpu.memory_total_bytes = sys_number(&dir, "mem_info_vram_total");
                    gpu.temperature_celsius = Reading::unavailable("gpu temperature not reported");
                    gpu.power_watts = Reading::unavailable("gpu power not reported");
                    for hwmon in entries(dir.join("hwmon")) {
                        if let Some(v) = number::<i64>(hwmon.join("temp1_input")) {
                            gpu.temperature_celsius = Reading::Available(v as f64 / 1000.0);
                        }
                        if let Some(v) = number::<u64>(hwmon.join("power1_average"))
                            .or_else(|| number(hwmon.join("power1_input")))
                        {
                            gpu.power_watts = Reading::Available(v as f64 / 1e6);
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
    fn awake_nvidia_gpu_is_queried_whatever_its_runtime_pm_state() {
        let status = "sys/bus/pci/devices/0000:01:00.0/power/runtime_status";
        for runtime in ["active", "resuming", "unsupported", "missing"] {
            let t = tempfile::tempdir().unwrap();
            device(t.path(), "0000:01:00.0", "0x10de", "0x28e0", runtime);
            if runtime == "missing" {
                std::fs::remove_file(t.path().join(status)).unwrap();
            }
            let (mut g, calls) = source(t.path(), Ok(CSV.into()));
            let gpus = g.sample();
            assert_eq!(calls.load(Ordering::SeqCst), 1, "{runtime}");
            let gpu = &gpus.value().unwrap()[0];
            assert_eq!(gpu.power_state, GpuPower::Active, "{runtime}");
            assert_eq!(
                gpu.temperature_celsius,
                Reading::Available(48.0),
                "{runtime}"
            );
        }
    }
    #[test]
    fn reaping_a_child_that_never_exits_is_bounded() {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let polls = AtomicUsize::new(0);
            let start = Instant::now();
            let exited = reaped_within(
                || {
                    polls.fetch_add(1, Ordering::SeqCst);
                    Ok(None)
                },
                Instant::now,
                Duration::from_millis(200),
            );
            let _ = done.send((exited, start.elapsed(), polls.load(Ordering::SeqCst)));
        });
        let (exited, elapsed, polls) = finished
            .recv_timeout(Duration::from_secs(2))
            .expect("reaping a stuck child must not block");
        assert!(!exited);
        assert!(elapsed >= Duration::from_millis(200), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(400), "{elapsed:?}");
        assert!(polls > 1);
    }
    #[test]
    fn real_runner_handles_success_failure_missing_and_timeout() {
        let mut r = SystemCommandRunner;
        assert_eq!(
            r.run("/bin/echo", &["hello"], Duration::from_secs(2))
                .unwrap(),
            "hello\n"
        );
        assert_eq!(
            r.run("/bin/false", &[], Duration::from_secs(2))
                .unwrap_err(),
            "nvidia-smi non-zero exit"
        );
        assert_eq!(
            r.run("/nonexistent/nvidia-smi", &[], Duration::from_secs(2))
                .unwrap_err(),
            "nvidia-smi not found"
        );
        assert_eq!(
            r.run("/bin/sleep", &["2"], Duration::from_millis(20))
                .unwrap_err(),
            "nvidia-smi timeout"
        );
        // A descendant that keeps stdout open must not stall the sample.
        let start = Instant::now();
        assert_eq!(
            r.run(
                "/bin/sh",
                &["-c", "sleep 3 & echo hi"],
                Duration::from_millis(100)
            )
            .unwrap_err(),
            "nvidia-smi output failed"
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
