# nervtop specification

nervtop is a terminal system monitor with a command-center look: hexagonal status cells, warning stripes, and an amber, red and green status language.
It is a companion to nerv-fieldnotes and shares its palette: charcoal green ground, amber headings, orange accents, pale green "nominal" text.
It is original work in the genre.
It ships no artwork, logos, fonts, audio or dialogue from any existing show.
It is read-only: it observes the machine and never kills, renices or changes anything.

## Goals

Show every important live metric of a Linux laptop on one screen that stays readable from 80x24 up to a full 4K terminal.
Every number on screen comes from a real source on the running machine.
A metric that cannot be read shows the word "unavailable" with a short reason, never a guessed or zero value.
Stay cheap enough to leave open all day: the event loop blocks while idle and redraws only when data or input arrives.
Restore the terminal on every exit path, including a panic and SIGTERM.

## Non-goals

nervtop is not a process manager, so it has no kill, signal or renice actions.
It does not run as root and does not ask for elevated permissions.
It does not support Windows or macOS; it targets Linux and reads `/proc` and `/sys` directly where `sysinfo` does not cover a metric.

## Screens

There are three screens, switched with `1`, `2`, `3` or `Tab`.

### 1 OVERVIEW

The header row shows the program name, hostname, kernel, uptime, local clock, sample interval, and a PAUSED tag when paused.
Below the header sits the consensus strip with three hexagonal judgement cells and the combined verdict.
When any alert is at CRITICAL, a warning-stripe banner spans the width under the header and names the worst alert.
The body holds these panels: CPU (per-core bars, total history sparkline, load average, frequency), MEMORY (RAM and swap gauges), GPU (one block per adapter), THERMAL (hottest sensors and fans), STORAGE (per-mount usage and I/O rates), NETWORK (per-interface rates and history), POWER (battery), PROCESSES (top consumers), and ALERTS (active alerts).
At 80x24 the overview drops the history sparklines and shows the most important rows of each panel; wider and taller terminals reveal more rows and history.

### 2 PROCESSES

A full-height process table with PID, user, CPU percent, memory, state and command name.
It supports sorting, reversing, filtering by name, and moving a selection.
The selection is kept on the same PID across refreshes when that process still exists.

### 3 DIAGNOSTICS

Every temperature sensor and fan, every GPU field, the full battery record, every disk and interface, the alert rule table with current values and thresholds, and the alert event log with timestamps.
This screen exists so that every reading the program takes can be inspected, including the ones the overview leaves out for space.

### Overlays and small terminals

`?` or `F1` opens a help overlay listing every key binding; any key closes it.
Below 80 columns or 24 rows nervtop draws a single centered message that states the current size and the minimum size, and it keeps sampling so data is ready when the terminal grows.

## Key bindings

| Key | Action |
| --- | --- |
| `q`, `Ctrl+C` | Quit |
| `?`, `F1` | Toggle help overlay |
| `1`, `2`, `3` | Overview, processes, diagnostics |
| `Tab`, `Shift+Tab` | Next, previous screen |
| `Space` | Pause or resume sampling (the screen keeps the last sample) |
| `+`, `-` | Lengthen or shorten the sample interval within 250 ms to 10 s |
| `t` | Cycle built-in themes for this session |
| `s` | Cycle the process sort key: CPU, memory, PID, name |
| `r` | Reverse the sort order |
| `/` | Start a process name filter; `Enter` keeps it, `Esc` clears it |
| `j`, `k`, arrows | Move the process selection |
| `PgUp`, `PgDn`, `g`, `G` | Page, top and bottom of the process list |
| `a` | Acknowledge current alerts, which stops the banner from flashing until a new alert fires |

## Data sources and fallbacks

Every source implements one trait so it can be swapped for a fake in tests.
Each sample returns `Reading::Available(value)` or `Reading::Unavailable(reason)`.

| Metric | Primary source | Fallback or unavailable rule |
| --- | --- | --- |
| CPU per core and total | `sysinfo` CPU usage | Unavailable on the first sample until two readings exist ("warming up") |
| Load average | `sysinfo::System::load_average` | Always available on Linux |
| CPU frequency | `sysinfo` per-core frequency | Hidden when every core reports 0 MHz |
| Memory and swap | `sysinfo` memory | Swap shows "none configured" when total swap is 0 |
| Disks | `sysinfo::Disks` for space, `/proc/diskstats` for read and write rates | Rates are unavailable until two samples exist; pseudo and duplicate filesystems are hidden |
| Network | `sysinfo::Networks` byte counters | Rates unavailable until two samples exist; loopback is hidden |
| Processes | `sysinfo` processes with CPU and memory refresh only | Users resolve through `sysinfo::Users`; unknown UIDs show the number |
| Temperatures | `/sys/class/hwmon/*/temp*_input` with labels and `crit` values | Unavailable when no readable sensor exists |
| Fans | `/sys/class/hwmon/*/fan*_input` | Section hidden when the machine exposes no fans |
| AMD GPU | `/sys/class/drm/card*/device/gpu_busy_percent`, `mem_info_vram_*`, hwmon temperature and power | Per-field unavailable when a file is missing |
| NVIDIA GPU | `nvidia-smi --query-gpu` CSV with a 2 s timeout, run in a sample only when every NVIDIA display adapter's `power/runtime_status` is `active`, `unsupported` (runtime PM disabled, so always powered) or missing; a killed nvidia-smi that cannot be reaped within 200 ms is left to a background reaper so a sample is never blocked by it | A `suspended` or `suspending` adapter shows SUSPENDED with every field "gpu suspended"; a `resuming` adapter shows "gpu resuming" and an `error` adapter "gpu runtime pm error"; while any NVIDIA adapter is in one of those states nvidia-smi is not run at all, because it enumerates and would wake every NVIDIA device, and the awake NVIDIA adapters show "not queried: another nvidia gpu is asleep"; when nvidia-smi is missing or fails the GPU shows unavailable with the reason |
| Battery | `/sys/class/power_supply/BAT*` capacity, status, energy or charge, power or current, charge limit | Time remaining only when the discharge or charge rate is known and non-zero; no battery shows "no battery present" |
| Host | hostname, kernel release and uptime from `sysinfo` | Unknown fields show unavailable |

NVIDIA queries are rate limited to at most one every 2 seconds, whatever the UI interval, because each call costs tens of milliseconds of CPU.
The PCI vendor ID `0x10de` identifies NVIDIA and `0x1002` identifies AMD.

## Alert rules

Each rule has a `warn` and a `crit` threshold in the config file.
A rule with the direction "above" fires when the value is at or above the threshold; "below" fires at or below.
A rule escalates only after its condition holds for `sustain` consecutive samples (default 3), so a single spike does not flash the screen.
A rule de-escalates when the value moves back past the threshold by its `hysteresis` margin, so a value hovering on the line does not flap.
Each rule can be disabled with `enabled = false`.
A rule whose source is unavailable is UNKNOWN and never counts as nominal or as an alert.

| Rule | Direction | Default warn | Default crit | Hysteresis | Notes |
| --- | --- | --- | --- | --- | --- |
| `cpu_usage` percent | above | 85 | 95 | 2 | Total CPU |
| `load` per logical core | above | 1.5 | 3.0 | 0.1 | One-minute load average divided by the core count |
| `cpu_temp` degrees C | above | 85 | 95 | 3 | Hottest CPU package sensor (`k10temp`, `coretemp`, `zenpower`) |
| `gpu_temp` degrees C | above | 80 | 90 | 3 | Hottest GPU |
| `memory` percent used | above | 85 | 95 | 2 | Used means total minus available |
| `swap` percent used | above | 60 | 90 | 2 | Skipped when no swap exists |
| `disk` percent used | above | 90 | 95 | 1 | Each real mount separately |
| `storage_temp` degrees C | above | 65 | 75 | 3 | NVMe and drive sensors |
| `battery` percent | below | 20 | 10 | 2 | Only while discharging |

## Consensus panel

Three judgement cells each watch one domain and vote NOMINAL, CAUTION or ALERT, or NO DATA when every input is unknown.
COMPUTE watches `cpu_usage` and `load`.
GPU utilisation is shown but not judged, so a busy GPU during a game is not an alarm.
THERMAL watches `cpu_temp`, `gpu_temp` and `storage_temp`.
RESOURCE watches `memory`, `swap`, `disk` and `battery`.
A cell's vote is the worst level among its rules.
The verdict is the worst vote among the three cells, and the panel also states whether the vote was unanimous or split, for example "SPLIT 2-1".

## Theme system

Built-in themes: `nerv` (default, 24-bit colour, matches nerv-fieldnotes), `phosphor` (green monochrome CRT), and `ansi` (only the 16 terminal palette colours, so it follows a pywal or base16 terminal scheme).
The config file selects a theme and may override any of its slots with `#rrggbb` or an ANSI colour name.
Slots: `background`, `surface`, `border`, `text`, `muted`, `accent`, `heading`, `nominal`, `caution`, `alert`, `unknown`, `stripe_dark`.
`background = "reset"` leaves the terminal background unpainted so transparency and blur from the terminal show through.

## Configuration

The config file is TOML at `$XDG_CONFIG_HOME/nervtop/config.toml`, falling back to `~/.config/nervtop/config.toml`, or the path given with `--config`.
A missing file means defaults.
Unknown keys and invalid values are errors that name the key, printed before the terminal enters raw mode, with exit code 2.
`nervtop --print-config` prints the full default file with a comment on every key, and a test proves that this text parses to exactly the built-in defaults.

## Command line

| Flag | Meaning |
| --- | --- |
| `--config PATH` | Read this config file |
| `--interval MS` | Sample interval in milliseconds, 250 to 10000 |
| `--theme NAME` | Start with this theme |
| `--print-config` | Print the documented default config and exit |
| `--once` | Take two samples one interval apart, print a plain text report, and exit |
| `--json` | With `--once`, print the report as JSON |
| `--screenshot PATH` | Render the real UI with live data to an SVG file and exit |
| `--size WxH` | Terminal size for `--screenshot`, default 120x40 |
| `--screen NAME` | Screen for `--screenshot`: overview, processes, diagnostics |
| `-h`, `--help` and `-V`, `--version` | Help and version |

## Architecture

`collect` owns the sources and produces one `Snapshot` per tick on a background thread.
`alert` turns a snapshot plus thresholds into rule states and the consensus.
`app` holds all UI state and applies key events and snapshots; it has no terminal code.
`ui` draws an `&App` into a ratatui frame and holds no state of its own.
`main` wires a collector thread and an input thread into one channel and blocks on it, so the program sleeps between events.
`svg` converts a ratatui buffer into an SVG image for honest screenshots.

## Test plan

Parsers have unit tests over captured real-world text: battery sysfs in both energy and charge units, hwmon trees, `/proc/diskstats`, nvidia-smi CSV including `[N/A]` fields, and amdgpu sysfs.
Sysfs readers take a root directory so tests run against temporary trees.
Alert tests cover thresholds in both directions, sustain, hysteresis, unknown handling and consensus voting.
App tests cover every key binding, sort order, filtering, selection that follows a PID, pause, interval limits and screen cycling.
Render tests draw the real UI into ratatui's `TestBackend` with fake snapshots at 80x24, 120x40 and 200x60 and assert on the buffer: panel titles present, "unavailable" shown for missing sources, alert banner present only at CRITICAL, the too-small message below 80x24, and the help overlay text.
An integration test runs the built binary with `--print-config` and `--once` and checks the output.
CI runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` on every push.
