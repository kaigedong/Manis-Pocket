#!/usr/bin/env bash
# End-to-end sync check between this Mac and Omarchy2.
#
# Both sides run the same GPUI binary against throwaway databases on a port of
# their own (the production app owns 31774). The script proves, in order:
#   discovery -> pairing (PIN) -> persistence -> clipboard propagation
#
# It is deliberately step-by-step with timeouts: mDNS and pairing are
# asynchronous, and a silent hang hides which step broke.
set -uo pipefail

PORT="${PORT:-31950}"
LINUX_HOST="${LINUX_HOST:-Omarchy2}"
MAC_IP="${MAC_IP:-192.168.3.101}"
LINUX_IP="${LINUX_IP:-192.168.3.12}"
MAC_LOG=/tmp/e2e/mac.log
LINUX_LOG=/tmp/e2e/linux.log
MAC_DB=/tmp/e2e/mac.db
LINUX_DB=/tmp/e2e/linux.db

say() { printf '\n=== %s ===\n' "$*"; }
fail() {
  say "FAILED: $*"
  pkill -x manis-pocket
  ssh "$LINUX_HOST" 'pkill -x manis-pocket' >/dev/null 2>&1
  exit 1
}

# Poll a log for a pattern, on either host.
wait_for() {
  local host="$1" file="$2" pattern="$3" seconds="$4"
  for _ in $(seq 1 "$seconds"); do
    local hit
    if [ "$host" = local ]; then
      hit=$(grep -o "$pattern" "$file" 2>/dev/null | head -1)
    else
      hit=$(ssh "$LINUX_HOST" "grep -o '$pattern' $file 2>/dev/null | head -1")
    fi
    [ -n "$hit" ] && { echo "$hit"; return 0; }
    sleep 1
  done
  return 1
}

say "clean slate"
pkill -x manis-pocket
ssh "$LINUX_HOST" 'pkill -x manis-pocket; sleep 2'
rm -rf /tmp/e2e && mkdir -p /tmp/e2e
echo "instances: mac=$(pgrep -x manis-pocket | wc -l) linux=$(ssh "$LINUX_HOST" 'pgrep -x manis-pocket | wc -l')"

say "start both sides on port $PORT"
printf '{"syncEnabled":"1","syncDeviceName":"MacE2E","syncDeviceID":"mac-e2e","historySize":"1004"}\n' > /tmp/e2e/settings.json
MANIS_POCKET_SYNC_PORT=$PORT MANIS_POCKET_DB=$MAC_DB ./target/debug/manis-pocket > "$MAC_LOG" 2>&1 &

ssh "$LINUX_HOST" "rm -rf /tmp/e2e && mkdir -p /tmp/e2e && \
  printf '{\"syncEnabled\":\"1\",\"syncDeviceName\":\"LinuxE2E\",\"syncDeviceID\":\"linux-e2e\"}\n' > /tmp/e2e/settings.json && \
  cd ~/manis-pocket-build && \
  nohup env MANIS_POCKET_SYNC_PORT=$PORT WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=/run/user/1000 \
    MANIS_POCKET_DB=$LINUX_DB ./target/debug/manis-pocket > $LINUX_LOG 2>&1 & sleep 2; echo started"

say "wait for mutual discovery"
MAC_SEES=$(wait_for local "$MAC_LOG" 'peer LinuxE2E ([0-9A-Za-z]*)' 20) || MAC_SEES=""
LINUX_SEES=$(wait_for "$LINUX_HOST" "$LINUX_LOG" 'peer MacE2E ([0-9A-Za-z]*)' 20) || LINUX_SEES=""
if [ -z "$MAC_SEES" ] || [ -z "$LINUX_SEES" ]; then
  say "mDNS did not converge, dialling explicitly"
  MANIS_POCKET_DB=$MAC_DB ./target/debug/manis-pocket --connect "$LINUX_IP:$PORT"
  ssh "$LINUX_HOST" "cd ~/manis-pocket-build && env MANIS_POCKET_DB=$LINUX_DB XDG_RUNTIME_DIR=/run/user/1000 ./target/debug/manis-pocket --connect $MAC_IP:$PORT"
  MAC_SEES=$(wait_for local "$MAC_LOG" 'peer LinuxE2E ([0-9A-Za-z]*)' 15) || fail "no peer seen on mac"
  LINUX_SEES=$(wait_for "$LINUX_HOST" "$LINUX_LOG" 'peer MacE2E ([0-9A-Za-z]*)' 15) || fail "no peer seen on linux"
fi
LINUX_PEER=$(echo "$MAC_SEES" | sed 's/.*(\(.*\))/\1/')
MAC_PEER=$(echo "$LINUX_SEES" | sed 's/.*(\(.*\))/\1/')
echo "mac sees linux as $LINUX_PEER"
echo "linux sees mac as $MAC_PEER"

say "pair"
# Pairing is a two-sided handshake over gossip: a request can arrive before the
# other side is listening for it, so the whole sequence is retried.
paired=false
for attempt in 1 2 3; do
  MANIS_POCKET_DB=$MAC_DB ./target/debug/manis-pocket --pair "$LINUX_PEER"
  PIN=$(wait_for "$LINUX_HOST" "$LINUX_LOG" 'pin=[0-9]*' 10) || PIN=""
  if [ -n "$PIN" ]; then
    echo "attempt $attempt pin $PIN"
    # The control socket lives next to the database, so the CLI must point at
    # the same one the running instance uses.
    ssh "$LINUX_HOST" "cd ~/manis-pocket-build && env MANIS_POCKET_DB=$LINUX_DB XDG_RUNTIME_DIR=/run/user/1000 ./target/debug/manis-pocket --accept $MAC_PEER ${PIN#pin=}"
  fi
  if wait_for local "$MAC_LOG" 'pairing with [0-9A-Za-z]* complete: true' 12 >/dev/null \
     && wait_for "$LINUX_HOST" "$LINUX_LOG" 'pairing with [0-9A-Za-z]* complete: true' 12 >/dev/null; then
    paired=true
    break
  fi
  echo "attempt $attempt did not pair, retrying"
done
[ "$paired" = true ] || fail "pairing never completed"
echo "mac pairs:   $(sqlite3 "$MAC_DB" 'select count(*) from sync_pairs;')"
echo "linux pairs: $(ssh "$LINUX_HOST" "sqlite3 $LINUX_DB 'select count(*) from sync_pairs;'")"

# The engine records the peer after the handshake settles; copy too early and the
# update is broadcast to nobody.
sleep 3

say "copy on mac, expect it on linux"
MARK="e2e-$(date +%s)"
printf '%s' "$MARK" | pbcopy
FOUND=""
for _ in $(seq 1 20); do
  FOUND=$(ssh "$LINUX_HOST" "sqlite3 $LINUX_DB \"select title from history_items where title='$MARK' limit 1;\"" 2>/dev/null)
  [ -n "$FOUND" ] && break
  sleep 1
done
[ -n "$FOUND" ] || fail "linux never received '$MARK'"

say "copy on linux, expect it on mac"
BACK="back-$(date +%s)"
ssh "$LINUX_HOST" "printf '%s' '$BACK' | env XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 wl-copy"
BACK_FOUND=""
for _ in $(seq 1 20); do
  BACK_FOUND=$(sqlite3 "$MAC_DB" "select title from history_items where title='$BACK' limit 1;" 2>/dev/null)
  [ -n "$BACK_FOUND" ] && break
  sleep 1
done
[ -n "$BACK_FOUND" ] || fail "mac never received '$BACK'"

say "PASS"
echo "linux received: $FOUND"
echo "mac received:   $BACK_FOUND"
echo "mac history:   $(sqlite3 "$MAC_DB" 'select count(*) from history_items;')"
echo "linux history: $(ssh "$LINUX_HOST" "sqlite3 $LINUX_DB 'select count(*) from history_items;'")"

say "cleanup"
pkill -x manis-pocket
ssh "$LINUX_HOST" 'pkill -x manis-pocket'
