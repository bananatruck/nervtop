//! Converts a ratatui [`Buffer`] into a standalone SVG for honest screenshot rendering.

use ratatui::buffer::{Buffer, CellDiffOption, CellWidth};
use ratatui::style::{Color, Modifier};
use std::fmt::Write;

use crate::theme::Palette;

const FONT_SIZE: f64 = 14.0;
const CELL_WIDTH: f64 = 8.4;
const LINE_HEIGHT: f64 = 18.0;
const PADDING: f64 = 12.0;
const FALLBACK_PAGE_BG: &str = "#121712";
const FALLBACK_TEXT: &str = "#dce0d5";
const FONT_FAMILY: &str = "'JetBrains Mono','Cascadia Mono','DejaVu Sans Mono',monospace";

const ANSI_16: [&str; 16] = [
    "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ff", "#cd00cd", "#00cdcd", "#e5e5e5",
    "#7f7f7f", "#ff0000", "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
];

/// Renders `buffer` as an SVG string using colours from `palette` for Reset fallbacks.
pub fn buffer_to_svg(buffer: &Buffer, palette: &Palette) -> String {
    let cols = buffer.area.width as usize;
    let rows = buffer.area.height as usize;
    let content_w = cols as f64 * CELL_WIDTH;
    let content_h = rows as f64 * LINE_HEIGHT;
    let svg_w = PADDING * 2.0 + content_w;
    let svg_h = PADDING * 2.0 + content_h;
    let page_bg = page_background(palette);

    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{svg_w:.1}" height="{svg_h:.1}" viewBox="0 0 {svg_w:.1} {svg_h:.1}" font-family="{FONT_FAMILY}" font-size="{FONT_SIZE:.0}">"#
    );
    let _ = writeln!(
        out,
        r#"<rect width="{svg_w:.1}" height="{svg_h:.1}" fill="{page_bg}"/>"#
    );

    for row in 0..rows {
        let y = PADDING + row as f64 * LINE_HEIGHT;
        let line_start = row * cols;
        let line = &buffer.content[line_start..line_start + cols];
        let hidden = hidden_columns(line);

        emit_background_runs(&mut out, line, &hidden, row, y, &page_bg, palette);
        emit_text_runs(&mut out, line, &hidden, row, y, palette, &page_bg);
    }

    out.push_str("</svg>");
    out
}

fn page_background(palette: &Palette) -> String {
    match palette.background {
        Color::Reset => FALLBACK_PAGE_BG.to_string(),
        other => color_to_hex(other, palette, true),
    }
}

fn hidden_columns(line: &[ratatui::buffer::Cell]) -> Vec<bool> {
    let mut hidden = vec![false; line.len()];
    let mut skip: u16 = 0;
    for (x, cell) in line.iter().enumerate() {
        if skip > 0 {
            hidden[x] = true;
            skip -= 1;
        }
        skip = skip.max(cell.cell_width()).saturating_sub(1);
    }
    hidden
}

fn emit_background_runs(
    out: &mut String,
    line: &[ratatui::buffer::Cell],
    hidden: &[bool],
    _row: usize,
    y: f64,
    page_bg: &str,
    palette: &Palette,
) {
    let width = line.len();
    let mut x = 0;
    while x < width {
        if hidden[x] {
            x += 1;
            continue;
        }
        if matches!(line[x].diff_option, CellDiffOption::Skip) {
            x += 1;
            continue;
        }
        let bg = effective_bg(&line[x], palette, page_bg);
        if should_skip_bg_paint(&bg, page_bg) {
            x += 1;
            continue;
        }
        let start = x;
        let mut run_w = 1usize;
        x += 1;
        while x < width && !hidden[x] && !matches!(line[x].diff_option, CellDiffOption::Skip) {
            let next_bg = effective_bg(&line[x], palette, page_bg);
            if next_bg != bg || should_skip_bg_paint(&next_bg, page_bg) {
                break;
            }
            run_w += 1;
            x += 1;
        }
        let rx = PADDING + start as f64 * CELL_WIDTH;
        let rw = run_w as f64 * CELL_WIDTH;
        let _ = writeln!(
            out,
            r#"<rect x="{rx:.1}" y="{y:.1}" width="{rw:.1}" height="{LINE_HEIGHT:.1}" fill="{bg}"/>"#
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TextStyleKey {
    fg: Color,
    bg: Color,
    modifier: Modifier,
}

fn text_style_key(cell: &ratatui::buffer::Cell) -> TextStyleKey {
    TextStyleKey {
        fg: cell.fg,
        bg: cell.bg,
        modifier: cell.modifier,
    }
}

fn emit_text_runs(
    out: &mut String,
    line: &[ratatui::buffer::Cell],
    hidden: &[bool],
    _row: usize,
    y: f64,
    palette: &Palette,
    page_bg: &str,
) {
    let width = line.len();
    let mut x = 0;
    while x < width {
        if hidden[x] {
            x += 1;
            continue;
        }
        let cell = &line[x];
        if matches!(cell.diff_option, CellDiffOption::Skip) {
            x += 1;
            continue;
        }
        let key = text_style_key(cell);
        if cell.symbol().chars().all(|ch| ch == ' ') {
            x += advance_columns(cell);
            continue;
        }

        let start_x = x;
        let mut col_count = 0u32;
        let mut run_text = String::new();

        while x < width && !hidden[x] {
            let c = &line[x];
            if matches!(c.diff_option, CellDiffOption::Skip) {
                break;
            }
            if text_style_key(c) != key {
                break;
            }
            let sym = c.symbol();
            if sym.chars().all(|ch| ch == ' ') {
                break;
            }
            let w = c.cell_width();
            run_text.push_str(sym);
            col_count += u32::from(w);
            x += usize::from(w);
            if w > 1 {
                break;
            }
        }

        if col_count == 0 || run_text.chars().all(|ch| ch == ' ') {
            continue;
        }

        let (fg_hex, opacity, attrs) = text_attributes(&line[start_x], palette, page_bg);
        let tx = PADDING + start_x as f64 * CELL_WIDTH;
        let ty = y + LINE_HEIGHT * 0.85;
        let text_len = col_count as f64 * CELL_WIDTH;
        let escaped = xml_escape(&run_text);

        let _ = writeln!(
            out,
            r#"<text x="{tx:.1}" y="{ty:.1}" fill="{fg_hex}"{attrs}{opacity} xml:space="preserve" textLength="{text_len:.1}" lengthAdjust="spacingAndGlyphs">{escaped}</text>"#
        );
    }
}

fn advance_columns(cell: &ratatui::buffer::Cell) -> usize {
    usize::from(cell.cell_width()).max(1)
}

fn should_skip_bg_paint(bg_hex: &str, page_bg: &str) -> bool {
    bg_hex == page_bg
}

fn effective_bg(cell: &ratatui::buffer::Cell, palette: &Palette, page_bg: &str) -> String {
    let (fg, bg) = if cell.modifier.contains(Modifier::REVERSED) {
        (cell.bg, cell.fg)
    } else {
        (cell.fg, cell.bg)
    };
    let _ = fg;
    resolve_bg_color(bg, palette, page_bg)
}

fn text_attributes(
    cell: &ratatui::buffer::Cell,
    palette: &Palette,
    page_bg: &str,
) -> (String, String, String) {
    let (fg, bg) = if cell.modifier.contains(Modifier::REVERSED) {
        (cell.bg, cell.fg)
    } else {
        (cell.fg, cell.bg)
    };
    let _ = bg;
    let fg_hex = resolve_fg_color(fg, palette, page_bg);
    let mut attrs = String::new();
    if cell.modifier.contains(Modifier::BOLD) {
        attrs.push_str(r#" font-weight="bold""#);
    }
    if cell.modifier.contains(Modifier::ITALIC) {
        attrs.push_str(r#" font-style="italic""#);
    }
    if cell.modifier.contains(Modifier::UNDERLINED) {
        attrs.push_str(r#" text-decoration="underline""#);
    }
    let opacity = if cell.modifier.contains(Modifier::DIM) {
        r#" opacity="0.6""#.to_string()
    } else {
        String::new()
    };
    (fg_hex, opacity, attrs)
}

fn resolve_fg_color(color: Color, palette: &Palette, _page_bg: &str) -> String {
    match color {
        Color::Reset => {
            if matches!(palette.text, Color::Reset) {
                FALLBACK_TEXT.to_string()
            } else {
                color_to_hex(palette.text, palette, true)
            }
        }
        other => color_to_hex(other, palette, false),
    }
}

fn resolve_bg_color(color: Color, palette: &Palette, page_bg: &str) -> String {
    match color {
        Color::Reset => page_bg.to_string(),
        other => color_to_hex(other, palette, true),
    }
}

fn color_to_hex(color: Color, _palette: &Palette, _is_bg: bool) -> String {
    match color {
        Color::Reset => FALLBACK_PAGE_BG.to_string(),
        Color::Black => ANSI_16[0].to_string(),
        Color::Red => ANSI_16[1].to_string(),
        Color::Green => ANSI_16[2].to_string(),
        Color::Yellow => ANSI_16[3].to_string(),
        Color::Blue => ANSI_16[4].to_string(),
        Color::Magenta => ANSI_16[5].to_string(),
        Color::Cyan => ANSI_16[6].to_string(),
        Color::Gray => ANSI_16[7].to_string(),
        Color::DarkGray => ANSI_16[8].to_string(),
        Color::LightRed => ANSI_16[9].to_string(),
        Color::LightGreen => ANSI_16[10].to_string(),
        Color::LightYellow => ANSI_16[11].to_string(),
        Color::LightBlue => ANSI_16[12].to_string(),
        Color::LightMagenta => ANSI_16[13].to_string(),
        Color::LightCyan => ANSI_16[14].to_string(),
        Color::White => ANSI_16[15].to_string(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => indexed_to_hex(i),
    }
}

fn indexed_to_hex(i: u8) -> String {
    if i < 16 {
        return ANSI_16[i as usize].to_string();
    }
    if i < 232 {
        let i = i - 16;
        let r = i / 36;
        let g = (i % 36) / 6;
        let b = i % 6;
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        return format!(
            "#{:02x}{:02x}{:02x}",
            LEVELS[r as usize], LEVELS[g as usize], LEVELS[b as usize]
        );
    }
    let level = 8 + (i - 232) * 10;
    format!("#{level:02x}{level:02x}{level:02x}")
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Palette, ThemeName};
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};

    fn nerv_palette() -> Palette {
        Palette::builtin(ThemeName::Nerv)
    }

    fn empty_buf(cols: u16, rows: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, cols, rows))
    }

    #[test]
    fn svg_dimensions_for_10x3_buffer() {
        let buf = empty_buf(10, 3);
        let svg = buffer_to_svg(&buf, &nerv_palette());
        let expected_w = PADDING * 2.0 + 10.0 * CELL_WIDTH;
        let expected_h = PADDING * 2.0 + 3.0 * LINE_HEIGHT;
        assert!(svg.contains(&format!(r#"width="{expected_w:.1}""#)));
        assert!(svg.contains(&format!(r#"height="{expected_h:.1}""#)));
        assert!(svg.contains(&format!(r#"viewBox="0 0 {expected_w:.1} {expected_h:.1}""#)));
    }

    #[test]
    fn xml_escaping_in_text() {
        let mut buf = empty_buf(8, 1);
        buf.set_string(0, 0, "a<b&>'\"", Style::default());
        let svg = buffer_to_svg(&buf, &nerv_palette());
        assert!(svg.contains("a&lt;b&amp;&gt;&apos;&quot;"));
    }

    #[test]
    fn one_style_row_merges_to_single_text() {
        let mut buf = empty_buf(5, 1);
        buf.set_string(0, 0, "hello", Style::default().fg(Color::Rgb(1, 2, 3)));
        let svg = buffer_to_svg(&buf, &nerv_palette());
        assert_eq!(svg.matches("<text").count(), 1);
    }

    #[test]
    fn background_runs_merge() {
        let mut buf = empty_buf(6, 1);
        buf.set_style(
            Rect::new(1, 0, 4, 1),
            Style::default().bg(Color::Rgb(0xaa, 0xbb, 0xcc)),
        );
        let svg = buffer_to_svg(&buf, &nerv_palette());
        assert_eq!(svg.matches("fill=\"#aabbcc\"").count(), 1);
    }

    #[test]
    fn bold_and_reversed_attributes() {
        let mut buf = empty_buf(3, 1);
        buf[(0, 0)].set_symbol("B").set_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
        buf[(1, 0)].set_symbol("R").set_style(
            Style::default()
                .fg(Color::Red)
                .bg(Color::Blue)
                .add_modifier(Modifier::REVERSED),
        );
        let svg = buffer_to_svg(&buf, &nerv_palette());
        assert!(svg.contains(r#"font-weight="bold""#));
        assert!(svg.contains("fill=\"#0000ff\""));
    }

    #[test]
    fn reset_colours_use_palette_and_fallback() {
        let mut palette = nerv_palette();
        palette.background = Color::Reset;
        palette.text = Color::Reset;
        let mut buf = empty_buf(1, 1);
        buf[(0, 0)].set_symbol("x").set_style(Style::default());
        let svg = buffer_to_svg(&buf, &palette);
        assert!(svg.contains(&format!(r#"fill="{FALLBACK_PAGE_BG}""#)));
        assert!(svg.contains(&format!(r#"fill="{FALLBACK_TEXT}""#)));
    }

    #[test]
    fn indexed_colours_map_to_xterm() {
        assert_eq!(indexed_to_hex(196), "#ff0000");
        assert_eq!(indexed_to_hex(244), "#808080");
        let mut buf = empty_buf(1, 1);
        buf[(0, 0)].set_symbol("!").set_fg(Color::Indexed(196));
        let svg = buffer_to_svg(&buf, &nerv_palette());
        assert!(svg.contains("fill=\"#ff0000\""));
    }

    #[test]
    fn wide_character_aligns_following_column() {
        let mut buf = empty_buf(4, 1);
        buf.set_string(0, 0, "あb", Style::default().fg(Color::Green));
        let svg = buffer_to_svg(&buf, &nerv_palette());
        let b_x = PADDING + 2.0 * CELL_WIDTH;
        assert!(svg.contains(&format!(r#"x="{b_x:.1}""#)));
        assert_eq!(svg.matches("<text").count(), 2);
    }

    #[test]
    fn text_tags_are_balanced() {
        let mut buf = empty_buf(8, 4);
        buf.set_string(
            0,
            0,
            "line1",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        );
        buf.set_string(0, 1, "あAB", Style::default().fg(Color::Cyan));
        buf.set_string(0, 2, "x & y", Style::default());
        let svg = buffer_to_svg(&buf, &nerv_palette());
        assert_eq!(svg.matches("<text").count(), svg.matches("</text>").count());
    }

    #[test]
    fn output_is_deterministic() {
        let mut buf = empty_buf(3, 2);
        buf.set_string(0, 0, "ab", Style::default());
        let p = nerv_palette();
        assert_eq!(buffer_to_svg(&buf, &p), buffer_to_svg(&buf, &p));
    }
}
