#!/usr/bin/env python3
"""agtalk HTTP Agent 最小 PollInbox 示例。

用法：
    1. agtalk daemon start
    2. agtalk join my-agent --role agent
    3. python3 examples/http_agent_poll.py --name my-agent [--port 19527]

环境变量：
    AGTALK_SESSION_FILE  - 显式指定 session 文件路径
    AGTALK_HTTP_PORT     - 显式指定 daemon HTTP 端口
"""
import argparse
import json
import os
import time
from pathlib import Path

import requests


def load_session(name: str, explicit_path: str | None = None) -> dict:
    if explicit_path:
        path = Path(explicit_path)
    else:
        path = Path.home() / ".agtalk" / "sessions" / f"{name}.json"

    if not path.exists():
        raise FileNotFoundError(f"找不到 session 文件: {path}")

    with open(path) as f:
        return json.load(f)


def make_headers(sess: dict) -> dict:
    return {
        "X-Agtalk-Session-Id": sess["session_id"],
        "X-Agtalk-Token": sess["token"],
        "Content-Type": "application/json",
    }


def call_api(base_url: str, headers: dict, msg: dict, timeout: float = 70.0) -> dict:
    r = requests.post(f"{base_url}/api", headers=headers, json=msg, timeout=timeout)
    r.raise_for_status()
    return r.json()


def poll_inbox(
    base_url: str,
    headers: dict,
    timeout_ms: int = 30000,
    limit: int = 10,
) -> dict:
    return call_api(
        base_url,
        headers,
        {
            "type": "poll_inbox",
            "filter": "unread",
            "timeout_ms": timeout_ms,
            "limit": limit,
        },
        timeout=(timeout_ms / 1000) + 10,
    )


def read_message(
    base_url: str, headers: dict, participant: str, msg_id: str
) -> dict:
    return call_api(
        base_url,
        headers,
        {
            "type": "read",
            "msg_id": msg_id,
            "participant": participant,
        },
    )


def reply_message(
    base_url: str, headers: dict, msg_id: str, choice: str, reason: str = ""
) -> dict:
    return call_api(
        base_url,
        headers,
        {
            "type": "reply",
            "msg_id": msg_id,
            "choice": choice,
            "reason": reason,
        },
    )


def mark_done(
    base_url: str, headers: dict, participant: str, msg_id: str
) -> dict:
    return call_api(
        base_url,
        headers,
        {
            "type": "done",
            "msg_id": msg_id,
            "participant": participant,
        },
    )


def handle_message(msg: dict) -> tuple[str, str] | None:
    """处理消息，返回 (choice, reason) 表示需要回复；返回 None 表示直接 done。"""
    body = msg.get("body", "")
    print(f"[{msg['id']}] 收到消息: {body[:200]}")
    # 在这里添加实际业务处理逻辑。
    # 简单示例：如果消息是任务，回复 ok。
    return "ok", "已处理"


def main() -> None:
    parser = argparse.ArgumentParser(description="agtalk HTTP Agent PollInbox 示例")
    parser.add_argument("--name", default="my-agent", help="Agent 名称（默认 my-agent）")
    parser.add_argument("--port", type=int, default=int(os.getenv("AGTALK_HTTP_PORT", "19527")), help="agtalk daemon HTTP 端口")
    parser.add_argument("--session-file", default=os.getenv("AGTALK_SESSION_FILE"), help="session 文件路径")
    parser.add_argument("--timeout", type=int, default=30000, help="PollInbox 最长等待毫秒数（默认 30000，最大 600000）")
    parser.add_argument("--limit", type=int, default=10, help="每次返回消息条数上限（默认 10，最大 50）")
    args = parser.parse_args()

    base_url = f"http://127.0.0.1:{args.port}"
    sess = load_session(args.name, args.session_file)
    headers = make_headers(sess)
    participant = sess["name"]

    print(f"Agent {participant} 开始 PollInbox 循环 ({base_url})...")

    while True:
        try:
            resp = poll_inbox(base_url, headers, timeout_ms=args.timeout, limit=args.limit)
            data = resp.get("data", {})

            if data.get("empty"):
                if data.get("timed_out"):
                    print("本轮无新消息（timeout）")
                continue

            for msg in data.get("messages", []):
                read_message(base_url, headers, participant, msg["id"])

                result = handle_message(msg)
                if result:
                    choice, reason = result
                    reply_message(base_url, headers, msg["id"], choice, reason)
                else:
                    mark_done(base_url, headers, participant, msg["id"])

        except requests.exceptions.Timeout:
            print("长轮询超时，继续下一轮...")
            time.sleep(1)
        except requests.exceptions.ConnectionError as e:
            print(f"连接失败: {e}")
            time.sleep(5)
        except Exception as e:
            print(f"错误: {e}")
            time.sleep(5)


if __name__ == "__main__":
    main()
