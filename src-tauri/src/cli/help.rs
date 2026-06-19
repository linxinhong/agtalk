//! 帮助信息格式化工具，统一对齐 usage / 描述 / 子选项。

use anstyle::AnsiColor;
use unicode_width::UnicodeWidthStr;

const DESC_COL: usize = 44; // 所有描述文本统一从这一列开始
const CMD_USAGE_COL: usize = DESC_COL - 2; // 2 空格缩进
const OPT_NAME_COL: usize = DESC_COL - 4; // 4 空格缩进

fn colored_label(text: &str, color: AnsiColor) -> String {
    let style = anstyle::Style::new().fg_color(Some(color.into()));
    format!("{}{}{}", style.render(), text, style.render_reset())
}

/// 青色标题："标题:"
pub fn section(title: &str) -> String {
    colored_label(&format!("{}:", title), AnsiColor::Cyan)
}

/// 命令行：2 空格缩进，usage 按显示宽度对齐，后接描述。
/// 当 desc 为空时可用于只打印 usage 行。
pub fn cmd(usage: &str, desc: &str) -> String {
    let usage_width = UnicodeWidthStr::width(usage);
    let pad = CMD_USAGE_COL.saturating_sub(usage_width);
    format!("  {}{}{}", usage, " ".repeat(pad), desc)
}

/// 子选项行：4 空格缩进，name 按显示宽度对齐，后接描述。
pub fn opt(name: &str, desc: &str) -> String {
    let name_width = UnicodeWidthStr::width(name);
    let pad = OPT_NAME_COL.saturating_sub(name_width);
    format!("    {}{}{}", name, " ".repeat(pad), desc)
}
