//! Local wall-clock time without a date-time dependency.

unsafe extern "C" {
    fn tzset();
}

/// The local UTC offset in seconds at `epoch_secs`, from the C library's
/// time zone database (honours `TZ` and `/etc/localtime`).
pub fn local_offset_secs(epoch_secs: i64) -> Option<i32> {
    let t = libc::time_t::try_from(epoch_secs).ok()?;
    // SAFETY: `tm` is plain old data that `localtime_r` fully initialises on
    // success; on failure it returns null and we do not read `tm`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // localtime_r need not re-read the time zone; tzset makes a change to
    // /etc/localtime (for example after travelling) show up while running.
    // SAFETY: tzset takes no arguments and only updates libc's zone state.
    unsafe { tzset() };
    let result = unsafe { libc::localtime_r(&t, &mut tm) };
    if result.is_null() {
        None
    } else {
        i32::try_from(tm.tm_gmtoff).ok()
    }
}

/// `HH:MM:SS` for `epoch_ms` shifted by `offset_secs`.
pub fn hms(epoch_ms: u64, offset_secs: i32) -> String {
    let secs = (epoch_ms / 1000) as i64 + i64::from(offset_secs);
    let day = secs.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day / 3600, (day % 3600) / 60, day % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_with_offsets_across_midnight() {
        // 2026-11-18T07:55:00Z
        let ms = 1_795_420_500_000;
        assert_eq!(hms(ms, 0), "07:55:00");
        assert_eq!(hms(ms, -8 * 3600), "23:55:00");
        assert_eq!(hms(ms, 5 * 3600 + 1800), "13:25:00");
    }

    /// Set in a child test process to the offset it must report.
    const EXPECTED: &str = "NERVTOP_CLOCK_EXPECTED_OFFSET";

    /// The offset follows `TZ` exactly, including zones beyond the real-world
    /// 14 h that glibc accepts. Each zone is checked in a child process with
    /// its own `TZ`, so neither the inherited `TZ` nor a change to it can
    /// affect other tests running in parallel.
    #[test]
    fn offset_follows_the_time_zone() {
        // 2026-11-18T07:55:00Z: outside daylight saving time in the zones
        // below, and inside the rule of the last one.
        let at = 1_795_420_500;
        if let Ok(expected) = std::env::var(EXPECTED) {
            assert_eq!(local_offset_secs(at), Some(expected.parse().unwrap()));
            return;
        }
        for (tz, expected) in [
            ("UTC0", 0),
            ("EST5", -5 * 3600),
            ("IST-5:30", 5 * 3600 + 1800),
            ("ABC-15", 15 * 3600),
            ("STD-24:30DST,M1.1.0,M12.5.0", 25 * 3600 + 1800),
        ] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "clock::tests::offset_follows_the_time_zone",
                    "--quiet",
                ])
                .env("TZ", tz)
                .env(EXPECTED, expected.to_string())
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "TZ={tz}: expected offset {expected}");
        }
    }
}
