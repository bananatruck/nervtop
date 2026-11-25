use super::{Source, entries_checked, number, read};
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
        let supplies = match entries_checked(self.root.join("sys/class/power_supply")) {
            Ok(supplies) => supplies,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Reading::unavailable("power supply list not readable"),
        };
        // Online if any mains supply says so; offline only if there is a
        // mains supply, every one was read and says 0, and every supply's
        // type could be read (an unreadable one may be a mains supply that is
        // online); otherwise unknown.
        let mut mains: Vec<Option<u8>> = Vec::new();
        let mut unclassified = false;
        for supply in &supplies {
            match read(supply.join("type")).as_deref() {
                Some("Mains") => mains.push(number::<u8>(supply.join("online"))),
                Some(_) => {}
                None => unclassified = true,
            }
        }
        let ac_online = if mains.iter().any(|v| matches!(v, Some(n) if *n != 0)) {
            Some(true)
        } else if !mains.is_empty() && !unclassified && mains.iter().all(|v| *v == Some(0)) {
            Some(false)
        } else {
            None
        };
        let Some(dir) = supplies.iter().find(|d| {
            read(d.join("type")).as_deref() == Some("Battery")
                && number::<u8>(d.join("present")) != Some(0)
                // Mice, keyboards and headsets report scope "Device".
                && read(d.join("scope")).as_deref() != Some("Device")
        }) else {
            return Reading::unavailable("no battery present");
        };
        // Stored energy and charge are never negative, so they parse as
        // unsigned and a negative value is "not reported". Only the rates
        // (current_now, power_now) are signed on some drivers while
        // discharging; the model carries their magnitude.
        let n = |name: &str| number::<u64>(dir.join(name)).map(|v| v as f64);
        let rate_of = |name: &str| number::<i64>(dir.join(name)).map(|v| (v as f64).abs());
        let status = match read(dir.join("status")).as_deref() {
            Some("Charging") => BatteryStatus::Charging,
            Some("Discharging") => BatteryStatus::Discharging,
            Some("Full") => BatteryStatus::Full,
            Some("Not charging") => BatteryStatus::NotCharging,
            _ => BatteryStatus::Unknown,
        };
        let power = rate_of("power_now")
            .map(|v| v / 1e6)
            .or_else(|| Some(rate_of("current_now")? * n("voltage_now")? / 1e12));
        // Every figure uses one complete pair in one native unit: energy
        // (µWh, with power in µW) when it has what the figure needs, else
        // charge (µAh, with current in µA). Units are never mixed.
        struct Units {
            now: Option<f64>,
            full: Option<f64>,
            design: Option<f64>,
            rate: Option<f64>,
        }
        let units = [
            Units {
                now: n("energy_now"),
                full: n("energy_full"),
                design: n("energy_full_design"),
                rate: power.map(|w| w * 1e6),
            },
            Units {
                now: n("charge_now"),
                full: n("charge_full"),
                design: n("charge_full_design"),
                rate: rate_of("current_now"),
            },
        ];
        let Some(percent) = n("capacity").filter(|c| *c <= 100.0).or_else(|| {
            units
                .iter()
                .find_map(|u| Some((u.now? / u.full.filter(|v| *v > 0.0)? * 100.0).min(100.0)))
        }) else {
            return Reading::unavailable("battery capacity not reported");
        };
        let hours = |u: &Units| -> Option<f64> {
            let rate = u.rate.filter(|r| *r > 0.0)?;
            match status {
                BatteryStatus::Charging => Some((u.full? - u.now?).max(0.0) / rate),
                BatteryStatus::Discharging => Some(u.now? / rate),
                _ => None,
            }
        };
        let time_remaining_secs =
            if !matches!(status, BatteryStatus::Charging | BatteryStatus::Discharging) {
                Reading::unavailable("not charging or discharging")
            } else if let Some(hours) = units.iter().find_map(hours) {
                Reading::Available((hours * 3600.0) as u64)
            } else {
                Reading::unavailable("rate not reported")
            };
        let health = units
            .iter()
            .find_map(|u| Some(u.full? / u.design.filter(|v| *v > 0.0)? * 100.0));
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
            health_percent: health
                .map(Reading::Available)
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
    fn capacity_must_be_a_real_percentage() {
        // A negative capacity must not become a healthy positive one.
        let t = tree(&[("status", "Discharging"), ("capacity", "-95")]);
        let b = Battery::new(t.path().into()).sample();
        assert_eq!(b.reason(), Some("battery capacity not reported"));
        // Out of range: fall back to charge_now / charge_full.
        let t = tree(&[
            ("status", "Discharging"),
            ("capacity", "101"),
            ("charge_now", "1000"),
            ("charge_full", "4000"),
        ]);
        let b = Battery::new(t.path().into()).sample();
        assert_eq!(b.value().unwrap().percent, 25.0);
        // Negative stored energy is not a magnitude either.
        let t = tree(&[
            ("status", "Discharging"),
            ("energy_now", "-5"),
            ("energy_full", "10"),
        ]);
        assert!(Battery::new(t.path().into()).sample().reason().is_some());
    }
    /// A regular file where the class directory belongs fails to list with
    /// NotADirectory for every user, privileged or not, which is the same
    /// "exists but cannot be listed" case as a permission error.
    #[test]
    fn unreadable_power_supply_list_is_not_no_battery() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("sys/class")).unwrap();
        std::fs::write(t.path().join("sys/class/power_supply"), "").unwrap();
        let r = Battery::new(t.path().into()).sample();
        assert_eq!(r.reason(), Some("power supply list not readable"));
    }
    #[test]
    fn a_supply_whose_type_cannot_be_read_keeps_ac_unknown() {
        let t = tree(&[("status", "Unknown"), ("capacity", "8")]);
        let put = super::super::sensors::tests::put;
        put(t.path(), "sys/class/power_supply/AC0/type", "Mains");
        put(t.path(), "sys/class/power_supply/AC0/online", "0");
        // AC1 exists but its type cannot be read.
        put(t.path(), "sys/class/power_supply/AC1/online", "1");
        let b = Battery::new(t.path().into()).sample();
        assert_eq!(b.value().unwrap().ac_online, None);
        // A known online mains supply is still online.
        put(t.path(), "sys/class/power_supply/AC0/online", "1");
        let b = Battery::new(t.path().into()).sample();
        assert_eq!(b.value().unwrap().ac_online, Some(true));
    }
    /// Each figure uses one complete pair in one unit: energy, else charge.
    #[test]
    fn each_figure_uses_a_complete_energy_or_charge_pair() {
        // energy_now alone, with a complete charge pair: percent and health
        // come from charge.
        let t = tree(&[
            ("status", "Unknown"),
            ("energy_now", "30000000"),
            ("charge_now", "2000000"),
            ("charge_full", "4000000"),
            ("charge_full_design", "5000000"),
        ]);
        let reading = Battery::new(t.path().into()).sample();
        let b = reading.value().unwrap();
        assert_eq!(b.percent, 50.0);
        assert_eq!(b.health_percent, Reading::Available(80.0));
        // Charging with energy_now and power but no energy_full: the time to
        // full comes from the complete charge pair and current.
        let t = tree(&[
            ("status", "Charging"),
            ("capacity", "50"),
            ("energy_now", "30000000"),
            ("power_now", "15000000"),
            ("charge_now", "2000000"),
            ("charge_full", "4000000"),
            ("current_now", "1000000"),
        ]);
        let reading = Battery::new(t.path().into()).sample();
        assert_eq!(
            reading.value().unwrap().time_remaining_secs,
            Reading::Available(7200)
        );
    }
    #[test]
    fn an_unreadable_mains_supply_makes_ac_unknown_not_offline() {
        let t = tree(&[("status", "Unknown"), ("capacity", "8")]);
        let put = super::super::sensors::tests::put;
        put(t.path(), "sys/class/power_supply/AC0/type", "Mains");
        put(t.path(), "sys/class/power_supply/AC0/online", "0");
        put(t.path(), "sys/class/power_supply/AC1/type", "Mains");
        put(t.path(), "sys/class/power_supply/AC1/online", "garbage");
        let b = Battery::new(t.path().into()).sample();
        assert_eq!(b.value().unwrap().ac_online, None);
        put(t.path(), "sys/class/power_supply/AC1/online", "1");
        let b = Battery::new(t.path().into()).sample();
        assert_eq!(b.value().unwrap().ac_online, Some(true));
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
