# agtalk/term/_escape.py — 终端转义工具
import re

_CONTROL_CHARS = re.compile(r"[\x00-\x1f\x7f-\x9f]")
_ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*[a-zA-Z]")


def escape_for_terminal(text: str) -> str:
    """移除危险的终端控制序列，防止写入恶意 ANSI 序列。"""
    text = _ANSI_ESCAPE.sub("", text)
    text = _CONTROL_CHARS.sub("", text)
    return text
