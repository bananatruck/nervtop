use std::process::ExitCode;

fn main() -> ExitCode {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--print-config") => {
            print!("{}", nervtop::config::DEFAULT_TOML);
            ExitCode::SUCCESS
        }
        Some("-V") | Some("--version") => {
            println!("nervtop {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("nervtop: the monitor is not wired up yet; try --print-config");
            ExitCode::from(2)
        }
    }
}
