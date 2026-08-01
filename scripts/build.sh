#!/usr/bin/env bash
# 开发态统一构建入口（AGENTS.md §5.5 已知陷阱的根治）：
#
# 直接 cargo build 必须带 --features custom-protocol，否则 tauri build.rs 判定 dev=true，
# 二进制连 devUrl(localhost:5173) 而非内嵌 dist，GUI/审批弹窗全部白屏。
# 所有开发/交付构建一律走本脚本，避免每次手动记得加 features。
#
# 用法：
#   ./scripts/build.sh            # debug 构建（默认，等价旧的 cargo build -p agtalk）
#   ./scripts/build.sh --release  # release 构建
# 传参直接透传给 cargo build（如 --release、--features ...）。
set -euo pipefail
cd "$(dirname "$0")/.."

args=("$@")

# 一律补上 custom-protocol（若用户显式传了 --features，追加会重复——这里接受 cargo 的
# "feature specified multiple times" 容错；大多数场景直接透传即可）
cargo build -p agtalk --features custom-protocol "$@"

echo "✔ 已按 custom-protocol 构建（GUI 内嵌 dist，不会白屏）"
