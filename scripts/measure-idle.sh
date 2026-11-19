#!/usr/bin/env bash
# Measures nervtop's own CPU and memory use while it runs untouched in a real
# terminal (a detached tmux session), the way it sits on a desktop all day.
# CPU time comes from /proc/<pid>/stat (utime + stime), so the figure includes
# every thread: sampling, drawing and input.
#
# Usage: scripts/measure-idle.sh [path/to/nervtop] [seconds] [interval_ms] [WxH]
set -euo pipefail

bin="${1:-$(cd "$(dirname "$0")/.." && pwd)/target/release/nervtop}"
seconds="${2:-60}"
interval="${3:-1000}"
size="${4:-160x48}"
session="nervtop-idle-$$"

command -v tmux >/dev/null || { echo "tmux is required" >&2; exit 1; }
[ -x "$bin" ] || { echo "nervtop binary not found at $bin" >&2; exit 1; }

cols="${size%x*}"; rows="${size#*x}"
# The binary and its arguments go to tmux as separate words, so tmux runs
# nervtop directly (no shell) and a path with spaces stays one argument.
# That also makes the pane's process nervtop itself, so its PID comes from
# this session and never from another nervtop that happens to be running.
trap 'tmux kill-session -t "$session" 2>/dev/null || true' EXIT
pid=$(tmux new-session -d -P -F '#{pane_pid}' -s "$session" -x "$cols" -y "$rows" \
  "$bin" --interval "$interval")
sleep 5  # let it start and warm up
[ "$(readlink -f "/proc/$pid/exe" || true)" = "$(readlink -f "$bin")" ] ||
  { echo "pane process $pid is not running $bin" >&2; exit 1; }

ticks=$(getconf CLK_TCK)
cpu_ticks() { awk '{print $14 + $15}' "/proc/$pid/stat"; }

t0=$(date +%s.%N); c0=$(cpu_ticks)
sleep "$seconds"
t1=$(date +%s.%N); c1=$(cpu_ticks)
rss_kib=$(awk '/VmRSS/ {print $2}' "/proc/$pid/status")
threads=$(awk '/Threads/ {print $2}' "/proc/$pid/status")

awk -v c0="$c0" -v c1="$c1" -v t0="$t0" -v t1="$t1" -v hz="$ticks" \
    -v rss="$rss_kib" -v th="$threads" -v iv="$interval" -v sz="$size" 'BEGIN {
  wall = t1 - t0; cpu = (c1 - c0) / hz
  printf "interval %d ms, terminal %s, %.1f s wall\n", iv, sz, wall
  printf "cpu time %.2f s = %.2f%% of one core (average)\n", cpu, 100 * cpu / wall
  printf "resident memory %.1f MiB, %d threads\n", rss / 1024, th
}'
