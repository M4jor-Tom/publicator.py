#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d /tmp/publicator-native.XXXXXX)
xvfb= wm= vnc= bridge= app=
cleanup() {
  local pids=("$bridge" "$vnc" "$app" "$wm" "$xvfb")
  for pid in "${pids[@]}"; do
    [[ -n "$pid" ]] && kill "$pid" 2>/dev/null || true
  done
  sleep .2
  for pid in "${pids[@]}"; do
    [[ -n "$pid" ]] && kill -KILL "$pid" 2>/dev/null || true
  done
  for pid in "${pids[@]}"; do
    [[ -n "$pid" ]] && wait "$pid" 2>/dev/null || true
  done
  printf 'cleanup: artifacts=%s\n' "$work"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

port() {
  python -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}
ready() {
  local name=$1; shift
  for _ in $(seq 1 100); do
    if "$@" >/dev/null 2>&1; then return 0; fi
    sleep .1
  done
  echo "readiness failed: $name" >&2
  return 1
}

cd "$repo"
nix develop -c cargo build --example ui_acceptance --features desktop --quiet
Xvfb -displayfd 3 -screen 0 1440x1000x24 -nolisten tcp -ac 3>"$work/display" >"$work/xvfb.log" 2>&1 &
xvfb=$!
ready Xvfb-display test -s "$work/display"
export DISPLAY=":$(cat "$work/display")"
unset WAYLAND_DISPLAY
export XDG_SESSION_TYPE=x11
ready X-server xdpyinfo -display "$DISPLAY"
openbox --sm-disable >"$work/openbox.log" 2>&1 &
wm=$!
ready window-manager wmctrl -m

vnc_port=$(port)
web_port=$(port)
x11vnc -localhost -forever -shared -nopw -display "$DISPLAY" -rfbport "$vnc_port" >"$work/x11vnc.log" 2>&1 &
vnc=$!
ready x11vnc-port python -c "import socket; s=socket.create_connection(('127.0.0.1',$vnc_port),timeout=.2); s.close()"
websockify --web "$NOVNC_WEB" \
  "127.0.0.1:$web_port" "127.0.0.1:$vnc_port" >"$work/websockify.log" 2>&1 &
bridge=$!
ready noVNC-http curl --fail --silent "http://127.0.0.1:$web_port/vnc.html"

printf 'noVNC: http://127.0.0.1:%s/vnc.html?autoconnect=1&resize=scale\nartifacts: %s\n' "$web_port" "$work"
for scenario in empty history mixed reopen; do
  fixture_scenario=$scenario
  [[ "$scenario" == reopen ]] && fixture_scenario=mixed
  fixture="$work/$fixture_scenario"
  mkdir -p "$fixture"
  "$repo/target/debug/examples/ui_acceptance" \
    --fixture-dir "$fixture" --scenario "$scenario" >"$work/$scenario.stdout" 2>"$work/$scenario.stderr" &
  app=$!
  ready "$scenario native window" bash -c \
    "for id in \$(xdotool search --onlyvisible --name 'Publicator' 2>/dev/null); do test -n \"\$id\" && exit 0; done; exit 1"
  wait "$app"
  app=
done
