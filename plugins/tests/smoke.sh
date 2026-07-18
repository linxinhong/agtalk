#!/bin/sh
# Smoke tests for the agtalk notify plugins (zellij / tmux).
# Run from the repo root:  bash plugins/tests/smoke.sh
# Uses fake zellij/tmux binaries, so the real tools are not required.
#
# Intentionally does NOT `set -e`: several cases assert that a plugin fails.

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
PLUGIN_DIR="$SCRIPT_DIR/.."
ZELLIJ_PLUGIN="$PLUGIN_DIR/agtalk-notify-zellij"
TMUX_PLUGIN="$PLUGIN_DIR/agtalk-notify-tmux"

FAKE=$(mktemp -d)
LOG="$FAKE/log"
OUT="$FAKE/out"
PASS=0
FAIL=0
trap 'rm -rf "$FAKE"' EXIT

ok()  { PASS=$((PASS + 1)); printf '  ok: %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL: %s\n' "$1" >&2; }

assert_contains() {
    if grep -qF "$2" "$1"; then ok "$3"; else bad "$3 (missing: $2)"; fi
}
assert_not_contains() {
    if grep -qF "$2" "$1"; then bad "$3 (unexpected: $2)"; else ok "$3"; fi
}

# Fake zellij: logs every call; every action succeeds.
cat > "$FAKE/zellij" <<'EOF'
#!/bin/sh
echo "$0 $*" >> "$AGTALK_TEST_LOG"
exit 0
EOF
chmod +x "$FAKE/zellij"

# Fake tmux: logs every call; display-message prints a pane id; rest succeeds.
cat > "$FAKE/tmux" <<'EOF'
#!/bin/sh
echo "$0 $*" >> "$AGTALK_TEST_LOG"
case "$1" in
    display-message) echo '%2' ;;
    *) exit 0 ;;
esac
EOF
chmod +x "$FAKE/tmux"

# Fake cmux: logs every call (and the socket capability seen); every action succeeds.
cat > "$FAKE/cmux" <<'EOF'
#!/bin/sh
echo "$0 $* cap=${CMUX_SOCKET_CAPABILITY:-}" >> "$AGTALK_TEST_LOG"
exit 0
EOF
chmod +x "$FAKE/cmux"

export PATH="$FAKE:$PATH"
export ZELLIJ_SESSION_NAME=s
export ZELLIJ_PANE_ID=1
export TMUX_PANE=%2
export TMUX=/tmp/tmux-501/default,12345,0
export CMUX_SURFACE_ID=449812E3-B071-4F09-92CB-60DAE9FB35DB
export CMUX_WORKSPACE_ID=6F164C22-1DFB-4F6F-A268-F9BFBE753A58
export CMUX_SOCKET_CAPABILITY=test-cap
export AGTALK_TEST_LOG="$LOG"

TEXT='[agtalk:abcdef12] | from nora | exec: agtalk --as x msg read'

printf '== zellij ==\n'

: > "$LOG"
"$ZELLIJ_PLUGIN" discover > "$OUT"
assert_contains "$OUT" '"ready":true'      'discover ready'
assert_contains "$OUT" '"session":"s"'     'discover session'
assert_contains "$OUT" '"pane":"1"'        'discover pane'

: > "$LOG"
printf '{"version":1,"endpoint":{"session":"s","pane":"1"},"text":"%s","send_enter":true}' "$TEXT" \
    | "$ZELLIJ_PLUGIN" send
RC=$?
[ "$RC" -eq 0 ] && ok 'send exit 0' || bad "send exit $RC"
assert_contains "$LOG" 'action paste --pane-id 1'        'send paste'
assert_contains "$LOG" "$TEXT"                           'send text'
assert_contains "$LOG" 'action send-keys --pane-id 1 Enter' 'send enter'

: > "$LOG"
printf '{"version":1,"endpoint":{"session":"s","pane":"1"},"text":"%s","send_enter":false}' "$TEXT" \
    | "$ZELLIJ_PLUGIN" send
assert_contains    "$LOG" 'action paste --pane-id 1' 'send no-enter paste'
assert_not_contains "$LOG" 'send-keys'              'send no-enter no send-keys'

: > "$LOG"
printf '{"version":1,"endpoint":{"session":"s","pane":"1"},"text":"%s"}' "$TEXT" \
    | "$ZELLIJ_PLUGIN" send --dry-run
RC=$?
[ "$RC" -eq 0 ] && ok 'dry-run exit 0' || bad "dry-run exit $RC"
assert_not_contains "$LOG" 'action paste' 'dry-run no paste'
assert_not_contains "$LOG" 'send-keys'    'dry-run no send-keys'

: > "$LOG"
if printf '{"version":2,"endpoint":{"session":"s","pane":"1"}}' \
    | "$ZELLIJ_PLUGIN" send >/dev/null 2>&1; then
    bad 'version=2 should fail'
else
    ok 'version=2 rejected'
fi

printf '\n== tmux ==\n'

: > "$LOG"
"$TMUX_PLUGIN" discover > "$OUT"
assert_contains "$OUT" '"ready":true'  'discover ready'
assert_contains "$OUT" '"pane":"%2"'   'discover pane'

: > "$LOG"
printf '{"version":1,"endpoint":{"pane":"%%2"},"text":"%s","send_enter":true}' "$TEXT" \
    | "$TMUX_PLUGIN" send
RC=$?
[ "$RC" -eq 0 ] && ok 'send exit 0' || bad "send exit $RC"
assert_contains "$LOG" 'send-keys -t %2 -R' 'send send-keys'
assert_contains "$LOG" "$TEXT"              'send text'
assert_contains "$LOG" 'Enter'              'send enter'

: > "$LOG"
printf '{"version":1,"endpoint":{"pane":"%%2"},"text":"%s","send_enter":false}' "$TEXT" \
    | "$TMUX_PLUGIN" send
assert_contains    "$LOG" 'send-keys -t %2 -R' 'send no-enter send-keys'
assert_not_contains "$LOG" 'Enter'            'send no-enter no Enter'

: > "$LOG"
printf '{"version":1,"endpoint":{"pane":"%%2"},"text":"%s"}' "$TEXT" \
    | "$TMUX_PLUGIN" send --dry-run
RC=$?
[ "$RC" -eq 0 ] && ok 'dry-run exit 0' || bad "dry-run exit $RC"
assert_not_contains "$LOG" 'send-keys' 'dry-run no send-keys'

: > "$LOG"
if printf '{"version":2,"endpoint":{"pane":"%%2"}}' \
    | "$TMUX_PLUGIN" send >/dev/null 2>&1; then
    bad 'version=2 should fail'
else
    ok 'version=2 rejected'
fi

printf '\n== cmux ==\n'

CMUX_PLUGIN="$PLUGIN_DIR/agtalk-notify-cmux"

: > "$LOG"
"$CMUX_PLUGIN" discover > "$OUT"
assert_contains "$OUT" '"ready":true'                                          'discover ready'
assert_contains "$OUT" '"surface":"449812E3-B071-4F09-92CB-60DAE9FB35DB"'      'discover surface'
assert_contains "$OUT" "\"cmux_bin\":\"$FAKE/cmux\""                            'discover cmux_bin pinned'
assert_contains "$OUT" '"capability":"test-cap"'                                'discover capability pinned'

: > "$LOG"
printf '{"version":1,"endpoint":{"surface":"449812E3-B071-4F09-92CB-60DAE9FB35DB","cmux_bin":"%s","capability":"test-cap-endpoint"},"text":"%s","send_enter":true}' "$FAKE/cmux" "$TEXT" \
    | "$CMUX_PLUGIN" send
RC=$?
[ "$RC" -eq 0 ] && ok 'send exit 0' || bad "send exit $RC"
assert_contains "$LOG" "$FAKE/cmux send --surface 449812E3-B071-4F09-92CB-60DAE9FB35DB" 'send uses pinned cmux_bin'
assert_contains "$LOG" "$TEXT"                                                   'send text'
assert_contains "$LOG" 'send-key --surface 449812E3-B071-4F09-92CB-60DAE9FB35DB enter' 'send enter'
assert_contains "$LOG" 'cap=test-cap-endpoint'                                   'send exports endpoint capability'

: > "$LOG"
printf '{"version":1,"endpoint":{"surface":"449812E3-B071-4F09-92CB-60DAE9FB35DB"},"text":"%s","send_enter":false}' "$TEXT" \
    | "$CMUX_PLUGIN" send
assert_contains    "$LOG" 'send --surface' 'send no-enter send'
assert_not_contains "$LOG" 'send-key'      'send no-enter no send-key'

: > "$LOG"
printf '{"version":1,"endpoint":{"surface":"449812E3-B071-4F09-92CB-60DAE9FB35DB"},"text":"%s"}' "$TEXT" \
    | "$CMUX_PLUGIN" send --dry-run
RC=$?
[ "$RC" -eq 0 ] && ok 'dry-run exit 0' || bad "dry-run exit $RC"
assert_contains    "$LOG" 'read-screen'   'dry-run reachability check'
assert_not_contains "$LOG" 'send --surface' 'dry-run no send'
assert_not_contains "$LOG" 'send-key'       'dry-run no send-key'

: > "$LOG"
if printf '{"version":2,"endpoint":{"surface":"449812E3-B071-4F09-92CB-60DAE9FB35DB"}}' \
    | "$CMUX_PLUGIN" send >/dev/null 2>&1; then
    bad 'version=2 should fail'
else
    ok 'version=2 rejected'
fi

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
