use super::{Source, entries, entries_checked, number, read};
use crate::model::{Fan, Reading, SensorKind, SensorStats, TempSensor};
use std::path::PathBuf;
pub struct Sensors {
    root: PathBuf,
}
impl Sensors {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}
/// Whether a temperature channel's `_input` is in millidegrees Celsius, as
/// the hwmon ABI says it should be. vt1211 scales only temp2 itself: temp1
/// is a raw thermal diode reading and temp3 to temp7 are thermistor pin
/// voltages in millivolts, both needing board-specific calibration in
/// userspace (Documentation/hwmon/vt1211.rst), so they are left out rather
/// than shown as a wrong temperature.
fn reports_celsius(chip: &str, stem: &str) -> bool {
    chip != "vt1211" || stem == "temp2"
}

impl Source for Sensors {
    type Output = SensorStats;
    fn sample(&mut self) -> Reading<SensorStats> {
        let mut stats = SensorStats {
            temps: Vec::new(),
            fans: Vec::new(),
        };
        let chips = match entries_checked(self.root.join("sys/class/hwmon")) {
            Ok(chips) => chips,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Reading::unavailable("hwmon sensors not readable"),
        };
        for dir in chips {
            if !dir
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("hwmon"))
            {
                continue;
            }
            let Some(chip) = read(dir.join("name")) else {
                continue;
            };
            // Reading a runtime-suspended device's inputs (an AMD or nouveau
            // dGPU, for example) can resume it; leave sleeping devices alone.
            if matches!(
                read(dir.join("device/power/runtime_status")).as_deref(),
                Some("suspended" | "suspending")
            ) {
                continue;
            }
            let kind = match chip.as_str() {
                "k10temp" | "coretemp" | "zenpower" => SensorKind::Cpu,
                "amdgpu" | "radeon" | "nouveau" => SensorKind::Gpu,
                "nvme" | "drivetemp" => SensorKind::Storage,
                _ => SensorKind::Other,
            };
            for path in entries(&dir) {
                let Some(stem) = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix("_input"))
                else {
                    continue;
                };
                // The hwmon ABI says not to trust a channel whose fault flag is set.
                if read(dir.join(format!("{stem}_fault"))).as_deref() == Some("1") {
                    continue;
                }
                let label = read(dir.join(format!("{stem}_label")))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| stem.into());
                if stem
                    .strip_prefix("temp")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                {
                    if !reports_celsius(&chip, stem) {
                        continue;
                    }
                    if let Some(value) = number::<i64>(&path) {
                        stats.temps.push(TempSensor {
                            chip: chip.clone(),
                            label,
                            kind,
                            celsius: value as f64 / 1000.0,
                            crit_celsius: number::<i64>(dir.join(format!("{stem}_crit")))
                                .map(|v| v as f64 / 1000.0),
                        });
                    }
                } else if stem
                    .strip_prefix("fan")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                    && let Some(rpm) = number(&path)
                {
                    stats.fans.push(Fan {
                        chip: chip.clone(),
                        label,
                        rpm,
                    });
                }
            }
        }
        if stats.temps.is_empty() && stats.fans.is_empty() {
            Reading::unavailable("no hwmon sensors")
        } else {
            Reading::Available(stats)
        }
    }
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn put(root: &std::path::Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    /// An absent or empty hwmon class has no sensors; a class that exists
    /// but cannot be listed may have some, and says so.
    #[test]
    fn an_unlistable_hwmon_class_is_not_reported_as_absent() {
        let t = tempfile::tempdir().unwrap();
        let mut sensors = Sensors::new(t.path().into());
        assert_eq!(sensors.sample().reason(), Some("no hwmon sensors"));
        std::fs::create_dir_all(t.path().join("sys/class/hwmon")).unwrap();
        assert_eq!(sensors.sample().reason(), Some("no hwmon sensors"));
        std::fs::remove_dir(t.path().join("sys/class/hwmon")).unwrap();
        // A regular file fails to list for every user, like a permission error.
        std::fs::write(t.path().join("sys/class/hwmon"), "").unwrap();
        assert_eq!(
            sensors.sample().reason(),
            Some("hwmon sensors not readable")
        );
    }

    /// vt1211 reports only temp2 in millidegrees Celsius; temp1 is a raw
    /// thermal diode reading and temp3 to temp7 are thermistor pin voltages
    /// in millivolts, which need board-specific calibration in userspace
    /// (Documentation/hwmon/vt1211.rst), so they are not shown as Celsius.
    #[test]
    fn vt1211_channels_without_celsius_scaling_are_left_out() {
        let t = tempfile::tempdir().unwrap();
        put(t.path(), "sys/class/hwmon/hwmon0/name", "vt1211");
        put(t.path(), "sys/class/hwmon/hwmon0/temp1_input", "1200");
        put(t.path(), "sys/class/hwmon/hwmon0/temp2_input", "41000");
        put(t.path(), "sys/class/hwmon/hwmon0/temp3_input", "900");
        put(t.path(), "sys/class/hwmon/hwmon0/temp7_input", "1500");
        put(t.path(), "sys/class/hwmon/hwmon1/name", "nct6775");
        put(t.path(), "sys/class/hwmon/hwmon1/temp1_input", "35000");
        let s = Sensors::new(t.path().into()).sample();
        let temps: Vec<(&str, &str, f64)> = s
            .value()
            .unwrap()
            .temps
            .iter()
            .map(|t| (t.chip.as_str(), t.label.as_str(), t.celsius))
            .collect();
        assert_eq!(
            temps,
            [("vt1211", "temp2", 41.0), ("nct6775", "temp1", 35.0)]
        );
    }

    #[test]
    fn faulted_channels_are_skipped() {
        let t = tempfile::tempdir().unwrap();
        put(t.path(), "sys/class/hwmon/hwmon0/name", "nct6775");
        put(t.path(), "sys/class/hwmon/hwmon0/temp1_input", "127000");
        put(t.path(), "sys/class/hwmon/hwmon0/temp1_fault", "1");
        put(t.path(), "sys/class/hwmon/hwmon0/temp2_input", "41000");
        put(t.path(), "sys/class/hwmon/hwmon0/temp2_fault", "0");
        put(t.path(), "sys/class/hwmon/hwmon0/fan1_input", "0");
        put(t.path(), "sys/class/hwmon/hwmon0/fan1_fault", "1");
        put(t.path(), "sys/class/hwmon/hwmon0/fan2_input", "900");
        let s = Sensors::new(t.path().into()).sample();
        let s = s.value().unwrap();
        assert_eq!(s.temps.len(), 1);
        assert_eq!(s.temps[0].label, "temp2");
        assert_eq!(s.fans.len(), 1);
        assert_eq!(s.fans[0].rpm, 900);
    }
    #[test]
    fn suspended_devices_are_not_read() {
        // Reading a suspended dGPU's hwmon inputs can resume it.
        let t = tempfile::tempdir().unwrap();
        put(t.path(), "sys/class/hwmon/hwmon0/name", "amdgpu");
        put(t.path(), "sys/class/hwmon/hwmon0/temp1_input", "45000");
        put(
            t.path(),
            "sys/class/hwmon/hwmon0/device/power/runtime_status",
            "suspended",
        );
        put(t.path(), "sys/class/hwmon/hwmon1/name", "k10temp");
        put(t.path(), "sys/class/hwmon/hwmon1/temp1_input", "50000");
        put(
            t.path(),
            "sys/class/hwmon/hwmon1/device/power/runtime_status",
            "active",
        );
        let s = Sensors::new(t.path().into()).sample();
        let temps = &s.value().unwrap().temps;
        assert_eq!(temps.len(), 1, "{temps:?}");
        assert_eq!(temps[0].chip, "k10temp");
    }
    #[test]
    fn fans_are_reported_without_any_temperature() {
        let t = tempfile::tempdir().unwrap();
        put(t.path(), "sys/class/hwmon/hwmon0/name", "asus");
        put(t.path(), "sys/class/hwmon/hwmon0/fan1_input", "2200");
        let s = Sensors::new(t.path().into()).sample();
        let s = s.value().expect("fans alone are data");
        assert!(s.temps.is_empty());
        assert_eq!(s.fans[0].rpm, 2200);
    }
    #[test]
    fn laptop_hwmon_tree_skips_unreadable_inputs() {
        let t = tempfile::tempdir().unwrap();
        for (index, chip, label, temp) in [
            (0, "k10temp", "Tctl", "62375"),
            (1, "amdgpu", "edge", "48000"),
            (2, "nvme", "Composite", "43850"),
            (3, "asus", "cpu", "45000"),
        ] {
            let base = format!("sys/class/hwmon/hwmon{index}");
            for (file, text) in [
                ("name", chip),
                ("temp1_input", temp),
                ("temp1_label", label),
                ("temp2_input", "Input/output error"),
            ] {
                put(t.path(), &format!("{base}/{file}"), text);
            }
        }
        put(t.path(), "sys/class/hwmon/hwmon2/temp1_crit", "84850");
        put(t.path(), "sys/class/hwmon/hwmon3/fan1_input", "2200");
        put(t.path(), "sys/class/hwmon/hwmon3/fan1_label", "cpu_fan");
        put(t.path(), "sys/class/hwmon/hwmon3/fan2_input", "1900");
        let s = Sensors::new(t.path().into())
            .sample()
            .value()
            .unwrap()
            .clone();
        assert_eq!(s.temps.len(), 4);
        assert_eq!(s.temps[0].celsius, 62.375);
        assert_eq!(s.temps[0].kind, SensorKind::Cpu);
        assert_eq!(s.temps[1].kind, SensorKind::Gpu);
        assert_eq!(s.temps[2].crit_celsius, Some(84.85));
        assert_eq!(s.fans[0].label, "cpu_fan");
        assert_eq!(s.fans[1].label, "fan2");
    }
    #[test]
    fn absent_sensors() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(
            Sensors::new(t.path().into()).sample().reason(),
            Some("no hwmon sensors")
        );
    }
}
