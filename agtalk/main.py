# agtalk/main.py — Typer app 入口
import typer
from agtalk.migrations.migrate import run_migrations

app = typer.Typer(rich_markup_mode="rich")

# 导入 cli 模块以注册所有命令到 app
from agtalk import cli  # noqa: F401, E402


@app.callback()
def main():
    """agtalk — Multi-Agent 通信框架"""
    run_migrations()
