use nervtop::app::App;
use nervtop::cli::{self, Command, Options, USAGE};
use nervtop::collect::Collector;
use nervtop::config::{self, Config, DEFAULT_TOML};
use nervtop::{report, runtime, screenshot};
use std::process::ExitCode;

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nervtop: {e}\nTry 'nervtop --help'.");
            return ExitCode::from(2);
        }
    };
    let options = match command {
        Command::Help => {
            return emit(USAGE);
        }
        Command::Version => {
            return emit(&format!("nervtop {}\n", env!("CARGO_PKG_VERSION")));
        }
        Command::PrintConfig => {
            return emit(DEFAULT_TOML);
        }
        Command::Run(options) => options,
    };
    let app = match build_app(&options) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("nervtop: {e}");
            return ExitCode::from(2);
        }
    };
    match run(options, app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nervtop: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Loads and validates the config and applies command line overrides, all
/// before the terminal is touched, so errors print normally.
fn build_app(options: &Options) -> Result<App, config::ConfigError> {
    let mut config = Config::load(options.config.as_deref())?;
    if let Some(ms) = options.interval_ms {
        config.general.interval_ms = ms;
    }
    if let Some(theme) = options.theme {
        if theme != config.theme.name {
            // Colour overrides belong to the theme they were written for.
            config.theme.colors.clear();
        }
        config.theme.name = theme;
    }
    config.validate()?;
    let palette = config.palette()?;
    let mut app = App::new(config, palette);
    app.screen = options.screen;
    Ok(app)
}

fn run(options: Options, mut app: App) -> std::io::Result<()> {
    let mut collector = Collector::system();
    if options.once {
        let (snapshot, eval) = report::once(&mut collector, app.interval_ms, &app.config.alerts);
        let out = if options.json {
            report::json(&snapshot, &eval)
        } else {
            report::text(&snapshot, &eval)
        };
        return write_stdout(&format!("{out}\n"));
    }
    if let Some(path) = &options.screenshot {
        screenshot::collect(&mut collector, &mut app, options.samples);
        let (w, h) = options.size;
        std::fs::write(path, screenshot::svg(&mut app, w, h))?;
        return Ok(());
    }
    runtime::run(collector, app)
}

/// Writes `text` to stdout. A reader that closed the pipe early (for example
/// `nervtop --once | head`) is a normal end, not an error.
fn write_stdout(text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

fn emit(text: &str) -> ExitCode {
    match write_stdout(text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nervtop: {e}");
            ExitCode::FAILURE
        }
    }
}
