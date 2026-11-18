//! Drawing. `draw` renders an `&App` into a frame and keeps no state.

use crate::app::App;
use ratatui::Frame;

/// Draws the whole screen for `app`.
pub fn draw(frame: &mut Frame, app: &App) {
    let _ = (frame, app);
}
