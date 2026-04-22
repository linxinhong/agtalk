# agtalk/term/_escape.py — 终端转义工具
import re

# 保留格式化字符 \n(0x0A) \r(0x0D) \t(0x09)
# 只移除真正危险的控制字符：NUL, BEL, BS, VT, FF, DEL, C1
_CONTROL_CHARS = re.compile(
    r"[\x00\x07\x08\x0b\x0c\x7f\x80-\x9f]|\x1b\[[0-9;]*[a-zA-Z]"
)


def escape_for_terminal(text: str) -> str:
    """移除危险的终端控制序列，保留 \n \r \t 等格式化字符。"""
    return _CONTROL_CHARS.sub("", text)
