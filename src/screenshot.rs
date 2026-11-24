//! Renders the real UI with live data for documentation screenshots.
//!
//! The same `ui::draw` that paints the terminal paints a ratatui
//! `TestBackend`; `svg::buffer_to_svg` then turns that buffer into an image.
//! Nothing is mocked: the app is fed snapshots from the given collector.

use crate::app::App;
use crate::collect::Collector;
use crate::redact::Redactor;
use crate::svg::buffer_to_svg;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use std::thread::sleep;
use std::time::Duration;

/// Feeds `app` with `samples` snapshots one interval apart, after a 250 ms
/// warm-up sample so the first one already has CPU usage and rates. With a
/// redactor, names are masked before the app sees them; numbers stay real.
pub fn collect(
    collector: &mut Collector,
    app: &mut App,
    samples: u32,
    mut redactor: Option<&mut Redactor>,
) {
    collector.sample();
    sleep(Duration::from_millis(250));
    for i in 0..samples {
        if i > 0 {
            sleep(Duration::from_millis(app.interval_ms));
        }
        let mut snapshot = collector.sample();
        if let Some(r) = redactor.as_deref_mut() {
            r.apply(&mut snapshot);
        }
        app.apply_snapshot(snapshot);
    }
}

/// Draws `app` into a `width` x `height` buffer exactly as the terminal would.
pub fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("the test backend cannot fail");
    terminal
        .draw(|frame| {
            crate::ui::prepare_frame(app, frame.area());
            crate::ui::draw(frame, app);
        })
        .expect("the test backend cannot fail");
    terminal.backend().buffer().clone()
}

/// Writes `contents` to `path` through a temporary file in the same
/// directory and a rename, so a full disk never leaves a truncated image and
/// an existing symlink at `path` is replaced rather than followed.
pub fn write_file(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => std::path::Path::new("."),
    };
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("screenshot path has no file name"))?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(
        ".{}.{}.{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        nanos
    ));
    // create_new fails if the name exists, and only a file this call created
    // is ever removed.
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    let result = f
        .write_all(contents.as_bytes())
        .and_then(|()| f.sync_all())
        .and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Renders `app` to an SVG document.
pub fn svg(app: &mut App, width: u16, height: u16) -> String {
    let buffer = render(app, width, height);
    buffer_to_svg(&buffer, &app.palette)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unrelated_leftover_temp_file_is_never_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.svg");
        // A file with the old fixed temporary name must survive.
        let old = dir
            .path()
            .join(format!(".shot.svg.{}.tmp", std::process::id()));
        std::fs::write(&old, "someone else's").unwrap();
        write_file(&path, "<svg/>").unwrap();
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "someone else's");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "<svg/>");
    }
    #[test]
    fn write_replaces_a_symlink_instead_of_following_it() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("precious.txt");
        std::fs::write(&target, "keep me").unwrap();
        let link = dir.path().join("shot.svg");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_file(&link, "<svg/>").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "<svg/>");
        assert!(!std::fs::symlink_metadata(&link).unwrap().is_symlink());
        let leftovers = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 2, "no temporary file left behind");
    }
}
