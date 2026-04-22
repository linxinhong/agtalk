# ══════════════════════════════════════════════════════════════
# agtalk/migrations/migrate.py  ← migration runner
# ══════════════════════════════════════════════════════════════
# 文件职责：
#   1. 维护 schema_migrations 表，记录已执行的版本
#   2. 按版本号顺序执行尚未运行的 migration 文件
#   3. 每次 migration 在事务内执行，失败自动回滚

import importlib
import pkgutil
import re
import sqlite3
from pathlib import Path

import typer

# 延迟导入，避免循环依赖
def _get_console():
    from agtalk.console import console
    return console


def _get_db_path() -> Path:
    from agtalk.db import DB_PATH
    return DB_PATH


# ── Schema migrations 表初始化 ──────────────────
def _ensure_migration_table(conn: sqlite3.Connection):
    conn.execute("""
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version     TEXT PRIMARY KEY,
            description TEXT,
            applied_at  TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        )
    """)
    conn.commit()


# ── 扫描所有 migration 模块 ─────────────────────
def _discover_migrations() -> list[tuple[str, str, object]]:
    """
    扫描 agtalk/migrations/versions/ 目录下所有符合
    格式 {version}_{description}.py 的文件，按版本号排序返回。
    返回：[(version, description, module), ...]
    """
    versions_pkg = "agtalk.migrations.versions"
    versions_dir = Path(__file__).parent / "versions"
    versions_dir.mkdir(exist_ok=True)

    pattern = re.compile(r"^(\d{3})_(.+)$")
    migrations = []

    for _finder, name, _ispkg in pkgutil.iter_modules([str(versions_dir)]):
        m = pattern.match(name)
        if not m:
            continue
        version = m.group(1)
        description = m.group(2).replace("_", " ")
        module = importlib.import_module(f"{versions_pkg}.{name}")
        migrations.append((version, description, module))

    return sorted(migrations, key=lambda x: x[0])


# ── Runner ──────────────────────────────────────
def run_migrations(db_path: Path | None = None) -> int:
    """
    执行所有待运行的 migration。
    返回本次执行的 migration 数量。
    """
    path = db_path or _get_db_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    console = _get_console()

    conn = sqlite3.connect(path)
    conn.row_factory = sqlite3.Row

    try:
        _ensure_migration_table(conn)

        applied = {
            row["version"]
            for row in conn.execute("SELECT version FROM schema_migrations").fetchall()
        }

        pending = [
            (ver, desc, mod)
            for ver, desc, mod in _discover_migrations()
            if ver not in applied
        ]

        if not pending:
            return 0

        executed = 0
        for version, description, module in pending:
            try:
                with conn:  # 事务，失败自动回滚
                    module.up(conn)
                    conn.execute(
                        "INSERT INTO schema_migrations (version, description) VALUES (?, ?)",
                        (version, description),
                    )
                executed += 1
            except Exception:
                console.print_exception()
                raise typer.Exit(1)

        return executed

    finally:
        conn.close()


# ── CLI ─────────────────────────────────────────
app = typer.Typer(help="数据库 migration 管理")


@app.command("run")
def cmd_run():
    """执行所有待运行的 migration。"""
    console = _get_console()
    n = run_migrations()
    if n:
        console.print(f"[green]✅ 完成，共执行 {n} 个 migration[/green]")


@app.command("list")
def cmd_list():
    """查看所有 migration 及执行状态。"""
    console = _get_console()
    path = _get_db_path()

    applied: dict[str, str] = {}
    if path.exists():
        conn = sqlite3.connect(path)
        conn.row_factory = sqlite3.Row
        _ensure_migration_table(conn)
        applied = {
            row["version"]: row["applied_at"]
            for row in conn.execute("SELECT version, applied_at FROM schema_migrations").fetchall()
        }
        conn.close()

    all_migrations = _discover_migrations()
    for version, description, _ in all_migrations:
        if version in applied:
            status = f"[green]✅ applied[/green]  {applied[version]}"
        else:
            status = "[yellow]⏳ pending[/yellow]"
        console.print(f"  {version}  {description:<40}  {status}")


if __name__ == "__main__":
    app()
