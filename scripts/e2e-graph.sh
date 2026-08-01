#!/usr/bin/env bash
# 端到端测试：图工程三节点 DAG（impl → test → verify）全链路
#   submit（编译建图+派发）→ heartbeat → result（验证：路径/artifact/schema）→ completed
# 使用临时配置目录与工作区，避免污染用户真实 daemon；需要 cargo build -p agtalk 的 debug 二进制。
#
# 前置：target/debug/agtalk 为含 graph 端点的版本（0.2.7+）。

set -euo pipefail

PORT=19529
TMP=$(mktemp -d)
CFG="$TMP/cfg"
WS="$TMP/ws"
mkdir -p "$CFG" "$WS"
echo '{"http_port":'"$PORT"'}' > "$CFG/config.json"
export AGTALK_CONFIG_DIR="$CFG"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AGTALK_BIN="${AGTALK_BIN:-$PROJECT_ROOT/target/debug/agtalk}"

cleanup() {
  echo "[cleanup] stopping daemon..."
  cd "$WS" && "$AGTALK_BIN" daemon stop >/dev/null 2>&1 || true
  rm -rf "$TMP"
}
trap cleanup EXIT

cd "$WS"

echo "[e2e-graph] starting daemon on 127.0.0.1:$PORT..."
"$AGTALK_BIN" daemon start

for _ in {1..30}; do
  if curl -sf "http://127.0.0.1:$PORT/api/v1/tool/version" >/dev/null; then
    echo "[e2e-graph] daemon ready"
    break
  fi
  sleep 0.2
done

# 身份：planner（提交者）+ executor（执行者）
echo "[e2e-graph] join identities..."
"$AGTALK_BIN" --as planner id join planner --notify none --json > "$TMP/planner.json"
"$AGTALK_BIN" --as executor id join executor --notify none --json > "$TMP/executor.json"
EXEC_ADDR=$(jq -r '.address' "$TMP/executor.json")
echo "[e2e-graph] executor=$EXEC_ADDR"

# artifact 产物（M2 验证：uri 必须真实存在）
mkdir -p "$WS/out"
echo "impl-diff" > "$WS/out/impl.txt"
echo "test-report" > "$WS/out/test.txt"

# Graph Spec：三节点串行，全部由 executor 执行
cat > "$WS/spec.yaml" <<'EOF'
version: 1
goal: "e2e 三节点串行验证"
nodes:
  - id: impl-backend
    type: executor
    outputs: { schema: source-diff, artifacts: [backend-diff] }
    executor_requirements: { participant: executor }
    workspace: w1
    read_paths: [src/backend]
    write_paths: [src/backend]
    forbidden_paths: [Cargo.lock]
    acceptance: [{ type: path, rule: changed_within_write_paths }]
    timeout_seconds: 120
  - id: test-backend
    type: deterministic
    dependencies: [impl-backend]
    outputs: { schema: test-report }
    executor_requirements: { participant: executor }
    read_paths: [src/backend]
    acceptance: [{ type: command, rule: "cargo test" }]
    timeout_seconds: 120
  - id: verify
    type: deterministic
    dependencies: [test-backend]
    outputs: { schema: verification-report }
    executor_requirements: { participant: executor }
    acceptance: [{ type: artifact, rule: checksum_matches }]
    timeout_seconds: 60
EOF

echo "[e2e-graph] submit graph..."
SUBMIT=$("$AGTALK_BIN" --as planner graph submit "$WS/spec.yaml" --json)
echo "$SUBMIT" | jq .
RUN_ID=$(echo "$SUBMIT" | jq -r '.run_id')
STATUS=$(echo "$SUBMIT" | jq -r '.status')
if [ "$STATUS" != "ready" ]; then
  echo "[e2e-graph] submit 未就绪 ✗ ($STATUS)"
  exit 1
fi

# 逐节点：派发 → heartbeat → result → succeeded
for NODE in impl-backend test-backend verify; do
  echo "[e2e-graph] === node $NODE ==="

  # 状态应 dispatched（已派发）
  DETAIL=$("$AGTALK_BIN" --as planner graph status "$RUN_ID" --json)
  NODE_STATUS=$(echo "$DETAIL" | jq -r --arg n "$NODE" '.nodes[] | select(.node_key == $n) | .status')
  if [ "$NODE_STATUS" != "dispatched" ]; then
    echo "[e2e-graph] $NODE 应为 dispatched，实际 $NODE_STATUS ✗"
    exit 1
  fi

  # heartbeat（dispatched → running）
  "$AGTALK_BIN" --as executor graph node heartbeat --run "$RUN_ID" --node "$NODE" --attempt 1 --json | jq .

  # 按节点写 result.json（changed_files 必须 ⊆ write_paths；artifact 必须真实存在）
  case "$NODE" in
    impl-backend)
      cat > "$WS/result.json" <<'EOF'
{
  "result": "实现后端模块",
  "changed_files": ["src/backend/api.rs"],
  "output_artifacts": [
    { "artifact_type": "source-diff", "schema_version": "v1", "uri": "REPLACE_IMPL", "checksum": "" }
  ],
  "verification_claims": [{ "command": "cargo check -p backend", "exit_code": 0, "summary": "ok" }],
  "blockers": []
}
EOF
      # 跨平台 sed（macOS -i 需参数；GNU 直接 -i）：用 .bak 后缀兼容
      sed -i.bak "s|REPLACE_IMPL|file://$WS/out/impl.txt|" "$WS/result.json" && rm -f "$WS/result.json.bak"
      ;;
    test-backend)
      cat > "$WS/result.json" <<'EOF'
{
  "result": "测试通过",
  "changed_files": [],
  "output_artifacts": [
    { "artifact_type": "test-report", "schema_version": "v1", "uri": "REPLACE_TEST", "checksum": "" }
  ],
  "verification_claims": [{ "command": "cargo test -p backend", "exit_code": 0, "summary": "42 passed" }],
  "blockers": []
}
EOF
      sed -i.bak "s|REPLACE_TEST|file://$WS/out/test.txt|" "$WS/result.json" && rm -f "$WS/result.json.bak"
      ;;
    verify)
      cat > "$WS/result.json" <<'EOF'
{
  "result": "验收通过",
  "changed_files": [],
  "output_artifacts": [],
  "verification_claims": [],
  "blockers": []
}
EOF
      ;;
  esac

  REPORT=$("$AGTALK_BIN" --as executor graph node result --run "$RUN_ID" --node "$NODE" --attempt 1 --file "$WS/result.json" --json)
  echo "$REPORT" | jq .
  R_STATUS=$(echo "$REPORT" | jq -r '.status')
  if [ "$R_STATUS" != "succeeded" ]; then
    echo "[e2e-graph] $NODE 未 succeeded ✗ ($R_STATUS)"
    exit 1
  fi
done

# 最终：图 completed
FINAL=$("$AGTALK_BIN" --as planner graph status "$RUN_ID" --json)
echo "$FINAL" | jq .
F_STATUS=$(echo "$FINAL" | jq -r '.run.status')
if [ "$F_STATUS" != "completed" ]; then
  echo "[e2e-graph] 图未 completed ✗ ($F_STATUS)"
  exit 1
fi

# 事件日志应有完整轨迹
LOG=$("$AGTALK_BIN" --as planner graph logs "$RUN_ID" --json)
echo "$LOG" | jq -r '.events[].event_type' | sort | uniq -c

echo "[e2e-graph] all checks passed ✓"
