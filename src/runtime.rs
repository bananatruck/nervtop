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

fn collect_loop(
    mut collector: Collector,
    mut interval: u64,
    controls: Receiver<Control>,
    events: Sender<AppEvent>,
) {
    collector.sample();
    let warm_until = Instant::now() + Duration::from_millis(250);
    let mut warming = true;
    let mut delay = Duration::from_millis(250);
    let mut paused = false;
    loop {
        let control = if paused {
            controls.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            controls.recv_timeout(if warming {
                warm_until.saturating_duration_since(Instant::now())
            } else {
                delay
            })
        };
        match control {
            Ok(Control::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Control::Interval(ms)) => {
                interval = ms;
                delay = Duration::ZERO;
            }
            Ok(Control::Pause(p)) => {
                paused = p;
                delay = Duration::ZERO;
            }
            Err(RecvTimeoutError::Timeout) => {
                warming = false;
                if events
                    .send(AppEvent::Snapshot(Box::new(collector.sample())))
                    .is_err()
                {
                    break;
                }
                delay = Duration::from_millis(interval);
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
struct CollectorExit(Sender<AppEvent>);
impl Drop for CollectorExit {
    fn drop(&mut self) {
        let _ = self.0.send(AppEvent::CollectorStopped);
    }
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
        loop {
            let event = events_rx
                .recv()
                .map_err(|_| io::Error::other("collector stopped"))?;
            match step(&mut app, event)? {
                Step::Idle => continue,
                Step::Quit => break,
                Step::Interval(ms) => control_tx
                    .send(Control::Interval(ms))
                    .map_err(|_| io::Error::other("collector stopped"))?,
                Step::Pause(p) => control_tx
                    .send(Control::Pause(p))
                    .map_err(|_| io::Error::other("collector stopped"))?,
                Step::Draw => (),
            }
            draw(&mut terminal, &mut app)?;
        }
        Ok(())
    })();
    let _ = control_tx.send(Control::Quit);
    signal_handle.close();
    drop(restore);
    let _ = collector_thread.join();
    let _ = signal_thread.join();
    result
}
fn draw(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    terminal.draw(|frame| {
        app.set_page_size(usize::from(frame.area().height.saturating_sub(8).max(1)));
        crate::ui::draw(frame, app);
    })?;
    Ok(())
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
    #[test]
    fn collector_warms_pauses_resumes_and_changes_interval() {
        let (tx, rx) = mpsc::channel();
        let (events, received) = mpsc::channel();
        let started = Instant::now();
        let worker = thread::spawn(move || collect_loop(collector(), 10000, rx, events));
        // An early interval change must not bypass CPU warm-up.
        tx.send(Control::Interval(10000)).unwrap();
        let first = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(250));
        assert!(matches!(first,AppEvent::Snapshot(s) if s.seq==2));
        tx.send(Control::Pause(true)).unwrap();
        tx.send(Control::Interval(250)).unwrap();
        assert!(matches!(
            received.recv_timeout(Duration::from_millis(350)),
            Err(RecvTimeoutError::Timeout)
        ));
        tx.send(Control::Pause(false)).unwrap();
        assert!(
            matches!(received.recv_timeout(Duration::from_secs(5)).unwrap(),AppEvent::Snapshot(s) if s.seq==3)
        );
        tx.send(Control::Quit).unwrap();
        worker.join().unwrap();
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
