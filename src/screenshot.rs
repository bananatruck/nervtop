//! Renders the real UI with live data for documentation screenshots.
//!
//! The same `ui::draw` that paints the terminal paints a ratatui
//! `TestBackend`; `svg::buffer_to_svg` then turns that buffer into an image.
//! Nothing is mocked: the app is fed snapshots from the given collector.

use crate::app::App;
use crate::collect::Collector;
use crate::svg::buffer_to_svg;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use std::thread::sleep;
use std::time::Duration;

/// Feeds `app` with `samples` snapshots one interval apart, after a 250 ms
/// warm-up sample so the first one already has CPU usage and rates.
pub fn collect(collector: &mut Collector, app: &mut App, samples: u32) {
    collector.sample();
    sleep(Duration::from_millis(250));
    for i in 0..samples {
        if i > 0 {
            sleep(Duration::from_millis(app.interval_ms));
        }
        app.apply_snapshot(collector.sample());
    }
}

/// Draws `app` into a `width` x `height` buffer exactly as the terminal would.
pub fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("the test backend cannot fail");
    terminal
        .draw(|frame| {
            app.set_page_size(usize::from(frame.area().height.saturating_sub(8).max(1)));
            crate::ui::draw(frame, app);
        })
        .expect("the test backend cannot fail");
    terminal.backend().buffer().clone()
}

/// Renders `app` to an SVG document.
pub fn svg(app: &mut App, width: u16, height: u16) -> String {
    let buffer = render(app, width, height);
    buffer_to_svg(&buffer, &app.palette)
}
