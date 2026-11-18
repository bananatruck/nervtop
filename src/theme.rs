//! Colour themes.
//!
//! A `Palette` is the only place colours live. Widgets ask the palette for a
//! semantic slot (`nominal`, `caution`, `alert`, ...) instead of naming colours.

use ratatui::style::Color;
use serde::Deserialize;
use std::fmt;
use std::str::FromStr;

/// Built-in themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    /// 24-bit colour, charcoal green ground with amber and orange, matching nerv-fieldnotes.
    Nerv,
    /// Green monochrome CRT.
    Phosphor,
    /// Only the 16 terminal palette colours, so it follows the terminal's own scheme.
    Ansi,
}

impl ThemeName {
    pub const ALL: [ThemeName; 3] = [ThemeName::Nerv, ThemeName::Phosphor, ThemeName::Ansi];

    pub fn as_str(self) -> &'static str {
        match self {
            ThemeName::Nerv => "nerv",
            ThemeName::Phosphor => "phosphor",
            ThemeName::Ansi => "ansi",
        }
    }

    /// The next theme in `ALL`, wrapping around.
    pub fn next(self) -> ThemeName {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

impl fmt::Display for ThemeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ThemeName {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| format!("unknown theme \"{s}\" (expected nerv, phosphor or ansi)"))
    }
}

/// Every colour slot a widget may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Painted behind everything. `Color::Reset` leaves the terminal background visible.
    pub background: Color,
    /// Slightly raised panel fill, used for the header and overlays.
    pub surface: Color,
    /// Panel borders and rules.
    pub border: Color,
    /// Normal body text.
    pub text: Color,
    /// Labels, units and secondary text.
    pub muted: Color,
    /// Highlights, selection and the program mark.
    pub accent: Color,
    /// Panel titles.
    pub heading: Color,
    /// Healthy values.
    pub nominal: Color,
    /// Values past a warn threshold.
    pub caution: Color,
    /// Values past a crit threshold.
    pub alert: Color,
    /// Values that could not be read.
    pub unknown: Color,
    /// The dark band of warning stripes.
    pub stripe_dark: Color,
}

impl Palette {
    pub fn builtin(name: ThemeName) -> Palette {
        match name {
            ThemeName::Nerv => Palette {
                background: rgb(0x12, 0x17, 0x12),
                surface: rgb(0x1b, 0x22, 0x1b),
                border: rgb(0x4b, 0x55, 0x42),
                text: rgb(0xdc, 0xe0, 0xd5),
                muted: rgb(0x8c, 0x99, 0x80),
                accent: rgb(0xef, 0x7b, 0x36),
                heading: rgb(0xe9, 0xa9, 0x3b),
                nominal: rgb(0xc6, 0xdb, 0x82),
                caution: rgb(0xf2, 0xc1, 0x4e),
                alert: rgb(0xe5, 0x48, 0x4d),
                unknown: rgb(0x6f, 0x7a, 0x68),
                stripe_dark: rgb(0x12, 0x17, 0x12),
            },
            ThemeName::Phosphor => Palette {
                background: rgb(0x07, 0x0d, 0x08),
                surface: rgb(0x0c, 0x16, 0x0e),
                border: rgb(0x1f, 0x5c, 0x2c),
                text: rgb(0x9c, 0xf5, 0xa8),
                muted: rgb(0x4f, 0x9a, 0x5c),
                accent: rgb(0xd9, 0xff, 0xde),
                heading: rgb(0x5d, 0xff, 0x7d),
                nominal: rgb(0x5d, 0xff, 0x7d),
                caution: rgb(0xe8, 0xff, 0x6a),
                alert: rgb(0xff, 0x5f, 0x5f),
                unknown: rgb(0x35, 0x66, 0x3d),
                stripe_dark: rgb(0x07, 0x0d, 0x08),
            },
            ThemeName::Ansi => Palette {
                background: Color::Reset,
                surface: Color::Reset,
                border: Color::DarkGray,
                text: Color::White,
                muted: Color::Gray,
                accent: Color::LightRed,
                heading: Color::Yellow,
                nominal: Color::Green,
                caution: Color::Yellow,
                alert: Color::Red,
                unknown: Color::DarkGray,
                stripe_dark: Color::Black,
            },
        }
    }

    /// Sets the slot called `slot` to `color`.
    pub fn set(&mut self, slot: &str, color: Color) -> Result<(), String> {
        let target = match slot {
            "background" => &mut self.background,
            "surface" => &mut self.surface,
            "border" => &mut self.border,
            "text" => &mut self.text,
            "muted" => &mut self.muted,
            "accent" => &mut self.accent,
            "heading" => &mut self.heading,
            "nominal" => &mut self.nominal,
            "caution" => &mut self.caution,
            "alert" => &mut self.alert,
            "unknown" => &mut self.unknown,
            "stripe_dark" => &mut self.stripe_dark,
            other => return Err(format!("unknown colour slot \"{other}\"")),
        };
        *target = color;
        Ok(())
    }
}

/// Every slot name accepted by `Palette::set`, in documentation order.
pub const SLOTS: [&str; 12] = [
    "background",
    "surface",
    "border",
    "text",
    "muted",
    "accent",
    "heading",
    "nominal",
    "caution",
    "alert",
    "unknown",
    "stripe_dark",
];

/// Parses `#rrggbb`, `reset`, or an ANSI colour name such as `red` or `lightblue`.
pub fn parse_color(s: &str) -> Result<Color, String> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix('#') {
        if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let v = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
            return Ok(rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
        }
        return Err(format!("\"{s}\" is not a #rrggbb colour"));
    }
    let named = match t.to_ascii_lowercase().replace(['_', '-', ' '], "").as_str() {
        "reset" | "default" | "terminal" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        "white" => Color::White,
        _ => {
            return Err(format!(
                "\"{s}\" is not a colour (use #rrggbb, reset or an ANSI name)"
            ));
        }
    };
    Ok(named)
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_names_and_reset() {
        assert_eq!(parse_color("#ef7b36"), Ok(Color::Rgb(0xef, 0x7b, 0x36)));
        assert_eq!(parse_color("LightBlue"), Ok(Color::LightBlue));
        assert_eq!(parse_color("dark-gray"), Ok(Color::DarkGray));
        assert_eq!(parse_color("reset"), Ok(Color::Reset));
        assert!(parse_color("#12345").is_err());
        assert!(parse_color("#gggggg").is_err());
        assert!(parse_color("orange").is_err());
    }

    #[test]
    fn theme_names_round_trip_and_cycle() {
        for t in ThemeName::ALL {
            assert_eq!(t.as_str().parse::<ThemeName>(), Ok(t));
        }
        assert_eq!(ThemeName::Nerv.next(), ThemeName::Phosphor);
        assert_eq!(ThemeName::Ansi.next(), ThemeName::Nerv);
        assert!("magi".parse::<ThemeName>().is_err());
    }

    #[test]
    fn every_documented_slot_is_settable() {
        let mut p = Palette::builtin(ThemeName::Nerv);
        for slot in SLOTS {
            p.set(slot, Color::Blue).unwrap();
        }
        assert_eq!(p.stripe_dark, Color::Blue);
        assert!(p.set("glow", Color::Blue).is_err());
    }
}
