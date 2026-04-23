# agtalk/term/tmux.py — TmuxMultiplexer 实现
import os
import subprocess

from .base import AbstractMultiplexer


def _run(args: list[str], capture: bool = False, check: bool = True, timeout: int = 10) -> str:
    """执行 tmux 子命令的通用封装。"""
    result = subprocess.run(
        ["tmux"] + args,
        capture_output=capture,
        text=True,
        check=check,
        timeout=timeout,
    )
    return result.stdout.strip() if capture else ""


def _pane_target(session: str, pane_id: int) -> str:
    """生成 tmux target。

    tmux 的 pane id（%0, %1 等）是全局唯一的，直接用 %<pane_id> 即可，
    不需要加 session。加 session 会被 tmux 误解析为 window 名。
    """
    return f"%{pane_id}"


class TmuxMultiplexer(AbstractMultiplexer):
    """Tmux 多路复用器实现。"""

    # ─── 环境检测 ──────────────────────────────────────
    def is_in_environment(self) -> bool:
        return os.environ.get("TMUX") is not None

    # ─── AbstractMultiplexer 接口 ──────────────────────
    def get_current_session(self) -> str:
        try:
            return _run(["display-message", "-p", "#{session_name}"], capture=True)
        except subprocess.CalledProcessError as e:
            raise EnvironmentError(f"无法获取当前 tmux session: {e}")

    def get_current_pane_id(self) -> int:
        try:
            raw = _run(["display-message", "-p", "#{pane_id}"], capture=True)
            # tmux pane_id 格式如 %0, %1
            return int(raw.lstrip("%"))
        except subprocess.CalledProcessError as e:
            raise EnvironmentError(f"无法获取当前 tmux pane ID: {e}")
        except ValueError:
            raise EnvironmentError(f"无法解析 tmux pane ID: {raw}")

    def pane_is_alive(self, session: str, pane_id: int) -> bool:
        try:
            # -a 列出 session 中所有 window 的 pane，不只当前 window
            output = _run(
                ["list-panes", "-a", "-t", session, "-F", "#{pane_id}"],
                capture=True, check=False, timeout=5,
            )
            alive_panes = {line.strip() for line in output.splitlines() if line.strip()}
            return f"%{pane_id}" in alive_panes
        except subprocess.TimeoutExpired:
            return False

    def write_chars_to_pane(self, session: str, pane_id: int, text: str, send_enter: bool = True):
        target = _pane_target(session, pane_id)
        # tmux send-keys 把每个参数当作一个 key sequence
        # 文字内容整体作为一个参数传入
        _run(["send-keys", "-t", target, text], timeout=10)
        if send_enter:
            self.send_keys(session, pane_id, "Enter")

    def send_keys(self, session: str, pane_id: int, key: str):
        target = _pane_target(session, pane_id)
        _run(["send-keys", "-t", target, key], timeout=5)

    def dump_screen(self, session: str, pane_id: int, lines: int = 100) -> str:
        target = _pane_target(session, pane_id)
        output = _run(
            ["capture-pane", "-t", target, "-p"],
            capture=True, timeout=5,
        )
        return "\n".join(output.splitlines()[-lines:])

    def rename_pane(self, session: str, pane_id: int, name: str):
        target = _pane_target(session, pane_id)
        _run(["select-pane", "-t", target, "-T", name], timeout=5)

    def list_panes(self, session: str) -> list[dict]:
        # -a 列出 session 中所有 window 的 pane
        output = _run(
            ["list-panes", "-a", "-t", session, "-F", "#{pane_id} #{pane_active} #{pane_dead}"],
            capture=True, check=False, timeout=5,
        )
        panes = []
        for line in output.splitlines():
            parts = line.strip().split()
            if len(parts) >= 3:
                panes.append({
                    "id": int(parts[0].lstrip("%")),
                    "active": parts[1] == "1",
                    "exited": parts[2] == "1",
                })
        return panes

    def get_current_agent_name(self) -> str:
        name = os.environ.get("AGTALK_AGENT_NAME", "")
        if name:
            return name
        try:
            pane_id = self.get_current_pane_id()
            session = self.get_current_session()
        except EnvironmentError:
            raise EnvironmentError(
                "AGTALK_AGENT_NAME 未设置，且无法获取当前 tmux pane/session。\n"
                "请先执行: agtalk register <name>"
            )
        from ..db import get_conn
        with get_conn() as conn:
            row = conn.execute(
                "SELECT agent_name FROM agents WHERE session=? AND pane_id=?",
                (session, pane_id)
            ).fetchone()
            if row:
                return row["agent_name"]
        raise EnvironmentError("AGTALK_AGENT_NAME 未设置，请先执行 agtalk register <name>")
