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

    #[test]
    fn offset_is_within_a_day() {
        let off = local_offset_secs(1_795_420_500).expect("libc reports an offset");
        // glibc accepts TZ offsets beyond the real-world 14 h (TZ=ABC-15),
        // so only the C library's own limit is a safe bound here.
        assert!(off.abs() < 25 * 3600, "{off}");
    }
}
