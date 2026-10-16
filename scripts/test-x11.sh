#!/usr/bin/env bash
# Run inside xvfb-run so these focus/input checks cannot affect a real desktop.
set -euo pipefail
unset WAYLAND_DISPLAY WAYLAND_SOCKET SWAYSOCK

diagnostics=${XWINDOW_TEST_DIAGNOSTICS:-target/desktop-x11}
mkdir -p "$diagnostics"
openbox >"$diagnostics/openbox.log" 2>&1 &
wm_pid=$!
trap 'kill "$wm_pid" 2>/dev/null || true; wait "$wm_pid" 2>/dev/null || true' EXIT

# Openbox must own the root before the test maps windows or requests focus.
# A fixed sleep can expire before the window manager finishes starting.
wm_ready() {
  kill -0 "$wm_pid" 2>/dev/null &&
    xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null |
      grep -Eq 'window id # 0x[0-9a-fA-F]+$'
}
for ((attempt = 0; attempt < 100; attempt++)); do
  if wm_ready; then break; fi
  if ! kill -0 "$wm_pid" 2>/dev/null; then break; fi
  sleep 0.1
done
if ! wm_ready; then
  echo 'Openbox did not become ready' >&2
  cat "$diagnostics/openbox.log" >&2
  exit 1
fi

# The same focus events must reach the adapter in both integration modes.
for external in 0 1; do
  XWINDOW_DESKTOP=1 XWINDOW_EXTERNAL_PUMP=$external XWINDOW_DESKTOP_TRACE=1 \
    cargo test --locked -p xwindow-check --test desktop 2>&1 |
    tee "$diagnostics/desktop-$external.log"
done
