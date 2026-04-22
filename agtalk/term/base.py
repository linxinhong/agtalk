# agtalk/term/base.py — AbstractMultiplexer ABC 接口
from abc import ABC, abstractmethod


class AbstractMultiplexer(ABC):
    """终端多路复用器抽象基类。

    封装了对多路复用器（如 Zellij、Tmux）的底层操作，
    使上层代码与具体的多路复用器实现解耦。
    """

    @abstractmethod
    def get_current_session(self) -> str:
        """返回当前 session 名。"""
        ...

    @abstractmethod
    def get_current_pane_id(self) -> int:
        """返回当前 pane ID。"""
        ...

    @abstractmethod
    def pane_is_alive(self, session: str, pane_id: int) -> bool:
        """验证指定 pane 是否存活。"""
        ...

    @abstractmethod
    def write_chars_to_pane(self, session: str, pane_id: int, text: str, send_enter: bool = True):
        """向指定 pane 写入文本。"""
        ...

    @abstractmethod
    def send_keys(self, session: str, pane_id: int, key: str):
        """向指定 pane 发送按键（如 Enter、Ctrl+C 等）。"""
        ...

    @abstractmethod
    def dump_screen(self, session: str, pane_id: int, lines: int = 100) -> str:
        """获取 pane 屏幕内容最后 lines 行。"""
        ...

    @abstractmethod
    def rename_pane(self, session: str, pane_id: int, name: str):
        """重命名 pane。"""
        ...

    @abstractmethod
    def list_panes(self, session: str) -> list[dict]:
        """获取 session 的所有 pane 列表。"""
        ...

    @abstractmethod
    def get_current_agent_name(self) -> str:
        """从环境变量或 DB 获取当前 agent 名。"""
        ...
