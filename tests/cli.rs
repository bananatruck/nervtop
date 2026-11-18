//! Runs the built binary the way a user would.

use std::process::{Command, Output};

fn nervtop(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nervtop"))
        .args(args)
        .env("XDG_CONFIG_HOME", env!("CARGO_TARGET_TMPDIR"))
        .output()
        .expect("run nervtop")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn print_config_prints_the_documented_defaults() {
    let o = nervtop(&["--print-config"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), nervtop::config::DEFAULT_TOML);
}

#[test]
fn version_and_help() {
    let v = nervtop(&["--version"]);
    assert!(v.status.success());
    assert_eq!(
        stdout(&v).trim(),
        format!("nervtop {}", env!("CARGO_PKG_VERSION"))
    );
    let h = nervtop(&["--help"]);
    assert!(h.status.success());
    let text = stdout(&h);
    for flag in [
        "--config",
        "--interval",
        "--theme",
        "--print-config",
        "--once",
        "--json",
        "--screenshot",
        "--size",
        "--screen",
    ] {
        assert!(text.contains(flag), "help is missing {flag}:\n{text}");
    }
}

#[test]
fn bad_flags_and_bad_config_exit_with_code_2_before_touching_the_terminal() {
    let o = nervtop(&["--frobnicate"]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains("--frobnicate"));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "[general]\ninterval_ms = 5\n").unwrap();
    let o = nervtop(&["--config", path.to_str().unwrap(), "--once"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("interval_ms"), "{}", stderr(&o));
    assert!(stdout(&o).is_empty());
}

#[test]
fn once_reports_every_section_from_the_real_machine() {
    let o = nervtop(&["--once", "--interval", "250"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let text = stdout(&o).to_lowercase();
    for section in [
        "cpu",
        "memory",
        "disk",
        "network",
        "process",
        "temperature",
        "gpu",
        "battery",
    ] {
        assert!(text.contains(section), "report lacks {section}:\n{text}");
    }
}

#[test]
fn once_json_is_valid_json_with_snapshot_rules_and_consensus() {
    let o = nervtop(&["--once", "--json", "--interval", "250"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("valid JSON");
    assert!(v.get("snapshot").is_some());
    assert!(v.get("rules").is_some());
    assert!(v.get("consensus").is_some());
    let cpu = &v["snapshot"]["cpu"];
    assert!(cpu["status"] == "available" || cpu["status"] == "unavailable");
}

#[test]
fn screenshot_writes_an_svg_of_the_requested_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shot.svg");
    let o = nervtop(&[
        "--screenshot",
        path.to_str().unwrap(),
        "--size",
        "80x24",
        "--screen",
        "processes",
        "--interval",
        "250",
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let svg = std::fs::read_to_string(&path).unwrap();
    assert!(svg.starts_with("<svg"), "{}", &svg[..svg.len().min(200)]);
    assert!(svg.contains("PROCESSES"));
}

#[test]
fn a_closed_stdout_is_not_a_crash() {
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_nervtop"))
        .args(["--once", "--json", "--interval", "250"])
        .env("XDG_CONFIG_HOME", env!("CARGO_TARGET_TMPDIR"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run nervtop");
    // Close the read end before nervtop writes, like `nervtop --once | head -0`.
    drop(child.stdout.take());
    let out = child.wait_with_output().expect("wait");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("panicked"), "{err}");
    assert_eq!(out.status.code(), Some(0), "{err}");
}

#[test]
fn a_non_utf8_argument_is_a_usage_error_not_a_panic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let o = Command::new(env!("CARGO_BIN_EXE_nervtop"))
        .arg("--config")
        .arg(OsString::from_vec(vec![0x66, 0xff, 0x6f]))
        .output()
        .expect("run nervtop");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!err.contains("panicked"), "{err}");
    assert_eq!(o.status.code(), Some(2), "{err}");
}
