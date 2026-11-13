#!/usr/bin/env bash
# Rebuilds Hoista and runs everything together: the engine, the desktop app,
# the docs and the landing page. Ctrl-C stops the desktop app, docs and page;
# the engine keeps running, like it does after the app closes.
#
#   scripts/dev.sh                  rebuild, (re)start the engine, open the desktop app on it
#   scripts/dev.sh --demo           the same on the demo engine: Sam Rivera's example
#                                   projects, in ~/.hoista-demo, never your own state
#   scripts/dev.sh --demo --fresh   the demo from a fresh seed
#
# Each run rebuilds `hoist` and `hoistd` into target/debug, installs npm
# packages when package-lock.json changed, then starts the engine from that
# build, or restarts a running one so it picks up the new build. A restart
# waits for work under way to finish; nothing is killed.
#
# The engine keeps its state in ~/.hoista. For another state folder, point
# HOISTA_SOCKET at the socket inside it; the engine and app both use it:
#   HOISTA_SOCKET=/tmp/hoista-test-state/hoistd.sock scripts/dev.sh
#
# HOISTA_DEV_LOGS=FOLDER also keeps each part's output in FOLDER (web.log,
# docs.log, desktop.log). The engine's own logs are in the state folder's logs/.
#
# Webpage: http://localhost:3000 (apps/webpage)
# Docs:    http://localhost:3001 (apps/docs; live at docs.hoista.dev)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

usage="Usage: scripts/dev.sh [--demo [--fresh]]"
demo=""
fresh=()
case "${1:-}" in
  "") ;;
  --demo)
    demo="${HOISTA_DEMO_DIR:-$HOME/.hoista-demo}"
    case "${2:-}" in
      "") ;;
      --fresh) fresh=(--fresh) ;;
      *) echo "$usage" >&2; exit 2 ;;
    esac
    ;;
  *) echo "$usage" >&2; exit 2 ;;
esac

# The demo engine: Hoista's own engine on example data, answered locally. It
# stops with the rest, since it holds nothing of yours.
if [ -n "$demo" ]; then
  echo "Building hoist and hoista-demo..."
  cargo build --locked -p hoist -p hoista-demo
  export HOISTA_SOCKET="$demo/hoistd.sock"
fi

# The engine, rebuilt and running on that build.
if [ -z "$demo" ]; then
  echo "Building hoist and hoistd..."
  cargo build --locked -p hoist -p hoistd

  state_args=()
  if [ -n "${HOISTA_SOCKET:-}" ]; then
    state_args=(--state-dir "$(dirname "$HOISTA_SOCKET")")
  fi
  hoist="$root/target/debug/hoist"
  hoistd="$root/target/debug/hoistd"
  if status="$("$hoist" ${state_args[@]+"${state_args[@]}"} daemon status 2>/dev/null)"; then
    # A running engine restarts from the program it was started from: check
    # that's this build, or the restart would bring back the old one.
    pid="$(printf '%s' "$status" | sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' | head -n 1)"
    if [ -e "/proc/$pid/exe" ]; then
      running="$(readlink "/proc/$pid/exe")"
    else
      running="$(ps -o comm= -p "$pid" 2>/dev/null || true)"
    fi
    here="$(cd "$(dirname "$hoistd")" && pwd -P)/hoistd"
    there="$(cd "$(dirname "$running")" 2>/dev/null && pwd -P)/$(basename "$running")"
    if [ -n "$running" ] && [ "$there" != "$here" ]; then
      echo "The engine running now (pid $pid) is $running, not this build." >&2
      echo "Stop it with: kill $pid (it finishes work under way, then exits). If it starts at" >&2
      echo "login, run hoist service uninstall first. Then run this again." >&2
      exit 1
    fi
    echo "Restarting the engine on the new build..."
    "$hoist" ${state_args[@]+"${state_args[@]}"} daemon restart >/dev/null
  else
    echo "Starting the engine..."
    "$hoist" ${state_args[@]+"${state_args[@]}"} start >/dev/null
  fi
fi

# npm packages, when package-lock.json is newer than the last install.
if [ ! -f node_modules/.package-lock.json ] || [ package-lock.json -nt node_modules/.package-lock.json ]; then
  echo "Installing npm packages..."
  npm install --no-audit --no-fund
fi

docs_port=3001
web_port=3000

for port in "$docs_port" "$web_port"; do
  if lsof -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "Port $port is already in use. Stop what's running there and try again." >&2
    exit 1
  fi
done

# Each one runs in its own process group, so stopping it stops everything
# it started too.
set -m
pids=""
stop() {
  trap - INT TERM EXIT
  for pid in $pids; do
    kill -- "-$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
}
trap stop INT TERM EXIT

# Each one's output, with its name in front so the three can be told apart.
# With HOISTA_DEV_LOGS set to a folder, each one's output is also kept there
# as NAME.log, started fresh each run.
logs="${HOISTA_DEV_LOGS:-}"
if [ -n "$logs" ]; then
  mkdir -p "$logs"
fi
run() {
  local name="$1"
  shift
  local log=/dev/null
  if [ -n "$logs" ]; then
    log="$logs/$name.log"
    : > "$log"
  fi
  ("$@" 2>&1 | while IFS= read -r line; do
    printf '[%s] %s\n' "$name" "$line"
    printf '%s\n' "$line" >> "$log"
  done) &
  pids="$pids $!"
}

if [ -n "$demo" ]; then
  rm -f "$HOISTA_SOCKET"
  run demo "$root/target/debug/hoista-demo" --state-dir "$demo" ${fresh[@]+"${fresh[@]}"}
  # The desktop app needs the engine answering before it opens.
  for _ in $(seq 300); do
    [ -S "$HOISTA_SOCKET" ] && break
    sleep 0.1
  done
fi
run web python3 -m http.server "$web_port" --bind 127.0.0.1 --directory apps/webpage
run docs npm run -s docs:dev
run desktop npm --workspace @hoista/desktop run dev

echo "Webpage:      http://localhost:$web_port"
echo "Docs:         http://localhost:$docs_port (the first start takes a minute or two)"
if [ -n "$demo" ]; then
  echo "Engine:       the demo engine on Sam Rivera's example projects, state in $demo"
  echo "Desktop:      opens in its own window, on the demo (the first start compiles it, a few minutes)"
else
  echo "Engine:       running on this build ($("$hoist" --version 2>/dev/null || echo hoist)), state in ${HOISTA_SOCKET:+$(dirname "$HOISTA_SOCKET")}${HOISTA_SOCKET:-$HOME/.hoista}"
  echo "Desktop:      opens in its own window, on the engine at ${HOISTA_SOCKET:-$HOME/.hoista/hoistd.sock} (the first start compiles it, a few minutes)"
fi
echo "Press Ctrl-C to stop them all."

# Stops everything when any one of them exits. (macOS's bash has no `wait -n`.)
while true; do
  for pid in $pids; do
    if ! kill -0 "$pid" 2>/dev/null; then
      exit 0
    fi
  done
  sleep 1
done
