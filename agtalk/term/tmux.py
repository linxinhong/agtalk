# agtalk/term/tmux.py — TmuxMultiplexer 实现
import os

from .base import AbstractMultiplexer


class TmuxMultiplexer(AbstractMultiplexer):
    """Tmux 多路复用器实现（当前为基本框架，核心操作待实现）。"""

    # ─── 类属性 ──────────────────────────────────────
    @property
    def name(self) -> str:
        return "tmux"

    @property
    def env_session_key(self) -> str:
        return "TMUX"

    @property
    def env_pane_key(self) -> str:
        return "TMUX_PANE"

    # ─── 环境检测 ──────────────────────────────────────
    def is_in_environment(self) -> bool:
        """检测当前是否在 Tmux 环境中。"""
        return os.environ.get("TMUX") is not None

    # ─── AbstractMultiplexer 接口 ──────────────────────
    def get_current_session(self) -> str:
        raise NotImplementedError("Tmux get_current_session 尚未实现")

    def get_current_pane_id(self) -> int:
        raise NotImplementedError("Tmux get_current_pane_id 尚未实现")

    def pane_is_alive(self, session: str, pane_id: int) -> bool:
        raise NotImplementedError("Tmux pane_is_alive 尚未实现")

    def write_chars_to_pane(self, session: str, pane_id: int, text: str, send_enter: bool = True):
        raise NotImplementedError("Tmux write_chars_to_pane 尚未实现")

    def send_keys(self, session: str, pane_id: int, key: str):
        raise NotImplementedError("Tmux send_keys 尚未实现")

    def dump_screen(self, session: str, pane_id: int, lines: int = 100) -> str:
        raise NotImplementedError("Tmux dump_screen 尚未实现")

    def rename_pane(self, session: str, pane_id: int, name: str):
        raise NotImplementedError("Tmux rename_pane 尚未实现")

    def list_panes(self, session: str) -> list[dict]:
        raise NotImplementedError("Tmux list_panes 尚未实现")

    def get_current_agent_name(self) -> str:
        raise NotImplementedError("Tmux get_current_agent_name 尚未实现")
