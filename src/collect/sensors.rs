use super::{Source, entries, number, read};
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
impl Source for Sensors {
    type Output = SensorStats;
    fn sample(&mut self) -> Reading<SensorStats> {
        let mut stats = SensorStats {
            temps: Vec::new(),
            fans: Vec::new(),
        };
        for dir in entries(self.root.join("sys/class/hwmon")) {
            if !dir
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("hwmon"))
            {
                continue;
            }
            let Some(chip) = read(dir.join("name")) else {
                continue;
            };
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
                let label = read(dir.join(format!("{stem}_label")))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| stem.into());
                if stem
                    .strip_prefix("temp")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                {
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
