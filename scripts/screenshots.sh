#!/usr/bin/env bash
# Renders README screenshots from the real program with live data from this
# machine. nervtop draws its actual UI into ratatui's test backend after
# sampling the system, writes that buffer as SVG, and rsvg-convert turns the
# SVG into PNG. Nothing here is mocked: every value is what the machine
# reported at the time the script ran.
#
# Usage: scripts/screenshots.sh [path/to/nervtop]
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="${1:-$root/target/release/nervtop}"
out="$root/docs/screenshots"
mkdir -p "$out"

command -v rsvg-convert >/dev/null || { echo "rsvg-convert (librsvg) is required" >&2; exit 1; }
[ -x "$bin" ] || { echo "nervtop binary not found at $bin; build it first" >&2; exit 1; }

shot() {
  local name="$1" screen="$2" size="$3"; shift 3
  # 40 samples 500 ms apart: 20 s of real history for the sparklines and
  # enough samples for sustained alert levels.
  "$bin" --screenshot "$out/$name.svg" --screen "$screen" --size "$size" \
    --samples 40 --interval 500 "$@"
  rsvg-convert --zoom 1.5 "$out/$name.svg" -o "$out/$name.png"
  echo "wrote $out/$name.png ($screen, $size)"
}

shot overview overview 160x48
shot overview-80x24 overview 80x24
shot processes processes 120x36
shot diagnostics diagnostics 160x48
shot theme-phosphor overview 120x36 --theme phosphor
