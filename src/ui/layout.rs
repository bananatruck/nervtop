//! Responsive grid layout calculations for 80x24 up to 4K terminals.

use ratatui::layout::{Constraint, Layout, Rect};

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;

/// Returns true when the terminal is smaller than the minimum 80x24 size.
pub fn is_too_small(area: Rect) -> bool {
    area.width < MIN_WIDTH || area.height < MIN_HEIGHT
}

/// Top-level frame regions shared across all screens.
#[derive(Debug, Clone, Copy)]
pub struct ShellLayout {
    pub header: Rect,
    pub banner: Option<Rect>,
    pub body: Rect,
    pub footer: Rect,
}

/// Regions for the OVERVIEW screen.
#[derive(Debug, Clone, Copy)]
pub struct OverviewLayout {
    pub consensus: Rect,
    pub cpu: Rect,
    pub memory: Rect,
    pub gpu: Rect,
    pub thermal: Rect,
    pub storage: Rect,
    pub network: Rect,
    pub power: Rect,
    pub processes: Rect,
    pub alerts: Rect,
}

/// Splits the terminal frame into header, optional warning-stripe banner, main body and footer.
pub fn split_shell(area: Rect, show_banner: bool) -> ShellLayout {
    if show_banner {
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
        ShellLayout {
            header: chunks[0],
            banner: Some(chunks[1]),
            body: chunks[2],
            footer: chunks[3],
        }
    } else {
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
        ShellLayout {
            header: chunks[0],
            banner: None,
            body: chunks[1],
            footer: chunks[2],
        }
    }
}

/// Splits the overview body into the consensus strip and the nine telemetry panels.
/// At 80x24 (`body.height < 30`) uses a 2-row compact consensus strip and guaranteed-minimum
/// panel heights so every panel title and core metric is visible.
/// At larger sizes (`120x40`, `200x60`) uses a 5-row hexagonal consensus strip and proportional heights.
pub fn split_overview(body: Rect) -> OverviewLayout {
    let consensus_h = if body.height >= 30 { 5 } else { 2 };
    let vert = Layout::vertical([Constraint::Length(consensus_h), Constraint::Min(1)]).split(body);
    let consensus = vert[0];
    let grid = vert[1];

    let cols =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(grid);

    let left = if grid.height <= 22 {
        Layout::vertical([
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(4),
        ])
        .split(cols[0])
    } else {
        Layout::vertical([
            Constraint::Percentage(34),
            Constraint::Percentage(18),
            Constraint::Percentage(24),
            Constraint::Percentage(24),
        ])
        .split(cols[0])
    };

    let right = if grid.height <= 22 {
        Layout::vertical([
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Min(3),
        ])
        .split(cols[1])
    } else {
        Layout::vertical([
            Constraint::Percentage(20),
            Constraint::Percentage(22),
            Constraint::Percentage(18),
            Constraint::Percentage(22),
            Constraint::Percentage(18),
        ])
        .split(cols[1])
    };

    OverviewLayout {
        consensus,
        cpu: left[0],
        memory: left[1],
        gpu: left[2],
        thermal: left[3],
        storage: right[0],
        network: right[1],
        power: right[2],
        processes: right[3],
        alerts: right[4],
    }
}

/// Returns a centred rectangle of at most `width` by `height` inside `area`.
pub fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x.saturating_add(area.width.saturating_sub(w) / 2);
    let y = area.y.saturating_add(area.height.saturating_sub(h) / 2);
    Rect::new(x, y, w, h)
}
