# agtalk/term/zellij.py — ZellijMultiplexer 实现
import os
import subprocess
import json

from ._escape import escape_for_terminal
from .base import AbstractMultiplexer


class ZellijMultiplexer(AbstractMultiplexer):
    """Zellij 多路复用器实现。"""

    def get_current_session(self) -> str:
        session = os.environ.get("ZELLIJ_SESSION_NAME")
        if not session:
            raise EnvironmentError("不在 Zellij 环境中 (ZELLIJ_SESSION_NAME 未设置)")
        return session

    def get_current_pane_id(self) -> int:
        pane_id = os.environ.get("ZELLIJ_PANE_ID")
        if not pane_id:
            raise EnvironmentError("不在 Zellij 环境中 (ZELLIJ_PANE_ID 未设置)")
        return int(pane_id)

    def pane_is_alive(self, session: str, pane_id: int) -> bool:
        try:
            result = subprocess.run(
                ["zellij", "--session", session, "action", "list-panes", "--json"],
                capture_output=True, text=True, timeout=5
            )
            if result.returncode != 0:
                return False
            panes = json.loads(result.stdout)
            return any(p["id"] == pane_id and not p.get("exited", False) for p in panes)
        except (subprocess.TimeoutExpired, json.JSONDecodeError, FileNotFoundError):
            return False

    def write_chars_to_pane(self, session: str, pane_id: int, text: str, send_enter: bool = True):
        text = escape_for_terminal(text)
        try:
            subprocess.run(
                ["zellij", "--session", session,
                 "action", "write-chars", "--pane-id", str(pane_id), text],
                check=True, timeout=10
            )
            if send_enter:
                self.send_keys(session, pane_id, "Enter")
        except subprocess.CalledProcessError as e:
            raise RuntimeError(f"write-chars 失败: {e}")
        except subprocess.TimeoutExpired:
            raise RuntimeError("write-chars 超时")

    def send_keys(self, session: str, pane_id: int, key: str):
        try:
            subprocess.run(
                ["zellij", "--session", session,
                 "action", "send-keys", "--pane-id", str(pane_id), key],
                check=True, timeout=5
            )
        except subprocess.CalledProcessError as e:
            raise RuntimeError(f"send-keys 失败: {e}")
        except subprocess.TimeoutExpired:
            raise RuntimeError("send-keys 超时")

    def dump_screen(self, session: str, pane_id: int, lines: int = 100) -> str:
        try:
            result = subprocess.run(
                ["zellij", "--session", session,
                 "action", "dump-screen", "--pane-id", str(pane_id), "--full"],
                capture_output=True, text=True, timeout=5
            )
            result.check_returncode()
            return "\n".join(result.stdout.splitlines()[-lines:])
        except subprocess.CalledProcessError as e:
            raise RuntimeError(f"dump-screen 失败: {e}")
        except subprocess.TimeoutExpired:
            raise RuntimeError("dump-screen 超时")

    def rename_pane(self, session: str, pane_id: int, name: str):
        subprocess.run(
            ["zellij", "--session", session,
             "action", "rename-pane", "--pane-id", str(pane_id), name],
            check=True, timeout=5
        )

    def list_panes(self, session: str) -> list[dict]:
        try:
            result = subprocess.run(
                ["zellij", "--session", session, "action", "list-panes", "--json"],
                capture_output=True, text=True, timeout=5
            )
            if result.returncode != 0:
                return []
            return json.loads(result.stdout)
        except Exception:
            return []

    def get_current_agent_name(self) -> str:
        name = os.environ.get("AGTALK_AGENT_NAME", "")
        if name:
            return name
        try:
            pane_id = self.get_current_pane_id()
            session = self.get_current_session()
        except EnvironmentError:
            raise EnvironmentError(
                "AGTALK_AGENT_NAME 未设置，且无法获取当前 Zellij pane/session。\n"
                "请先执行: agtalk register <name>"
            )
        # 延迟导入避免循环依赖
        from ..db import get_conn
        with get_conn() as conn:
            row = conn.execute(
                "SELECT agent_name FROM agents WHERE session=? AND pane_id=?",
                (session, pane_id)
            ).fetchone()
            if row:
                return row["agent_name"]
        raise EnvironmentError("AGTALK_AGENT_NAME 未设置，请先执行 agtalk register <name>")
