# agtalk/factory.py — multiplexer 自动检测与工厂
import os
from .term.base import AbstractMultiplexer
from .term.zellij import ZellijMultiplexer
from .term.tmux import TmuxMultiplexer

_multiplexer_instance: AbstractMultiplexer | None = None


def detect() -> type[AbstractMultiplexer]:
    """根据环境变量自动检测当前终端多路复用器类型。"""
    if os.environ.get("ZELLIJ_SESSION_NAME"):
        return ZellijMultiplexer
    if os.environ.get("TMUX"):
        return TmuxMultiplexer
    raise EnvironmentError(
        "无法检测到支持的终端多路复用器。"
        "请在 Zellij 或 Tmux 环境中运行。"
    )


def detect_name() -> str:
    """返回当前多路复用器名称（zellij / tmux / unknown）。"""
    if os.environ.get("ZELLIJ_SESSION_NAME"):
        return "zellij"
    if os.environ.get("TMUX"):
        return "tmux"
    return "unknown"


def get() -> AbstractMultiplexer:
    """获取当前 multiplexer 的单例实例。"""
    global _multiplexer_instance
    if _multiplexer_instance is None:
        cls = detect()
        _multiplexer_instance = cls()
    return _multiplexer_instance


def get_agent_name() -> str:
    """快捷方法：获取当前 agent 名称。"""
    return get().get_current_agent_name()
