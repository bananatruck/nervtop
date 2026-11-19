//! Formatting helpers for bytes, rates, durations, percentages and unavailable readings.

use crate::alert::Level;
use crate::model::Reading;
use crate::theme::Palette;
use ratatui::buffer::CellWidth;
use ratatui::style::{Color, Style};
use ratatui::text::Span;

const KIB: f64 = 1024.0;
const MIB: f64 = KIB * 1024.0;
const GIB: f64 = MIB * 1024.0;
const TIB: f64 = GIB * 1024.0;

/// Maps an alert `Level` to its semantic colour slot in `palette`.
pub fn level_color(palette: &Palette, level: Level) -> Color {
    match level {
        Level::Nominal => palette.nominal,
        Level::Caution => palette.caution,
        Level::Critical => palette.alert,
        Level::Unknown => palette.unknown,
    }
}

/// Colour for a CPU core bar by load percentage:
/// below 60 nominal, below 85 caution, else alert.
pub fn cpu_bar_color(palette: &Palette, pct: f32) -> Color {
    if pct < 60.0 {
        palette.nominal
    } else if pct < 85.0 {
        palette.caution
    } else {
        palette.alert
    }
}

/// Formats a byte count using binary IEC units (`B`, `KiB`, `MiB`, `GiB`, `TiB`).
pub fn bytes(b: u64) -> String {
    let f = b as f64;
    if b < 1024 {
        format!("{b} B")
    } else if f < MIB {
        format!("{:.1} KiB", f / KIB)
    } else if f < GIB {
        format!("{:.1} MiB", f / MIB)
    } else if f < TIB {
        format!("{:.1} GiB", f / GIB)
    } else {
        format!("{:.2} TiB", f / TIB)
    }
}

/// Formats `used / total` in binary units.
pub fn bytes_pair(used: u64, total: u64) -> String {
    format!("{}/{}", bytes(used), bytes(total))
}

/// Formats a byte rate in bytes per second using binary units (`B/s`, `KiB/s`, `MiB/s`, `GiB/s`).
pub fn rate(bps: f64) -> String {
    let v = if bps.is_finite() && bps >= 0.0 {
        bps
    } else {
        0.0
    };
    if v < KIB {
        format!("{v:.0} B/s")
    } else if v < MIB {
        format!("{:.1} KiB/s", v / KIB)
    } else if v < GIB {
        format!("{:.1} MiB/s", v / MIB)
    } else {
        format!("{:.2} GiB/s", v / GIB)
    }
}

/// Formats a duration in seconds as compact human-readable units.
pub fn duration(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hours = rem / 3_600;
    let rem = rem % 3_600;
    let mins = rem / 60;
    let s = rem % 60;
    if days > 0 {
        format!("{days}d {hours:02}h {mins:02}m")
    } else if hours > 0 {
        format!("{hours}h {mins:02}m {s:02}s")
    } else if mins > 0 {
        format!("{mins}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Formats a percentage with one decimal place.
pub fn percent(pct: f64) -> String {
    if !pct.is_finite() {
        return "0.0%".to_string();
    }
    format!("{pct:.1}%")
}

/// Formats a frequency in MHz as MHz or GHz.
pub fn freq_mhz(mhz: u64) -> String {
    if mhz >= 1000 {
        format!("{:.2} GHz", mhz as f64 / 1000.0)
    } else {
        format!("{mhz} MHz")
    }
}

/// Formats an unavailable reason string, omitting the reason when `max_width` is too tight.
pub fn unavailable_str(reason: &str, max_width: usize) -> String {
    let r = reason.trim();
    if r.is_empty() {
        return "unavailable".to_string();
    }
    let full = format!("unavailable ({r})");
    if max_width >= full.len() {
        full
    } else {
        "unavailable".to_string()
    }
}

/// Builds a `Span` in `palette.unknown` for an unavailable reading.
pub fn unavailable_span(palette: &Palette, reason: &str, max_width: usize) -> Span<'static> {
    Span::styled(
        unavailable_str(reason, max_width),
        Style::default().fg(palette.unknown),
    )
}

/// Renders a `Reading<T>` either via `f` with `ok_style` or as `unavailable` in `palette.unknown`.
pub fn reading_span<T>(
    palette: &Palette,
    reading: &Reading<T>,
    max_width: usize,
    ok_style: Style,
    f: impl FnOnce(&T) -> String,
) -> Span<'static> {
    match reading {
        Reading::Available(v) => Span::styled(f(v), ok_style),
        Reading::Unavailable(r) => unavailable_span(palette, r, max_width),
    }
}

/// Width of `s` in terminal cells, counted the way ratatui lays it out
/// (whole graphemes, control characters dropped).
pub fn display_width(s: &str) -> usize {
    Span::raw(s)
        .styled_graphemes(Style::default())
        .map(|g| usize::from(g.symbol.cell_width()))
        .sum()
}

/// Shortens `s` to at most `max` terminal cells. Nothing is cut when it
/// fits; otherwise whole graphemes are kept and "…" marks the cut.
pub fn fit_width(s: &str, max: usize) -> String {
    if display_width(s) <= max {
        return s.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let room = max - 1;
    let mut out = String::new();
    let mut used = 0;
    for g in Span::raw(s).styled_graphemes(Style::default()) {
        let w = usize::from(g.symbol.cell_width());
        if used + w > room {
            break;
        }
        out.push_str(g.symbol);
        used += w;
    }
    out.push('\u{2026}');
    out
}

/// Builds a horizontal line bar of `width` characters for `pct` (0.0..=100.0),
/// returning `(filled, empty)` using `━` for the filled part and `─` for the empty part.
pub fn bar(width: usize, pct: f64) -> (String, String) {
    if width == 0 {
        return (String::new(), String::new());
    }
    let clamped = if pct.is_finite() {
        pct.clamp(0.0, 100.0)
    } else {
        0.0
    };
    let filled = ((clamped / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width.saturating_sub(filled);
    ("━".repeat(filled), "─".repeat(empty))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeName;

    #[test]
    fn formats_bytes_in_binary_units() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1024), "1.0 KiB");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(bytes(8 * 1024 * 1024 * 1024), "8.0 GiB");
        assert_eq!(bytes(2 * 1024 * 1024 * 1024 * 1024), "2.00 TiB");
        assert_eq!(bytes_pair(1024, 2048), "1.0 KiB/2.0 KiB");
    }

    #[test]
    fn formats_rates_and_handles_edge_values() {
        assert_eq!(rate(0.0), "0 B/s");
        assert_eq!(rate(-5.0), "0 B/s");
        assert_eq!(rate(f64::NAN), "0 B/s");
        assert_eq!(rate(512.0), "512 B/s");
        assert_eq!(rate(2048.0), "2.0 KiB/s");
        assert_eq!(rate(5.0 * 1024.0 * 1024.0), "5.0 MiB/s");
        assert_eq!(rate(1.5 * 1024.0 * 1024.0 * 1024.0), "1.50 GiB/s");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(45), "45s");
        assert_eq!(duration(125), "2m 05s");
        assert_eq!(duration(3661), "1h 01m 01s");
        assert_eq!(duration(90_061), "1d 01h 01m");
    }

    #[test]
    fn formats_percent_freq_and_bars() {
        assert_eq!(percent(0.0), "0.0%");
        assert_eq!(percent(97.34), "97.3%");
        assert_eq!(percent(f64::NAN), "0.0%");
        assert_eq!(freq_mhz(800), "800 MHz");
        assert_eq!(freq_mhz(3450), "3.45 GHz");
        assert_eq!(bar(0, 50.0), ("".into(), "".into()));
        assert_eq!(bar(4, 0.0), ("".into(), "────".into()));
        assert_eq!(bar(4, 50.0), ("━━".into(), "──".into()));
        assert_eq!(bar(4, 100.0), ("━━━━".into(), "".into()));
    }

    #[test]
    fn unavailable_string_always_starts_with_unavailable() {
        assert_eq!(
            unavailable_str("no hwmon sensors", 40),
            "unavailable (no hwmon sensors)"
        );
        assert_eq!(unavailable_str("no hwmon sensors", 11), "unavailable");
        assert_eq!(unavailable_str("", 40), "unavailable");
    }

    #[test]
    fn fit_width_counts_display_cells_and_marks_cuts() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("数据"), 4);
        assert_eq!(fit_width("abcdef", 6), "abcdef");
        assert_eq!(fit_width("abcdef", 4), "abc\u{2026}");
        assert_eq!(fit_width("abcdef", 1), "\u{2026}");
        assert_eq!(fit_width("abcdef", 0), "");
        // A wide character that would cross the limit is left out whole.
        assert_eq!(fit_width("a数据", 4), "a数\u{2026}");
        assert_eq!(fit_width("数据数", 4), "数\u{2026}");
    }

    #[test]
    fn level_and_cpu_bar_colors_come_from_palette() {
        let p = Palette::builtin(ThemeName::Nerv);
        assert_eq!(level_color(&p, Level::Nominal), p.nominal);
        assert_eq!(level_color(&p, Level::Caution), p.caution);
        assert_eq!(level_color(&p, Level::Critical), p.alert);
        assert_eq!(level_color(&p, Level::Unknown), p.unknown);
        assert_eq!(cpu_bar_color(&p, 20.0), p.nominal);
        assert_eq!(cpu_bar_color(&p, 60.0), p.caution);
        assert_eq!(cpu_bar_color(&p, 85.0), p.alert);
    }
}
