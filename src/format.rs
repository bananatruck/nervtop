//! Shared display formatting with binary byte units.
pub fn fmt_bytes(bytes: u64) -> String {
    binary(bytes as f64)
}
pub fn fmt_rate(bytes_per_second: f64) -> String {
    format!("{}/s", binary(bytes_per_second))
}
pub fn fmt_percent(percent: f64) -> String {
    format!("{percent:.1}%")
}
pub fn fmt_duration(seconds: u64) -> String {
    let days = seconds / 86400;
    let clock = format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    );
    if days > 0 {
        format!("{days}d {clock}")
    } else {
        clock
    }
}
fn binary(mut value: f64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut i = 0;
    while value.abs() >= 1024.0 && i < units.len() - 1 {
        value /= 1024.0;
        i += 1;
    }
    format!("{value:.1} {}", units[i])
}
