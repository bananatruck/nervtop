//! Blocking event loop and collector controls.
use crate::{
    app::{App, Effect},
    collect::Collector,
    model::Snapshot,
};
use crossterm::event::{Event, KeyEvent};
use std::{
    io,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub enum AppEvent {
    Snapshot(Box<Snapshot>),
    Key(KeyEvent),
    Resize,
    Quit,
    CollectorStopped,
    InputError(io::Error),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Idle,
    Draw,
    Quit,
    Interval(u64),
    Pause(bool),
}
pub fn step(app: &mut App, event: AppEvent) -> io::Result<Step> {
    Ok(match event {
        AppEvent::Snapshot(s) => {
            if app.paused {
                Step::Idle
            } else {
                app.apply_snapshot(*s);
                Step::Draw
            }
        }
        AppEvent::Key(k) => match app.handle_key(k) {
            Effect::None => Step::Idle,
            Effect::Redraw => Step::Draw,
            Effect::Quit => Step::Quit,
            Effect::IntervalChanged(ms) => Step::Interval(ms),
            Effect::PauseChanged(p) => Step::Pause(p),
        },
        AppEvent::Resize => Step::Draw,
        AppEvent::Quit => {
            app.should_quit = true;
            Step::Quit
        }
        AppEvent::CollectorStopped => return Err(io::Error::other("collector stopped")),
        AppEvent::InputError(e) => return Err(e),
    })
}
#[derive(Debug)]
enum Control {
    Interval(u64),
    Pause(bool),
    Quit,
}

/// Gap between a baseline sample and the first published one, at start and
/// after a resume, so rate counters cover a short fresh window.
const BASELINE: Duration = Duration::from_millis(250);

fn collect_loop(
    mut collector: Collector,
    mut interval: u64,
    controls: Receiver<Control>,
    events: Sender<AppEvent>,
) {
    // The first sample only sets the counter baselines (CPU, disk, network
    // rates) and is not published.
    collector.sample();
    let mut next = Instant::now() + BASELINE;
    // True while waiting to publish the first sample after a baseline; an
    // interval change then must not push that sample out.
    let mut after_baseline = true;
    let mut paused = false;
    loop {
        let control = if paused {
            // Paused blocks without polling.
            controls.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            controls.recv_timeout(next.saturating_duration_since(Instant::now()))
        };
        match control {
            Ok(Control::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Control::Interval(ms)) => {
                interval = ms;
                if !after_baseline {
                    next = Instant::now() + Duration::from_millis(interval);
                }
            }
            Ok(Control::Pause(true)) => paused = true,
            Ok(Control::Pause(false)) => {
                if paused {
                    paused = false;
                    // Refresh the baselines so the next rates do not average
                    // over the whole paused gap; forget them first, so a
                    // counter that fails to read now cannot keep its
                    // pre-pause value.
                    collector.restart();
                    collector.sample();
                    next = Instant::now() + BASELINE;
                    after_baseline = true;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if events
                    .send(AppEvent::Snapshot(Box::new(collector.sample())))
                    .is_err()
                {
                    break;
                }
                after_baseline = false;
                next = Instant::now() + Duration::from_millis(interval);
            }
        }
    }
}

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        ratatui::restore();
    }
}
/// Ends the main loop when the input thread ends for any reason, a panic
/// inside `crossterm::event::read` included, so the main loop never waits on
/// input that will not come. When the main loop is already gone the send
/// fails and that is fine.
struct InputExit(Sender<AppEvent>);
impl Drop for InputExit {
    fn drop(&mut self) {
        let _ = self.0.send(AppEvent::Quit);
    }
}
struct CollectorExit(Sender<AppEvent>);
impl Drop for CollectorExit {
    fn drop(&mut self) {
        let _ = self.0.send(AppEvent::CollectorStopped);
    }
}

/// Waits at most `within` for the collector thread to finish after it was
/// told to quit, and joins it if it did. A collector stuck inside `sample()`
/// is left behind (returning `false`) so quitting never hangs on it; the
/// process exits without it.
fn wait_for_collector(collector: thread::JoinHandle<()>, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while !collector.is_finished() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    collector.join().is_ok()
}

pub fn run(collector: Collector, mut app: App) -> io::Result<()> {
    use signal_hook::{
        consts::{SIGHUP, SIGINT, SIGTERM},
        iterator::Signals,
    };
    let mut signals = Signals::new([SIGTERM, SIGHUP, SIGINT])?;
    let signal_handle = signals.handle();
    let (events_tx, events_rx) = mpsc::channel();
    let (control_tx, control_rx) = mpsc::channel();
    if app.paused {
        let _ = control_tx.send(Control::Pause(true));
    }
    let collector_tx = events_tx.clone();
    let interval = app.interval_ms;
    let collector_thread = thread::spawn(move || {
        let _exit = CollectorExit(collector_tx.clone());
        collect_loop(collector, interval, control_rx, collector_tx);
    });
    let input_tx = events_tx.clone();
    thread::spawn(move || {
        let _exit = InputExit(input_tx.clone());
        loop {
            let event = match crossterm::event::read() {
                Ok(Event::Key(k)) => AppEvent::Key(k),
                Ok(Event::Resize(_, _)) => AppEvent::Resize,
                Ok(_) => continue,
                Err(e) => {
                    let _ = input_tx.send(AppEvent::InputError(e));
                    break;
                }
            };
            if input_tx.send(event).is_err() {
                break;
            }
        }
    });
    let signal_thread = thread::spawn(move || {
        if signals.forever().next().is_some() {
            let _ = events_tx.send(AppEvent::Quit);
        }
    });
    let restore = Restore;
    let mut terminal = ratatui::init();
    let result = (|| {
        // Paint the frame immediately; panels say "acquiring" until the
        // warm-up sample arrives about 250 ms later.
        draw(&mut terminal, &mut app)?;
        'events: loop {
            let first = events_rx
                .recv()
                .map_err(|_| io::Error::other("collector stopped"))?;
            let mut redraw = false;
            for event in drain_ready(first, &events_rx) {
                match step(&mut app, event)? {
                    Step::Idle => continue,
                    Step::Quit => break 'events,
                    Step::Interval(ms) => control_tx
                        .send(Control::Interval(ms))
                        .map_err(|_| io::Error::other("collector stopped"))?,
                    Step::Pause(p) => control_tx
                        .send(Control::Pause(p))
                        .map_err(|_| io::Error::other("collector stopped"))?,
                    Step::Draw => (),
                }
                redraw = true;
            }
            if redraw {
                draw(&mut terminal, &mut app)?;
            }
        }
        Ok(())
    })();
    let _ = control_tx.send(Control::Quit);
    drop(restore);
    // Drop the signal handlers first so a second Ctrl-C works normally even
    // if the collector is stuck.
    signal_handle.close();
    let _ = signal_thread.join();
    wait_for_collector(collector_thread, Duration::from_secs(1));
    result
}
fn draw(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    terminal.draw(|frame| {
        crate::ui::prepare_frame(app, frame.area());
        crate::ui::draw(frame, app);
    })?;
    Ok(())
}

/// `first` plus every event already waiting, so a burst of snapshots (for
/// example while the terminal is slow to draw) costs one redraw, not one each.
fn drain_ready(first: AppEvent, rx: &Receiver<AppEvent>) -> Vec<AppEvent> {
    let mut batch = vec![first];
    batch.extend(rx.try_iter());
    batch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        collect::{HostSource, Source, Sources},
        model::*,
    };
    use std::marker::PhantomData;
    struct Missing<T>(PhantomData<T>);
    impl<T: Send> Source for Missing<T> {
        type Output = T;
        fn sample(&mut self) -> Reading<T> {
            Reading::unavailable("test source")
        }
    }
    fn missing<T: Send + 'static>() -> Box<dyn Source<Output = T>> {
        Box::new(Missing(PhantomData))
    }
    struct Host;
    impl HostSource for Host {
        fn sample(&mut self) -> HostInfo {
            HostInfo {
                hostname: Reading::unavailable("test host"),
                kernel: Reading::unavailable("test kernel"),
                uptime_secs: Reading::unavailable("test uptime"),
            }
        }
    }
    fn collector() -> Collector {
        Collector::new(Sources {
            host: Box::new(Host),
            cpu: missing(),
            memory: missing(),
            disks: missing(),
            network: missing(),
            processes: missing(),
            sensors: missing(),
            gpus: missing(),
            battery: missing(),
        })
    }
    /// Records, in one log shared with the test, each sample and restart.
    struct Recording(std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>);
    impl Source for Recording {
        type Output = Vec<NetInterface>;
        fn sample(&mut self) -> Reading<Self::Output> {
            self.0.lock().unwrap().push("sample");
            Reading::unavailable("test source")
        }
        fn restart(&mut self) {
            self.0.lock().unwrap().push("restart");
        }
    }

    #[test]
    fn resume_restarts_rate_baselines_before_its_unpublished_sample() {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut sources = collector();
        sources.replace_network(Box::new(Recording(log.clone())));
        let (tx, rx) = mpsc::channel();
        let (events, received) = mpsc::channel();
        let worker = thread::spawn(move || collect_loop(sources, 10000, rx, events));
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        tx.send(Control::Pause(true)).unwrap();
        tx.send(Control::Pause(false)).unwrap();
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        tx.send(Control::Quit).unwrap();
        worker.join().unwrap();
        // Start: baseline sample, published sample. Resume: restart, baseline
        // sample, published sample. No restart anywhere else.
        assert_eq!(
            *log.lock().unwrap(),
            ["sample", "sample", "restart", "sample", "sample"]
        );
    }

    #[test]
    fn collector_warms_pauses_resumes_and_changes_interval() {
        // Every collector.sample() bumps seq, so seq counts samples taken,
        // published or not.
        let seq = |event: AppEvent| match event {
            AppEvent::Snapshot(s) => s.seq,
            other => panic!("expected a snapshot, got {other:?}"),
        };
        let (tx, rx) = mpsc::channel();
        let (events, received) = mpsc::channel();
        let started = Instant::now();
        let worker = thread::spawn(move || collect_loop(collector(), 10000, rx, events));
        // An early interval change must not bypass CPU warm-up.
        tx.send(Control::Interval(10000)).unwrap();
        let first = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(250));
        assert_eq!(seq(first), 2);
        // An interval change publishes nothing at once; the next sample comes
        // one full new interval later.
        let changed = Instant::now();
        tx.send(Control::Interval(400)).unwrap();
        assert!(matches!(
            received.recv_timeout(Duration::from_millis(300)),
            Err(RecvTimeoutError::Timeout)
        ));
        assert_eq!(
            seq(received.recv_timeout(Duration::from_secs(5)).unwrap()),
            3
        );
        assert!(changed.elapsed() >= Duration::from_millis(400));
        // Paused: nothing is sampled, whatever the interval does.
        tx.send(Control::Pause(true)).unwrap();
        tx.send(Control::Interval(250)).unwrap();
        assert!(matches!(
            received.recv_timeout(Duration::from_millis(350)),
            Err(RecvTimeoutError::Timeout)
        ));
        // Resume: one sample refreshes the counter baselines and is dropped,
        // and exactly one snapshot is published 250 ms later.
        let resumed = Instant::now();
        tx.send(Control::Pause(false)).unwrap();
        assert_eq!(
            seq(received.recv_timeout(Duration::from_secs(5)).unwrap()),
            5
        );
        assert!(resumed.elapsed() >= Duration::from_millis(250));
        assert!(matches!(
            received.recv_timeout(Duration::from_millis(150)),
            Err(RecvTimeoutError::Timeout)
        ));
        // Then sampling continues at the interval.
        assert_eq!(
            seq(received.recv_timeout(Duration::from_secs(5)).unwrap()),
            6
        );
        tx.send(Control::Quit).unwrap();
        worker.join().unwrap();
    }
    #[test]
    fn stopping_a_stuck_collector_is_bounded() {
        let (control, _ignored) = mpsc::channel();
        // A collector stuck inside sample() never reads its controls.
        let stuck = thread::spawn(|| {
            loop {
                thread::park();
            }
        });
        let (done, finished) = mpsc::channel();
        thread::spawn(move || {
            let start = Instant::now();
            let _ = control.send(Control::Quit);
            let stopped = wait_for_collector(stuck, Duration::from_secs(1));
            let _ = done.send((stopped, start.elapsed()));
        });
        let (stopped, elapsed) = finished
            .recv_timeout(Duration::from_secs(3))
            .expect("quit must not wait on a stuck collector");
        assert!(!stopped);
        assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
    }
    #[test]
    fn stopping_a_healthy_collector_joins_it() {
        let (control, controls) = mpsc::channel();
        let (events, _received) = mpsc::channel();
        let worker = thread::spawn(move || collect_loop(collector(), 10000, controls, events));
        let start = Instant::now();
        control.send(Control::Quit).unwrap();
        assert!(wait_for_collector(worker, Duration::from_secs(1)));
        assert!(start.elapsed() < Duration::from_millis(500));
    }
    #[test]
    fn queued_events_are_handled_before_one_redraw() {
        let (tx, rx) = mpsc::channel();
        for _ in 0..3 {
            tx.send(AppEvent::Resize).unwrap();
        }
        tx.send(AppEvent::Quit).unwrap();
        let batch = drain_ready(AppEvent::Resize, &rx);
        assert_eq!(
            batch.len(),
            5,
            "the first event plus everything already queued"
        );
        assert!(matches!(batch.last(), Some(AppEvent::Quit)));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn collector_control_disconnect_exits() {
        let (tx, rx) = mpsc::channel();
        let (events, _) = mpsc::channel();
        drop(tx);
        let worker = thread::spawn(move || collect_loop(collector(), 10000, rx, events));
        worker.join().unwrap();
    }
    #[test]
    fn collector_exit_notifies_even_during_panic() {
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _exit = CollectorExit(tx);
            panic!("test collector failure");
        });
        assert!(worker.join().is_err());
        assert!(matches!(rx.recv().unwrap(), AppEvent::CollectorStopped));
    }
    #[test]
    fn input_exit_sends_quit_even_during_panic() {
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _exit = InputExit(tx);
            panic!("test input failure");
        });
        assert!(worker.join().is_err());
        assert!(matches!(rx.recv().unwrap(), AppEvent::Quit));
        // With the main loop gone the failed send is ignored, not a panic.
        let (tx, rx) = mpsc::channel();
        drop(rx);
        drop(InputExit(tx));
    }
    #[test]
    #[ignore = "requires a terminal and permission for local socket sends; run alone with --ignored --exact runtime::tests::terminal_restored_after_sigterm"]
    fn terminal_restored_after_sigterm() {
        let config = crate::config::Config::default();
        let palette = config.palette().unwrap();
        let sender = thread::spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !crossterm::terminal::is_raw_mode_enabled().unwrap() {
                assert!(Instant::now() < deadline, "terminal never entered raw mode");
                thread::sleep(Duration::from_millis(10));
            }
            // Give the warmed sample time to traverse the real event loop.
            thread::sleep(Duration::from_millis(350));
            signal_hook::low_level::raise(signal_hook::consts::SIGTERM).unwrap();
        });
        run(collector(), App::new(config, palette)).unwrap();
        sender.join().unwrap();
        assert!(!crossterm::terminal::is_raw_mode_enabled().unwrap());
    }
}
