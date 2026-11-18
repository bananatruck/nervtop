use super::{Source, entries, number, read};
use crate::model::{BatteryStats, BatteryStatus, Reading};
use std::path::PathBuf;
pub struct Battery {
    root: PathBuf,
}
impl Battery {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}
impl Source for Battery {
    type Output = BatteryStats;
    fn sample(&mut self) -> Reading<BatteryStats> {
        let supplies = entries(self.root.join("sys/class/power_supply"));
        let mains: Vec<_> = supplies
            .iter()
            .filter(|d| read(d.join("type")).as_deref() == Some("Mains"))
            .filter_map(|d| number::<u8>(d.join("online")))
            .collect();
        let ac_online = (!mains.is_empty()).then(|| mains.iter().any(|v| *v != 0));
        let Some(dir) = supplies.iter().find(|d| {
            read(d.join("type")).as_deref() == Some("Battery")
                && number::<u8>(d.join("present")) != Some(0)
                // Mice, keyboards and headsets report scope "Device".
                && read(d.join("scope")).as_deref() != Some("Device")
        }) else {
            return Reading::unavailable("no battery present");
        };
        // Some drivers report current_now and power_now as negative while
        // discharging; the model carries magnitudes.
        let n = |name: &str| number::<i64>(dir.join(name)).map(|v| (v as f64).abs());
        let status = match read(dir.join("status")).as_deref() {
            Some("Charging") => BatteryStatus::Charging,
            Some("Discharging") => BatteryStatus::Discharging,
            Some("Full") => BatteryStatus::Full,
            Some("Not charging") => BatteryStatus::NotCharging,
            _ => BatteryStatus::Unknown,
        };
        let power = n("power_now")
            .map(|v| v / 1e6)
            .or_else(|| Some(n("current_now")? * n("voltage_now")? / 1e12));
        // Keep capacity, design capacity and rate in the same native units.
        let (now, full, design, rate) = if n("energy_now").is_some() {
            (
                n("energy_now"),
                n("energy_full"),
                n("energy_full_design"),
                power.map(|w| w * 1e6),
            )
        } else {
            (
                n("charge_now"),
                n("charge_full"),
                n("charge_full_design"),
                n("current_now"),
            )
        };
        let Some(percent) =
            n("capacity").or_else(|| Some(now? / full.filter(|v| *v > 0.0)? * 100.0))
        else {
            return Reading::unavailable("battery capacity not reported");
        };
        let remaining = match status {
            BatteryStatus::Charging => now.zip(full).map(|(now, full)| (full - now).max(0.0)),
            BatteryStatus::Discharging => now,
            BatteryStatus::Full | BatteryStatus::NotCharging | BatteryStatus::Unknown => None,
        };
        let time_remaining_secs =
            if !matches!(status, BatteryStatus::Charging | BatteryStatus::Discharging) {
                Reading::unavailable("not charging or discharging")
            } else if let Some((remaining, rate)) = remaining.zip(rate.filter(|r| *r > 0.0)) {
                Reading::Available((remaining / rate * 3600.0) as u64)
            } else {
                Reading::unavailable("rate not reported")
            };
        Reading::Available(BatteryStats {
            name: dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            percent,
            status,
            ac_online,
            power_watts: power
                .map(Reading::Available)
                .unwrap_or_else(|| Reading::unavailable("rate not reported")),
            time_remaining_secs,
            health_percent: full
                .zip(design.filter(|v| *v > 0.0))
                .map(|(f, d)| Reading::Available(f / d * 100.0))
                .unwrap_or_else(|| Reading::unavailable("design capacity not reported")),
            charge_limit_percent: number(dir.join("charge_control_end_threshold")),
            cycle_count: number::<u64>(dir.join("cycle_count")).filter(|v| *v > 0),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn tree(values: &[(&str, &str)]) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        for (file, text) in values {
            super::super::sensors::tests::put(
                t.path(),
                &format!("sys/class/power_supply/BAT1/{file}"),
                text,
            );
        }
        super::super::sensors::tests::put(t.path(), "sys/class/power_supply/BAT1/type", "Battery");
        t
    }
    #[test]
    fn peripheral_batteries_are_not_the_system_battery() {
        let t = tempfile::tempdir().unwrap();
        let put = super::super::sensors::tests::put;
        put(
            t.path(),
            "sys/class/power_supply/hidpp_battery_0/type",
            "Battery",
        );
        put(
            t.path(),
            "sys/class/power_supply/hidpp_battery_0/scope",
            "Device",
        );
        put(
            t.path(),
            "sys/class/power_supply/hidpp_battery_0/capacity",
            "40",
        );
        let mut b = Battery::new(t.path().into());
        assert_eq!(b.sample().reason(), Some("no battery present"));
        put(t.path(), "sys/class/power_supply/BAT0/type", "Battery");
        put(t.path(), "sys/class/power_supply/BAT0/capacity", "77");
        assert_eq!(b.sample().value().unwrap().name, "BAT0");
    }
    #[test]
    fn target_charge_units() {
        let t = tree(&[
            ("capacity", "95"),
            ("status", "Charging"),
            ("charge_now", "3348000"),
            ("charge_full", "3509000"),
            ("charge_full_design", "4580000"),
            ("current_now", "512000"),
            ("voltage_now", "17144000"),
            ("charge_control_end_threshold", "100"),
            ("cycle_count", "0"),
        ]);
        let result = Battery::new(t.path().into()).sample();
        let b = result.value().unwrap();
        assert_eq!(b.percent, 95.0);
        assert_eq!(b.status, BatteryStatus::Charging);
        assert_eq!(b.cycle_count, None);
        assert_eq!(b.charge_limit_percent, Some(100));
        assert_eq!(b.time_remaining_secs, Reading::Available(1132));
        assert!((b.power_watts.value().unwrap() - 8.777728).abs() < 1e-6);
        assert!((b.health_percent.value().unwrap() - 3509.0 / 4580.0 * 100.0).abs() < 1e-6);
    }
    #[test]
    fn energy_units_and_mains() {
        let t = tree(&[
            ("status", "Discharging"),
            ("energy_now", "30000000"),
            ("energy_full", "60000000"),
            ("energy_full_design", "65000000"),
            ("power_now", "15000000"),
        ]);
        super::super::sensors::tests::put(t.path(), "sys/class/power_supply/AC0/type", "Mains");
        super::super::sensors::tests::put(t.path(), "sys/class/power_supply/AC0/online", "0");
        let result = Battery::new(t.path().into()).sample();
        let b = result.value().unwrap();
        assert_eq!(b.percent, 50.0);
        assert_eq!(b.time_remaining_secs, Reading::Available(7200));
        assert_eq!(b.ac_online, Some(false));
        assert_eq!(b.power_watts, Reading::Available(15.0));
    }
    #[test]
    fn discharge_time_does_not_need_full_capacity() {
        let t = tree(&[
            ("status", "Discharging"),
            ("capacity", "50"),
            ("energy_now", "30000000"),
            ("power_now", "15000000"),
        ]);
        let reading = Battery::new(t.path().into()).sample();
        assert_eq!(
            reading.value().unwrap().time_remaining_secs,
            Reading::Available(7200)
        );
    }
    #[test]
    fn negative_discharge_current_is_a_magnitude() {
        let t = tree(&[
            ("status", "Discharging"),
            ("capacity", "50"),
            ("charge_now", "2000000"),
            ("charge_full", "4000000"),
            ("current_now", "-1000000"),
            ("voltage_now", "15000000"),
        ]);
        let b = Battery::new(t.path().into()).sample();
        let b = b.value().unwrap();
        assert_eq!(b.power_watts, Reading::Available(15.0));
        assert_eq!(b.time_remaining_secs, Reading::Available(7200));
    }
    #[test]
    fn missing_rate_and_full_status() {
        let t = tree(&[("status", "Charging"), ("capacity", "80")]);
        let mut b = Battery::new(t.path().into());
        assert_eq!(
            b.sample().value().unwrap().time_remaining_secs.reason(),
            Some("rate not reported")
        );
        super::super::sensors::tests::put(t.path(), "sys/class/power_supply/BAT1/status", "Full");
        assert_eq!(
            b.sample().value().unwrap().time_remaining_secs.reason(),
            Some("not charging or discharging")
        );
        super::super::sensors::tests::put(t.path(), "sys/class/power_supply/BAT1/present", "0");
        assert_eq!(b.sample().reason(), Some("no battery present"));
    }
}
