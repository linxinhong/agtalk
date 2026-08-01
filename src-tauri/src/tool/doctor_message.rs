//! doctor 消息/等待/通知检查（doctor.rs 拆分，控制行数红线）。

use crate::identity::mailbox;
use crate::identity::session_file::NotifyTarget;
use crate::notify;
use crate::notify::NotifyHint;
use crate::proto::DiagnosisCheck;
use crate::routing::inbox;
use crate::tool::doctor::*;
use crate::tool::doctor_checks::*;
use crate::tool::DoctorContext;

pub(crate) fn message_checks(
    ctx: &DoctorContext,
    identity: &Option<ResolvedIdentity>,
) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();

    let db_ok = ctx
        .storage
        .as_ref()
        .map(|s| s.conn().query_row("SELECT 1", [], |_| Ok(())).is_ok())
        .unwrap_or(false);
    checks.push(check(
        "message",
        "message.db",
        if db_ok { "ok" } else { "error" },
        if db_ok {
            "DB 可打开并执行 SELECT 1".to_string()
        } else {
            "DB 无法打开".to_string()
        },
        if db_ok {
            None
        } else {
            Some("检查 daemon 日志与数据库文件权限")
        },
        None,
        serde_json::Value::Null,
    ));

    let Some(id) = identity else {
        checks.push(check(
            "message",
            "message.mailbox",
            "skip",
            "身份未解析，跳过 inbox 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "message",
            "message.pending",
            "skip",
            "身份未解析，跳过 pending 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "message",
            "message.latest_event",
            "skip",
            "身份未解析，跳过 latest_event 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "message",
            "message.read_ready",
            "skip",
            "身份未解析，read 未就绪",
            Some("先执行 agtalk id join <name>"),
            Some("agtalk id join <name>"),
            serde_json::Value::Null,
        ));
        return checks;
    };

    let mailbox_ok = ctx
        .storage
        .as_ref()
        .and_then(|s| mailbox::get_by_address(s, &id.address).ok())
        .flatten()
        .is_some();
    checks.push(check(
        "message",
        "message.mailbox",
        if mailbox_ok { "ok" } else { "error" },
        if mailbox_ok {
            "当前 address 可查询 inbox".to_string()
        } else {
            "当前 address 在 DB 中无 mailbox".to_string()
        },
        if mailbox_ok {
            None
        } else {
            Some("重新 join 恢复 mailbox")
        },
        if mailbox_ok {
            None
        } else {
            Some("agtalk id join <name>")
        },
        serde_json::Value::Null,
    ));

    let pending = ctx
        .storage
        .as_ref()
        .and_then(|s| inbox::inbox(s, &id.address, false).ok())
        .map(|msgs| msgs.into_iter().filter(|m| m.status == "pending").count())
        .unwrap_or(0);
    checks.push(check(
        "message",
        "message.pending",
        "ok",
        format!("{} 条 pending 消息", pending),
        None,
        None,
        serde_json::json!({ "pending": pending }),
    ));

    let latest_event = ctx
        .storage
        .as_ref()
        .and_then(|s| inbox::max_event_id(s, &id.address).ok())
        .unwrap_or(0);
    checks.push(check(
        "message",
        "message.latest_event",
        "ok",
        format!("最新 event_id: {}", latest_event),
        None,
        None,
        serde_json::json!({ "latest_event_id": latest_event }),
    ));

    checks.push(check(
        "message",
        "message.read_ready",
        "ok",
        "agtalk msg read 可解析当前身份",
        None,
        None,
        serde_json::Value::Null,
    ));

    checks
}

// ---- wait / sse ----

pub(crate) fn wait_checks(
    ctx: &DoctorContext,
    identity: &Option<ResolvedIdentity>,
) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();

    let http_url = format!("http://127.0.0.1:{}", ctx.config.http_port);
    let daemon_running = crate::server::daemon::read_status_file()
        .ok()
        .flatten()
        .is_some();
    let mut http_ok = false;

    if daemon_running {
        http_ok = http_get(&format!("{}/api/v1/daemon/status", http_url), &[]).is_ok();
        checks.push(check(
            "wait",
            "wait.http",
            if http_ok { "ok" } else { "error" },
            if http_ok {
                format!("daemon HTTP 可达: {}", http_url)
            } else {
                format!("daemon HTTP 不可达: {}", http_url)
            },
            if http_ok {
                None
            } else {
                Some("启动 daemon 后再使用 wait")
            },
            if http_ok {
                None
            } else {
                Some("agtalk daemon start")
            },
            serde_json::json!({ "url": http_url }),
        ));
    } else {
        checks.push(check(
            "wait",
            "wait.http",
            "skip",
            "daemon 未运行，跳过 HTTP 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
    }

    let Some(id) = identity else {
        checks.push(check(
            "wait",
            "wait.identity",
            "skip",
            "身份未解析，wait 无法认证",
            Some("先执行 agtalk id join <name>"),
            Some("agtalk id join <name>"),
            serde_json::Value::Null,
        ));
        checks.push(check(
            "wait",
            "wait.sse",
            "skip",
            "身份未解析，跳过 SSE 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "wait",
            "wait.replay",
            "skip",
            "身份未解析，跳过 replay 基线检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        return checks;
    };

    checks.push(check(
        "wait",
        "wait.identity",
        "ok",
        format!("身份已解析: {}", id.name),
        None,
        None,
        serde_json::Value::Null,
    ));

    if http_ok {
        let sse_url = format!("{}/api/v1/events", http_url);
        let headers = vec![
            ("X-AgTalk-Address", id.address.clone()),
            ("X-AgTalk-Pid", std::process::id().to_string()),
            (
                "X-AgTalk-Workspace-Root",
                ctx.dot_agtalk.to_string_lossy().to_string(),
            ),
        ];
        let sse_ok = http_get_stream_head(&sse_url, &headers).is_ok();
        checks.push(check(
            "wait",
            "wait.sse",
            if sse_ok { "ok" } else { "error" },
            if sse_ok {
                "SSE 可建立连接".to_string()
            } else {
                "SSE 无法建立连接".to_string()
            },
            if sse_ok {
                None
            } else {
                Some("检查 daemon 日志与身份认证")
            },
            None,
            serde_json::json!({ "url": sse_url }),
        ));
    } else {
        checks.push(check(
            "wait",
            "wait.sse",
            "skip",
            "daemon HTTP 不可达，跳过 SSE 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
    }

    let latest_event = ctx
        .storage
        .as_ref()
        .and_then(|s| inbox::max_event_id(s, &id.address).ok())
        .unwrap_or(0);
    checks.push(check(
        "wait",
        "wait.replay",
        "ok",
        format!("replay 基线 event_id: {}", latest_event),
        None,
        None,
        serde_json::json!({ "latest_event_id": latest_event }),
    ));

    checks
}

// ---- notify ----

pub(crate) fn notify_checks(
    ctx: &DoctorContext,
    identity: &Option<ResolvedIdentity>,
) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();

    let (detected_channel, detected_target) = notify::auto_detect();
    checks.push(check(
        "notify",
        "notify.environment",
        if detected_channel == "none" {
            "info"
        } else {
            "ok"
        },
        if detected_channel == "none" {
            "未检测到可用 notify plugin".to_string()
        } else {
            format!("检测到 {}", detected_channel)
        },
        if detected_channel == "none" {
            Some("安装 agtalk-notify-zellij / agtalk-notify-tmux 到 PATH 可获得终端通知")
        } else {
            None
        },
        None,
        serde_json::to_value(&detected_target).unwrap_or_default(),
    ));

    let Some(id) = identity else {
        checks.push(check(
            "notify",
            "notify.channel",
            "skip",
            "身份未解析，跳过 notify 通道检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "notify",
            "notify.target",
            "skip",
            "身份未解析，跳过 notify target 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        return checks;
    };

    let mb = ctx
        .storage
        .as_ref()
        .and_then(|s| mailbox::get_by_address(s, &id.address).ok())
        .flatten();
    let channel = mb
        .as_ref()
        .map(|m| m.notify_channel.clone())
        .unwrap_or_default();
    let target = mb
        .as_ref()
        .and_then(|m| serde_json::from_value::<NotifyTarget>(m.notify_target.clone()).ok());

    let (detected_channel, _detected_target) = notify::auto_detect();
    let channel_is_none = channel.is_empty() || channel == "none";
    if channel_is_none && detected_channel != "none" {
        checks.push(check(
            "notify",
            "notify.channel",
            "warn",
            format!(
                "notify 通道为 {}，但当前环境可用 {}",
                if channel.is_empty() { "none" } else { &channel },
                detected_channel
            ),
            Some("在对应终端环境内重新 join 以启用 notify"),
            Some(&format!("agtalk id join {} --notify auto", id.name)),
            serde_json::json!({ "channel": channel, "detected": detected_channel }),
        ));
    } else {
        checks.push(check(
            "notify",
            "notify.channel",
            "ok",
            format!(
                "notify 通道为 {}",
                if channel.is_empty() { "none" } else { &channel }
            ),
            None,
            None,
            serde_json::json!({ "channel": channel }),
        ));
    }

    let target_desc = match &target {
        Some(NotifyTarget::Plugin { name, endpoint }) => {
            format!("plugin {} endpoint: {}", name, endpoint)
        }
        _ => "none".to_string(),
    };
    checks.push(check(
        "notify",
        "notify.target",
        if target.is_some() && !matches!(target, Some(NotifyTarget::None)) {
            "ok"
        } else {
            "info"
        },
        &target_desc,
        None,
        None,
        serde_json::to_value(&target).unwrap_or_default(),
    ));

    // 插件通道：检查 discover + send --dry-run
    if let Some(NotifyTarget::Plugin { name, endpoint }) = target.as_ref() {
        let join_notify_cmd = format!("agtalk id join {} --notify plugin:{}", id.name, name);
        let set_plugin_path_cmd = format!(
            "agtalk config set notify.plugins.{}.path <name-or-abs-path>",
            name
        );
        match crate::notify::plugin::PluginChannel::new(name) {
            Ok(plugin) => match plugin.resolve_binary() {
                Ok(path) => match crate::notify::plugin::PluginChannel::validate_binary(&path) {
                    Ok(()) => {
                        checks.push(check(
                            "notify",
                            "notify.plugin.binary",
                            "ok",
                            format!("插件 {} 二进制: {}", name, path.display()),
                            None,
                            None,
                            serde_json::json!({ "path": path.to_string_lossy() }),
                        ));

                        // discover
                        match plugin.discover() {
                            Ok(endpoint_result) => {
                                if endpoint_result.ready {
                                    checks.push(check(
                                        "notify",
                                        "notify.plugin.discover",
                                        "ok",
                                        format!(
                                            "插件 {} discover ready: {}",
                                            name, endpoint_result.message
                                        ),
                                        None,
                                        None,
                                        serde_json::to_value(&endpoint_result).unwrap_or_default(),
                                    ));

                                    // endpoint 一致性：session 中保存的 endpoint 和当前 discover 结果是否一致。
                                    if let Some(NotifyTarget::Plugin {
                                        endpoint: saved_endpoint,
                                        ..
                                    }) = target.as_ref()
                                    {
                                        if saved_endpoint != &endpoint_result.endpoint {
                                            checks.push(check(
                                                "notify",
                                                "notify.plugin.endpoint_stale",
                                                "warn",
                                                format!(
                                                    "插件 {} endpoint 已过期（session 保存 {:?}，当前环境 {:?}）",
                                                    name, saved_endpoint, endpoint_result.endpoint
                                                ),
                                                Some("在当前终端环境内重新 join 以刷新 endpoint"),
                                                Some(&join_notify_cmd),
                                                serde_json::to_value(&endpoint_result)
                                                    .unwrap_or_default(),
                                            ));
                                        }
                                    }

                                    // dry-run
                                    let dummy = NotifyHint {
                                        from_name: "doctor".to_string(),
                                        binary_path: "agtalk".to_string(),
                                        agent_name: id.name.clone(),
                                        agent_address: id.address.clone(),
                                        message_id: "doctor-dry-run".to_string(),
                                        send_enter: true,
                                    };
                                    match plugin.send(endpoint, &dummy, true) {
                                        Ok(()) => {
                                            checks.push(check(
                                                "notify",
                                                "notify.plugin.dry_run",
                                                "ok",
                                                format!("插件 {} send --dry-run 成功", name),
                                                None,
                                                None,
                                                serde_json::Value::Null,
                                            ));
                                        }
                                        Err(e) => {
                                            checks.push(check(
                                                "notify",
                                                "notify.plugin.dry_run",
                                                "error",
                                                format!("插件 {} send --dry-run 失败: {}", name, e),
                                                Some(
                                                    "检查插件是否能在当前环境访问对应终端/session",
                                                ),
                                                None,
                                                serde_json::Value::Null,
                                            ));
                                        }
                                    }
                                } else {
                                    checks.push(check(
                                        "notify",
                                        "notify.plugin.discover",
                                        "error",
                                        format!(
                                            "插件 {} discover 返回 not ready: {}",
                                            name, endpoint_result.message
                                        ),
                                        Some("在对应终端环境内重新 join，或安装可用插件"),
                                        Some(&join_notify_cmd),
                                        serde_json::to_value(&endpoint_result).unwrap_or_default(),
                                    ));
                                }
                            }
                            Err(e) => {
                                checks.push(check(
                                    "notify",
                                    "notify.plugin.discover",
                                    "error",
                                    format!("插件 {} discover 失败: {}", name, e),
                                    Some("检查插件是否可执行、配置路径是否正确"),
                                    Some(&set_plugin_path_cmd),
                                    serde_json::Value::Null,
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        checks.push(check(
                            "notify",
                            "notify.plugin.binary",
                            "error",
                            format!("插件 {} 不可执行: {}", name, e),
                            Some("检查文件权限或使用 chmod +x"),
                            Some(&set_plugin_path_cmd),
                            serde_json::Value::Null,
                        ));
                    }
                },
                Err(e) => {
                    let msg = e.to_string();
                    let (suggestion, command) = if msg.contains("找不到") {
                        (
                            format!(
                                "将可执行文件放入 {}（推荐）或在 PATH 中安装 agtalk-notify-{}",
                                crate::paths::plugins_dir()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_else(|_| "<config_dir>/plugins".to_string()),
                                name
                            ),
                            Some(format!(
                                "agtalk config set notify.plugins.{}.path <name-or-abs-path>",
                                name
                            )),
                        )
                    } else if msg.contains("'..'") || msg.contains("逃逸") {
                        (
                            "插件相对路径禁止包含 '..'".to_string(),
                            Some(format!(
                                "agtalk config set notify.plugins.{}.path <name-or-abs-path>",
                                name
                            )),
                        )
                    } else {
                        (
                            format!(
                                "agtalk config set notify.plugins.{}.path <name-or-abs-path>",
                                name
                            ),
                            Some(format!(
                                "agtalk config set notify.plugins.{}.path <name-or-abs-path>",
                                name
                            )),
                        )
                    };
                    checks.push(check(
                        "notify",
                        "notify.plugin.binary",
                        "error",
                        format!("插件 {} 不可用: {}", name, msg),
                        Some(&suggestion),
                        command.as_deref(),
                        serde_json::Value::Null,
                    ));
                }
            },
            Err(e) => {
                checks.push(check(
                    "notify",
                    "notify.plugin.binary",
                    "error",
                    format!("非法插件名 {}: {}", name, e),
                    None,
                    None,
                    serde_json::Value::Null,
                ));
            }
        }
    }

    checks
}

// ---- http helpers ----
