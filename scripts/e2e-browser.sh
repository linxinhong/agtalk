#!/usr/bin/env bash
# 端到端测试：浏览器扩展身份生命周期（join → lookup → send → inbox → leave）
# 使用临时配置目录与工作区，避免污染用户真实 daemon。

set -euo pipefail

PORT=19528
TMP=$(mktemp -d)
CFG="$TMP/cfg"
WS="$TMP/ws"
mkdir -p "$CFG" "$WS"
echo '{"http_port":'"$PORT"'}' > "$CFG/config.json"
export AGTALK_CONFIG_DIR="$CFG"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AGTALK_BIN="$PROJECT_ROOT/target/debug/agtalk"

cleanup() {
  echo "[cleanup] stopping daemon..."
  cd "$WS" && "$AGTALK_BIN" daemon stop >/dev/null 2>&1 || true
  rm -rf "$TMP"
}
trap cleanup EXIT

cd "$WS"

echo "[e2e] starting daemon on 127.0.0.1:$PORT..."
"$AGTALK_BIN" daemon start

for i in {1..30}; do
  if curl -sf "http://127.0.0.1:$PORT/api/v1/tool/version" >/dev/null; then
    echo "[e2e] daemon ready"
    break
  fi
  sleep 0.2
done

echo "[e2e] join browser..."
JOIN=$(curl -sf -X POST "http://127.0.0.1:$PORT/api/v1/browser/join" \
  -H 'Content-Type: application/json' \
  -d '{"name":"e2e-browser","intro":"e2e","workspace":"test"}')
echo "$JOIN" | jq .
ADDR=$(echo "$JOIN" | jq -r '.address')
TOKEN=$(echo "$JOIN" | jq -r '.token')
NAME=$(echo "$JOIN" | jq -r '.name')
echo "[e2e] address=$ADDR name=$NAME"

echo "[e2e] lookup..."
curl -sf "http://127.0.0.1:$PORT/api/v1/id/lookup?name=e2e-browser" | jq .

echo "[e2e] send to self..."
SEND=$(curl -sf -X POST "http://127.0.0.1:$PORT/api/v1/msg/send" \
  -H 'Content-Type: application/json' \
  -H "X-AgTalk-Address: $ADDR" \
  -H "X-AgTalk-Browser-Token: $TOKEN" \
  -d "{\"to\":\"$ADDR\",\"body\":\"hello e2e\"}")
echo "$SEND" | jq .
MSG_ID=$(echo "$SEND" | jq -r '.id')

echo "[e2e] inbox..."
INBOX=$(curl -sf -X GET "http://127.0.0.1:$PORT/api/v1/msg/inbox" \
  -H "X-AgTalk-Address: $ADDR" \
  -H "X-AgTalk-Browser-Token: $TOKEN")
echo "$INBOX" | jq .
INBOX_IDS=$(echo "$INBOX" | jq -r '.messages[].id')
if echo "$INBOX_IDS" | grep -q "$MSG_ID"; then
  echo "[e2e] inbox contains sent message ✓"
else
  echo "[e2e] inbox missing sent message ✗"
  exit 1
fi

echo "[e2e] leave..."
curl -sf -X POST "http://127.0.0.1:$PORT/api/v1/id/leave" \
  -H 'Content-Type: application/json' \
  -H "X-AgTalk-Address: $ADDR" \
  -H "X-AgTalk-Browser-Token: $TOKEN" \
  -d '{}' | jq .

echo "[e2e] lookup after leave (should be empty)..."
AFTER=$(curl -sf "http://127.0.0.1:$PORT/api/v1/id/lookup?name=e2e-browser")
echo "$AFTER" | jq .
COUNT=$(echo "$AFTER" | jq '.mailboxes | length')
if [ "$COUNT" -eq 0 ]; then
  echo "[e2e] leave cleaned up mailbox ✓"
else
  echo "[e2e] mailbox still visible after leave ✗"
  exit 1
fi

echo "[e2e] all checks passed"
