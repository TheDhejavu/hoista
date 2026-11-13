#!/usr/bin/env bash
# Starts Hoista in Docker, from a copy of its repository:
#
#   ./deploy/server/start.sh
#
# On a server, give the HTTPS address it will answer on (a reverse proxy
# there sends it to port 8787):
#
#   HOISTA_PUBLIC_URL=https://hoista.example.com ./deploy/server/start.sh
#
# It builds Hoista, makes its key the first time (kept in deploy/server/.env,
# readable only by you), starts it, and prints the claim code you make it
# yours with. Running it again rebuilds and restarts it; its data stays.
set -euo pipefail
cd "$(dirname "$0")"

port=8787
# How long to wait for it to start: tries, two seconds apart.
tries=90

if ! command -v docker >/dev/null 2>&1; then
  echo "Docker isn't installed. Install Docker Desktop, OrbStack or Colima, then run this again." >&2
  exit 1
fi
if ! docker info >/dev/null 2>&1; then
  echo "Docker isn't running. Start Docker Desktop or OrbStack, or run: colima start --memory 6" >&2
  exit 1
fi

umask 077
touch .env
saved_url="$(sed -n 's/^HOISTA_PUBLIC_URL=//p' .env | tail -n 1)"
url="${HOISTA_PUBLIC_URL:-${saved_url:-http://localhost:$port}}"
url="${url%/}"

echo "Building Hoista. The first build takes a while."
docker compose build

if ! grep -q '^HOISTA_SEALED_KEY=' .env; then
  echo "Making Hoista's key."
  key="$(docker compose run --rm --no-deps -e HOISTA_PUBLIC_URL= hoista hoistd --new-sealed-key | tr -d '\r')"
  if [ -z "$key" ]; then
    echo "Couldn't make the key. Nothing was started." >&2
    exit 1
  fi
  printf 'HOISTA_SEALED_KEY=%s\n' "$key" >>.env
fi
# The address it answers on, kept for the next run.
grep -v '^HOISTA_PUBLIC_URL=' .env >.env.next || true
printf 'HOISTA_PUBLIC_URL=%s\n' "$url" >>.env.next
mv .env.next .env

HOISTA_PUBLIC_URL="$url" docker compose up -d --force-recreate

echo "Starting Hoista."
for _ in $(seq 1 "$tries"); do
  logs="$(docker compose logs --no-log-prefix hoista 2>/dev/null || true)"
  claim="$(printf '%s\n' "$logs" | grep 'claim code' | tail -n 1 || true)"
  if [ -n "$claim" ]; then
    code="${claim##* }"
    echo
    echo "Hoista is running at $url"
    echo
    echo "  Claim code: $code"
    echo
    echo "To connect it: in the Hoista app, open Settings → Instances → Add a remote,"
    echo "give $url and click Sign in. Its page asks for the claim code once,"
    echo "then you make a passkey and click Allow."
    echo
    echo "Or set it up in your browser now: $url/#code=$code"
    exit 0
  fi
  if curl -fsS -o /dev/null "http://127.0.0.1:$port/healthz" 2>/dev/null; then
    # It prints the claim code before it answers, so a fresh look finds it.
    if docker compose logs --no-log-prefix hoista 2>/dev/null | grep -q 'claim code'; then
      continue
    fi
    echo
    echo "Hoista is running at $url, and it's already set up."
    echo "To connect another computer: in the Hoista app, Settings → Instances → Add a remote → $url"
    exit 0
  fi
  if [ "$(docker compose ps --status exited -q hoista)" != "" ]; then
    echo "Hoista stopped while starting. Its log:" >&2
    docker compose logs --tail 40 hoista >&2
    exit 1
  fi
  sleep 2
done
echo "Hoista is taking a while to start. Watch it with: docker compose -f deploy/server/compose.yaml logs -f" >&2
exit 1
