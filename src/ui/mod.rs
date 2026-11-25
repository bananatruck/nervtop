//! Drawing. `draw` renders an `&App` into a frame and keeps no state.

pub mod alerts;
pub mod cpu;
pub mod diagnostics;
pub mod fmt;
pub mod gpu;
pub mod header;
pub mod help;
pub mod hex;
pub mod layout;
pub mod memory;
pub mod network;
pub mod power;
pub mod processes;
pub mod storage;
pub mod stripes;
#[cfg(test)]
pub mod testkit;
pub mod thermal;

use crate::alert::Level;
use crate::app::{App, Screen};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

/// Updates navigation bounds for the frame about to be drawn.
pub fn prepare_frame(app: &mut App, area: Rect) {
    app.set_page_size(usize::from(area.height.saturating_sub(8).max(1)));
    // A frame too small for any table has no bounds; the saved position is
    // kept until the tables can be drawn again.
    let bounds = (!layout::is_too_small(area)).then(|| {
        let shell = layout::split_shell(area, app.worst_level() == Level::Critical);
        diagnostics::bounds(shell.body, app)
    });
    app.set_diagnostics_bounds(bounds);
}

/// Draws the whole screen for `app`.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    // Paint the entire frame with `app.palette.background` first.
    let bg_block = Block::new().style(
        Style::default()
            .bg(app.palette.background)
            .fg(app.palette.text),
    );
    frame.render_widget(bg_block, area);

    if layout::is_too_small(area) {
        draw_too_small(frame, area, app);
        return;
    }

    let show_banner = app.worst_level() == Level::Critical;
    let shell = layout::split_shell(area, show_banner);

    header::draw_header(frame, shell.header, app);
    if let Some(banner_area) = shell.banner {
        stripes::draw_banner(frame, banner_area, app);
    }

    match app.screen {
        Screen::Overview => draw_overview(frame, shell.body, app),
        Screen::Processes => processes::draw_processes_screen(frame, shell.body, app),
        Screen::Diagnostics => diagnostics::draw_diagnostics(frame, shell.body, app),
    }

    header::draw_footer(frame, shell.footer, app);

    if app.show_help {
        help::draw_help(frame, area, &app.palette);
    }
}

fn draw_overview(frame: &mut Frame, body: Rect, app: &App) {
    let ov = layout::split_overview(body);
    hex::draw_consensus(frame, ov.consensus, &app.consensus, &app.palette);
    cpu::draw_cpu(frame, ov.cpu, app);
    memory::draw_memory(frame, ov.memory, app);
    gpu::draw_gpu(frame, ov.gpu, app);
    thermal::draw_thermal(frame, ov.thermal, app);
    storage::draw_storage(frame, ov.storage, app);
    network::draw_network(frame, ov.network, app);
    power::draw_power(frame, ov.power, app);
    processes::draw_processes_overview(frame, ov.processes, app);
    alerts::draw_alerts(frame, ov.alerts, app);
}

fn draw_too_small(frame: &mut Frame, area: Rect, app: &App) {
    let msg = format!(
        "TERMINAL TOO SMALL  {}x{}  NEED {}x{}",
        area.width,
        area.height,
        layout::MIN_WIDTH,
        layout::MIN_HEIGHT
    );
    let y = area.y + area.height / 2;
    let row = Rect::new(area.x, y, area.width, 1.min(area.height));
    let line = Line::from(Span::styled(
        msg,
        Style::default()
            .fg(app.palette.caution)
            .bg(app.palette.background)
            .add_modifier(Modifier::BOLD),
    ));
    let p = Paragraph::new(line).alignment(Alignment::Center);
    frame.render_widget(p, row);
}

#[cfg(test)]
mod tests {
    use super::stripes::STRIPE_CHAR;
    use super::testkit::{buffer_lines, buffer_text, critical_disk_app, nominal_app, render};
    use crate::alert::RuleId;
    use crate::app::{App, Screen};
    use crate::config::Config;
    use crate::model::Reading;
    use ratatui::style::Color;

    fn all_unavailable_app() -> App {
        let mut app = nominal_app();
        let mut snap = app.snapshot.clone().unwrap();
        snap.cpu = Reading::unavailable("x");
        snap.memory = Reading::unavailable("x");
        snap.disks = Reading::unavailable("x");
        snap.sensors = Reading::unavailable("x");
        snap.gpus = Reading::unavailable("x");
        snap.battery = Reading::unavailable("x");
        let config = Config::default();
        let palette = config.palette().unwrap();
        app = App::new(config, palette);
        app.apply_snapshot(snap);
        app
    }

    #[test]
    fn nothing_judged_is_never_called_nominal() {
        let app = all_unavailable_app();
        assert_eq!(app.consensus.verdict, crate::alert::Level::Unknown);
        let text = buffer_text(&render(&app, 160, 48));
        assert!(!text.contains("no active alerts"), "{text}");
        assert!(!text.contains("within configured bounds"), "{text}");
        assert!(text.contains("NO DATA"), "{text}");
    }

    #[test]
    fn diagnostics_shows_battery_name_and_charge() {
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        let b = app
            .snapshot
            .as_ref()
            .unwrap()
            .battery
            .value()
            .unwrap()
            .clone();
        let lines = buffer_lines(&render(&app, 160, 48));
        let row = lines
            .iter()
            .find(|l| l.contains("CHARGE"))
            .expect("a CHARGE row in the battery box");
        assert!(row.contains(&b.name), "{row}");
        assert!(row.contains(&format!("{:.1}%", b.percent)), "{row}");
    }

    #[test]
    fn one_long_rule_subject_does_not_hide_the_others() {
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        for r in app.rules.iter_mut() {
            if r.rule == RuleId::GpuTemp {
                r.subject =
                    "0000:01:00.0 NVIDIA GeForce RTX 4060 Laptop GPU with a very long name".into();
            }
        }
        let disks: Vec<String> = app
            .rules
            .iter()
            .filter(|r| r.rule == RuleId::Disk)
            .map(|r| format!("disk {}", r.subject))
            .collect();
        assert!(!disks.is_empty());
        let text = buffer_text(&render(&app, 160, 48));
        for d in disks {
            assert!(text.contains(&d), "{d} missing:\n{text}");
        }
    }

    #[test]
    fn core_strip_fits_many_cores_at_80x24() {
        let mut app = nominal_app();
        if let Some(s) = app.snapshot.as_mut()
            && let Reading::Available(c) = &mut s.cpu
        {
            c.per_core = (0..256).map(|i| (i % 100) as f32).collect();
        }
        let lines = buffer_lines(&render(&app, 80, 24));
        let strip = lines
            .iter()
            .find(|l| l.contains("CORES"))
            .expect("a core strip");
        assert!(
            strip.contains("max C"),
            "the hottest core must stay visible: {strip}"
        );
    }

    #[test]
    fn overview_panel_titles_present_at_80x24_120x40_and_200x60() {
        let app = nominal_app();
        for (w, h) in [(80, 24), (120, 40), (200, 60)] {
            let buf = render(&app, w, h);
            let text = buffer_text(&buf);
            for title in [
                "CPU",
                "MEMORY",
                "GPU",
                "THERMAL",
                "STORAGE",
                "NETWORK",
                "POWER",
                "PROCESSES",
                "ALERTS",
            ] {
                assert!(
                    text.contains(title),
                    "missing panel title {title} at {w}x{h}:\n{text}"
                );
            }
            assert!(text.contains("COMPUTE-1"), "missing COMPUTE-1 at {w}x{h}");
            assert!(text.contains("THERMAL-2"), "missing THERMAL-2 at {w}x{h}");
            assert!(text.contains("RESOURCE-3"), "missing RESOURCE-3 at {w}x{h}");
            assert!(
                text.contains("VERDICT NOMINAL"),
                "missing VERDICT NOMINAL at {w}x{h}"
            );
            assert!(text.contains("UNANIMOUS"), "missing UNANIMOUS at {w}x{h}");
        }
    }

    #[test]
    fn unavailable_shown_for_each_kind_of_unavailable_source() {
        // 1. CPU unavailable
        let mut app = nominal_app();
        app.snapshot.as_mut().unwrap().cpu = Reading::unavailable("warming up");
        let text = buffer_text(&render(&app, 120, 40));
        assert!(text.contains("unavailable (warming up)"), "{text}");

        // 2. Memory unavailable
        let mut app = nominal_app();
        app.snapshot.as_mut().unwrap().memory = Reading::unavailable("proc meminfo missing");
        let text = buffer_text(&render(&app, 120, 40));
        assert!(
            text.contains("unavailable (proc meminfo missing)"),
            "{text}"
        );

        // 3. Disks unavailable
        let mut app = nominal_app();
        app.snapshot.as_mut().unwrap().disks = Reading::unavailable("mounts unreadable");
        let text = buffer_text(&render(&app, 120, 40));
        assert!(text.contains("unavailable (mounts unreadable)"), "{text}");

        // 4. Network unavailable
        let mut app = nominal_app();
        app.snapshot.as_mut().unwrap().network = Reading::unavailable("net dev unreadable");
        let text = buffer_text(&render(&app, 120, 40));
        assert!(text.contains("unavailable (net dev unreadable)"), "{text}");

        // 5. Processes unavailable (both overview and processes screen)
        let mut app = nominal_app();
        app.snapshot.as_mut().unwrap().processes = Reading::unavailable("procfs blocked");
        let text_ov = buffer_text(&render(&app, 120, 40));
        assert!(
            text_ov.contains("unavailable (procfs blocked)"),
            "{text_ov}"
        );
        app.screen = Screen::Processes;
        let text_pr = buffer_text(&render(&app, 120, 40));
        assert!(
            text_pr.contains("unavailable (procfs blocked)"),
            "{text_pr}"
        );

        // 6. Sensors unavailable
        let mut app = nominal_app();
        app.snapshot.as_mut().unwrap().sensors = Reading::unavailable("no hwmon sensors");
        let text = buffer_text(&render(&app, 120, 40));
        assert!(text.contains("unavailable (no hwmon sensors)"), "{text}");

        // 7. GPUs unavailable
        let mut app = nominal_app();
        app.snapshot.as_mut().unwrap().gpus = Reading::unavailable("nvidia-smi not found");
        let text = buffer_text(&render(&app, 120, 40));
        assert!(
            text.contains("unavailable (nvidia-smi not found)"),
            "{text}"
        );

        // 8. Battery unavailable
        let mut app = nominal_app();
        app.snapshot.as_mut().unwrap().battery = Reading::unavailable("no battery present");
        let text = buffer_text(&render(&app, 120, 40));
        assert!(text.contains("unavailable (no battery present)"), "{text}");

        // 9. Host fields unavailable
        let mut app = nominal_app();
        app.snapshot.as_mut().unwrap().host.hostname = Reading::unavailable("no hostname");
        let text = buffer_text(&render(&app, 120, 40));
        assert!(text.contains("unavailable (no hostname)"), "{text}");
    }

    #[test]
    fn suspended_gpu_shows_suspended_and_no_numbers_for_it() {
        let app = nominal_app();
        let text = buffer_text(&render(&app, 120, 40));
        let gpu_line = text
            .lines()
            .find(|l| l.contains("GeForce RTX 4060"))
            .expect("GeForce RTX 4060 line present");
        assert!(gpu_line.contains("SUSPENDED"), "{gpu_line}");
        assert!(!gpu_line.contains("°C"), "{gpu_line}");
        assert!(!gpu_line.contains("GiB"), "{gpu_line}");
    }

    #[test]
    fn critical_banner_present_only_at_critical_and_stripes_absent_when_acknowledged() {
        let nom = nominal_app();
        let nom_text = buffer_text(&render(&nom, 120, 40));
        assert!(!nom_text.contains("CRITICAL  DISK /  97.0%"), "{nom_text}");
        assert!(!nom_text.contains(STRIPE_CHAR), "{nom_text}");

        let mut crit = critical_disk_app();
        let crit_buf = render(&crit, 120, 40);
        let crit_lines = buffer_lines(&crit_buf);
        assert!(
            crit_lines[1].contains("CRITICAL  DISK /  97.0%"),
            "banner row: {}",
            crit_lines[1]
        );
        assert!(
            crit_lines[1].contains(STRIPE_CHAR),
            "expected stripes on banner row: {}",
            crit_lines[1]
        );

        crit.acknowledged = true;
        let ack_buf = render(&crit, 120, 40);
        let ack_lines = buffer_lines(&ack_buf);
        assert!(
            ack_lines[1].contains("CRITICAL  DISK /  97.0%"),
            "acknowledged banner row: {}",
            ack_lines[1]
        );
        assert!(
            !ack_lines[1].contains(STRIPE_CHAR),
            "stripes should be absent when acknowledged: {}",
            ack_lines[1]
        );
    }

    #[test]
    fn help_overlay_text_present_when_show_help_is_true() {
        let mut app = nominal_app();
        app.show_help = true;
        let text = buffer_text(&render(&app, 100, 30));
        assert!(text.contains("HELP - KEY BINDINGS"), "{text}");
        assert!(text.contains("Toggle help overlay"), "{text}");
        assert!(text.contains("Pause or resume sampling"), "{text}");
        assert!(text.contains("Acknowledge current alerts"), "{text}");
    }

    #[test]
    fn too_small_message_shown_at_79x24_and_80x23() {
        let app = nominal_app();
        let text_w = buffer_text(&render(&app, 79, 24));
        assert!(
            text_w.contains("TERMINAL TOO SMALL  79x24  NEED 80x24"),
            "{text_w}"
        );
        assert!(!text_w.contains("COMPUTE-1"), "{text_w}");

        let text_h = buffer_text(&render(&app, 80, 23));
        assert!(
            text_h.contains("TERMINAL TOO SMALL  80x23  NEED 80x24"),
            "{text_h}"
        );
        assert!(!text_h.contains("COMPUTE-1"), "{text_h}");
    }

    #[test]
    fn processes_screen_highlights_selected_pid_row() {
        let mut app = nominal_app();
        app.screen = Screen::Processes;
        app.procs.selected_pid = Some(4242);

        let buf = render(&app, 120, 40);
        let lines = buffer_lines(&buf);

        let sel_y = lines
            .iter()
            .position(|l| l.contains("4242") && l.contains("nervtop"))
            .expect("selected process row 4242 is rendered") as u16;
        let unsel_y = lines
            .iter()
            .position(|l| l.contains("1337") && l.contains("rustc"))
            .expect("unselected process row 1337 is rendered") as u16;

        let sel_cell = &buf[(4, sel_y)];
        let unsel_cell = &buf[(4, unsel_y)];
        assert_ne!(
            sel_cell.style(),
            unsel_cell.style(),
            "selected row style must differ from unselected row style"
        );
        assert_eq!(sel_cell.fg, app.palette.accent);
    }

    #[test]
    fn diagnostics_screen_lists_every_rule() {
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        for (w, h) in [(80, 24), (120, 40), (200, 60)] {
            let text = buffer_text(&render(&app, w, h));
            for rule in RuleId::ALL {
                assert!(
                    text.contains(rule.key()),
                    "missing rule key {} at {w}x{h}:\n{text}",
                    rule.key()
                );
                assert!(
                    text.contains(rule.label()),
                    "missing rule label {} at {w}x{h}:\n{text}",
                    rule.label()
                );
            }
            assert!(
                text.contains("SUSPENDED"),
                "missing SUSPENDED on diagnostics at {w}x{h}:\n{text}"
            );
        }
    }

    #[test]
    fn swap_none_configured_thermal_crit_mark_paused_and_acquiring_states() {
        // Swap total 0 -> "none configured"
        let mut app = nominal_app();
        app.snapshot.as_mut().unwrap().memory.value().unwrap();
        if let Reading::Available(mem) = &mut app.snapshot.as_mut().unwrap().memory {
            mem.swap_total_bytes = 0;
            mem.swap_used_bytes = 0;
        }
        app.paused = true;
        let text = buffer_text(&render(&app, 120, 40));
        assert!(text.contains("none configured"), "{text}");
        assert!(text.contains("PAUSED"), "{text}");
        // Sensor k10temp:Tctl is 92.0°C with crit 100.0°C (within 10°C), so marked with "!"
        assert!(text.contains("! (crit 100°C)"), "{text}");

        // Snapshot is None -> panels show "acquiring"
        let cfg = Config::default();
        let pal = cfg.palette().unwrap();
        let acquiring_app = App::new(cfg, pal);
        let acq_text = buffer_text(&render(&acquiring_app, 120, 40));
        assert!(acq_text.contains("acquiring"), "{acq_text}");
    }

    #[test]
    fn no_panic_across_all_sizes_and_screens() {
        let mut app = critical_disk_app();
        let sizes = [(1, 1), (10, 5), (79, 23), (80, 24), (81, 25), (300, 100)];
        for screen in Screen::ALL {
            app.screen = screen;
            for show_help in [false, true] {
                app.show_help = show_help;
                for (w, h) in sizes {
                    let _ = render(&app, w, h);
                }
            }
        }

        // Also test when snapshot is None ("acquiring" state)
        let cfg = Config::default();
        let pal = cfg.palette().unwrap();
        let empty_app = App::new(cfg, pal);
        for screen in Screen::ALL {
            let mut a = empty_app.clone();
            a.screen = screen;
            for (w, h) in sizes {
                let _ = render(&a, w, h);
            }
        }
    }

    #[test]
    fn draw_uses_palette_background_color_for_cell_0_0_region() {
        let mut app = nominal_app();
        let custom_bg = Color::Rgb(0x23, 0x34, 0x45);
        app.palette.background = custom_bg;

        let buf_small = render(&app, 79, 24);
        assert_eq!(buf_small[(0, 0)].bg, custom_bg);

        let buf_full = render(&app, 80, 24);
        assert_eq!(buf_full[(0, 0)].bg, custom_bg);
        assert_eq!(buf_full[(0, 2)].bg, custom_bg);
        assert_eq!(buf_full[(0, 3)].bg, custom_bg);
    }

    #[test]
    fn header_clock_present_when_offset_some_and_absent_when_none() {
        let mut app = nominal_app();
        // taken_at_ms = 1_763_452_319_000 -> 07:51:59 UTC -> 23:51:59 at UTC-8
        app.utc_offset_secs = None;
        let lines_none = buffer_lines(&render(&app, 120, 40));
        assert!(!lines_none[0].contains("23:51:59"), "{}", lines_none[0]);
        assert!(!lines_none[0].contains("07:51:59"), "{}", lines_none[0]);

        app.utc_offset_secs = Some(-8 * 3600);
        let lines_some = buffer_lines(&render(&app, 120, 40));
        assert!(
            lines_some[0].ends_with("23:51:59"),
            "header should end with local clock: '{}'",
            lines_some[0]
        );
    }

    #[test]
    fn header_at_80_columns_drops_whole_parts_never_cuts_words_and_keeps_clock() {
        let mut app = nominal_app();
        app.utc_offset_secs = Some(-8 * 3600);

        let lines = buffer_lines(&render(&app, 80, 24));
        let hdr = &lines[0];
        assert!(
            hdr.contains("1 OVR") || hdr.contains("1:OVERVIEW"),
            "expected short or full tab label in 80-col header: '{hdr}'"
        );
        assert!(
            hdr.ends_with("23:51:59"),
            "expected clock at right end of 80-col header: '{hdr}'"
        );
        assert!(hdr.starts_with("NERVTOP "), "'{hdr}'");
        assert!(hdr.contains("1000ms"), "'{hdr}'");

        // Ensure no word is cut in half: every whitespace-separated token must be a complete part
        let valid_tokens = [
            "NERVTOP",
            "magi-01",
            "6.12.4-arch1",
            "up",
            "4h",
            "02m",
            "00s",
            "[1:OVERVIEW]",
            "2:PROCESSES",
            "3:DIAGNOSTICS",
            "[1",
            "OVR]",
            "1",
            "OVR",
            "2",
            "PROC",
            "3",
            "DIAG",
            "1000ms",
            "nerv",
            "PAUSED",
            "23:51:59",
        ];
        for tok in hdr.split_whitespace() {
            assert!(
                valid_tokens.contains(&tok),
                "unexpected or cut token '{tok}' in 80-col header '{hdr}'"
            );
        }

        // With a longer hostname and PAUSED active at 80 columns, short tabs `1 OVR` are used
        app.paused = true;
        app.snapshot.as_mut().unwrap().host.hostname =
            Reading::Available("archlinux-workstation".into());
        let paused_hdr = &buffer_lines(&render(&app, 80, 24))[0];
        assert!(paused_hdr.contains("[1 OVR]"), "'{paused_hdr}'");
        assert!(paused_hdr.contains("2 PROC"), "'{paused_hdr}'");
        assert!(paused_hdr.contains("3 DIAG"), "'{paused_hdr}'");
        assert!(paused_hdr.contains("PAUSED"), "'{paused_hdr}'");
        assert!(paused_hdr.ends_with("23:51:59"), "'{paused_hdr}'");
    }

    #[test]
    fn hex_cell_area_contains_no_reason_text_while_verdict_box_keeps_it() {
        let app = critical_disk_app();
        let buf = render(&app, 160, 48);
        let lines = buffer_lines(&buf);
        // Row 0 is header, Row 1 is critical banner, Rows 2..7 are the 5-row consensus strip.
        // The three hex cells occupy columns 0..66 (3 * 22); the verdict box is at 66..160.
        for row in &lines[2..7] {
            let hex_region: String = row.chars().take(66).collect();
            assert!(
                !hex_region.contains("DISK /"),
                "hex cell region must not contain reason text: '{hex_region}'"
            );
        }
        let verdict_region: String = lines[2..7]
            .iter()
            .map(|r| r.chars().skip(66).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            verdict_region.contains("RESOURCE-3: DISK / 97.0%"),
            "verdict box must contain reason text:\n{verdict_region}"
        );
    }

    #[test]
    fn overview_at_80x24_shows_all_16_core_glyphs_and_hottest_core() {
        let app = nominal_app();
        let text = buffer_text(&render(&app, 80, 24));
        let cores_line = text
            .lines()
            .find(|l| l.contains("CORES "))
            .expect("compact CORES strip present at 80x24");
        // nominal_app has 16 cores; core 6 is 88.0%
        let expected_glyphs = "▂▂▅▁▃▄█▂▂▂▃▄▁▁▃▃";
        assert_eq!(expected_glyphs.chars().count(), 16);
        assert!(
            cores_line.contains(&format!("CORES {expected_glyphs} max C06 88%")),
            "expected all 16 core glyphs and max C06 88% on line: '{cores_line}'"
        );
    }

    #[test]
    fn memory_rows_have_no_gauge_overlap_and_match_exact_ram_line() {
        let app = nominal_app();
        let buf = render(&app, 120, 40);
        let lines = buffer_lines(&buf);
        let ram_row = lines
            .iter()
            .find(|l| l.contains("RAM  "))
            .expect("RAM row present");
        // Left column at 120x40 is 60 wide; inner content is between borders at cols 1..59
        let ram_inner: String = ram_row.chars().skip(1).take(58).collect();
        assert_eq!(
            ram_inner.trim_end(),
            "RAM  ━━━━──── 7.0 GiB/16.0 GiB  43.8%"
        );

        let swap_row = lines
            .iter()
            .find(|l| l.contains("SWAP "))
            .expect("SWAP row present");
        let swap_inner: String = swap_row.chars().skip(1).take(58).collect();
        assert_eq!(
            swap_inner.trim_end(),
            "SWAP ━─────── 512.0 MiB/4.0 GiB  12.5%"
        );
    }

    #[test]
    fn network_orders_busiest_interface_first_on_overview_and_diagnostics() {
        use crate::model::NetInterface;
        let mut app = nominal_app();
        // Put idle docker bridges before wlan0; wlan0 has higher rate (and higher total when rates tie)
        app.snapshot.as_mut().unwrap().network = Reading::Available(vec![
            NetInterface {
                name: "br-7237d48b080b".into(),
                rx_bps: Reading::Available(0.0),
                tx_bps: Reading::Available(0.0),
                rx_total_bytes: Reading::Available(230 * 1024),
                tx_total_bytes: Reading::Available(260 * 1024),
            },
            NetInterface {
                name: "veth5d9f83b".into(),
                rx_bps: Reading::unavailable("warming up"),
                tx_bps: Reading::unavailable("warming up"),
                rx_total_bytes: Reading::Available(126),
                tx_total_bytes: Reading::Available(50 * 1024),
            },
            NetInterface {
                name: "wlan0".into(),
                rx_bps: Reading::Available(0.0),
                tx_bps: Reading::Available(0.0),
                rx_total_bytes: Reading::Available(8 * 1024 * 1024 * 1024),
                tx_total_bytes: Reading::Available(1024 * 1024 * 1024),
            },
        ]);

        let ov_lines = buffer_lines(&render(&app, 160, 48));
        let wlan_ov = ov_lines
            .iter()
            .position(|l| l.contains("wlan0"))
            .expect("wlan0 in overview");
        let br_ov = ov_lines
            .iter()
            .position(|l| l.contains("br-7237d48b080b"))
            .expect("br in overview");
        assert!(
            wlan_ov < br_ov,
            "wlan0 (row {wlan_ov}) must appear before idle bridge (row {br_ov}) in overview"
        );

        app.screen = Screen::Diagnostics;
        let diag_lines = buffer_lines(&render(&app, 160, 48));
        let wlan_diag = diag_lines
            .iter()
            .position(|l| l.contains("wlan0"))
            .expect("wlan0 in diagnostics");
        let br_diag = diag_lines
            .iter()
            .position(|l| l.contains("br-7237d48b080b"))
            .expect("br in diagnostics");
        assert!(
            wlan_diag < br_diag,
            "wlan0 (row {wlan_diag}) must appear before idle bridge (row {br_diag}) in diagnostics"
        );
    }

    #[test]
    fn diagnostics_at_160_cols_shows_full_gpu_name_storage_values_battery_kv_and_alert_log_text() {
        let mut app = nominal_app();
        app.screen = Screen::Diagnostics;
        if let Reading::Available(gpus) = &mut app.snapshot.as_mut().unwrap().gpus {
            gpus[0].name = "NVIDIA GeForce RTX 4060 Laptop GPU".into();
            gpus[0].pci_slot = "0000:01:00.0".into();
            gpus[0].memory_used_bytes = Reading::Available(485_597_184); // 463.1 MiB
            gpus[0].memory_total_bytes = Reading::Available(536_870_912); // 512.0 MiB
        }
        if let Reading::Available(bat) = &mut app.snapshot.as_mut().unwrap().battery {
            bat.cycle_count = None;
        }

        let text = buffer_text(&render(&app, 160, 48));
        assert!(
            text.contains("NVIDIA GeForce RTX 4060 Laptop GPU"),
            "full GPU name must be visible at 160 cols:\n{text}"
        );
        assert!(text.contains("0000:01:00.0"), "{text}");
        assert!(text.contains("463.1 MiB/512.0 MiB"), "{text}");
        assert!(text.contains("280.0 GiB/500.0 GiB"), "{text}");
        assert!(text.contains("STATUS"), "{text}");
        assert!(text.contains("discharging"), "{text}");
        assert!(text.contains(" disconnected"), "{text}");
        assert!(text.contains("TIME LEFT"), "{text}");
        assert!(text.contains("CYCLES"), "{text}");
        assert!(text.contains("unavailable (not reported)"), "{text}");
        assert!(
            text.contains("no level changes since start"),
            "expected empty alert log message:\n{text}"
        );
    }
}
