//! CLI `msg` 命名空间客户端。

use crate::cli::client::{get, post};
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};
use crate::proto::{AskOptions, ServerMsg};
use std::time::Duration;
use tokio_stream::StreamExt;

#[allow(clippy::too_many_arguments)]
pub fn send(
    ctx: Context,
    to: String,
    body: String,
    subject: Option<String>,
    files: Vec<String>,
    notify: Option<bool>,
    more: bool,
    json: bool,
) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/msg/send",
        serde_json::json!({
            "to": to,
            "body": body,
            "subject": subject,
            "files": files,
            "notify": notify,
            "more": more,
        }),
    )?;
    print_server_msg(json, &resp);
    Ok(())
}

pub fn reply(
    ctx: Context,
    message_id: String,
    body: String,
    files: Vec<String>,
    notify: Option<bool>,
    json: bool,
) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/msg/reply",
        serde_json::json!({
            "message_id": message_id,
            "body": body,
            "files": files,
            "notify": notify,
        }),
    )?;
    print_server_msg(json, &resp);
    Ok(())
}

pub fn done(
    ctx: Context,
    message_id: Option<String>,
    body: Option<String>,
    files: Vec<String>,
    json: bool,
) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/msg/done",
        serde_json::json!({
            "message_id": message_id,
            "body": body,
            "files": files,
        }),
    )?;
    print_server_msg(json, &resp);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn ask(
    ctx: Context,
    message: String,
    questions: Vec<String>,
    options: Vec<String>,
    recommended: Option<String>,
    single: bool,
    select_only: bool,
    wait: bool,
    timeout: Option<u64>,
    json: bool,
) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/msg/ask",
        serde_json::json!({
            "message": message,
            "questions": questions,
            "options": AskOptions {
                questions,
                options,
                recommended,
                single,
                select_only,
            },
            "wait": wait,
            "timeout": timeout,
        }),
    )?;
    print_server_msg(json, &resp);
    Ok(())
}

pub fn inbox(ctx: Context, all: bool, _limit: Option<usize>, json: bool) -> Result<(), CliError> {
    let endpoint: String = if all {
        "/api/v1/msg/inbox?all=true".into()
    } else {
        "/api/v1/msg/inbox".into()
    };
    let resp = get(&ctx, &endpoint)?;
    print_server_msg(json, &resp);
    Ok(())
}

pub fn read(ctx: Context, message_id: Option<String>, json: bool) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/msg/read",
        serde_json::json!({
            "message_id": message_id,
        }),
    )?;
    print_server_msg(json, &resp);
    Ok(())
}

pub fn wait(
    ctx: Context,
    msg_id: Option<String>,
    timeout: Option<u64>,
    since: Option<i64>,
    json: bool,
) -> Result<(), CliError> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| CliError::new("runtime", e.to_string()))?;
    rt.block_on(wait_async(ctx, msg_id, timeout, since, json))
}

pub fn attachment(_ctx: Context, _attachment_id: String, json: bool) -> Result<(), CliError> {
    let resp = ServerMsg::Error {
        code: "not_supported".into(),
        message: "附件下载尚未实现".into(),
    };
    print_server_msg(json, &resp);
    Ok(())
}

#[derive(serde::Serialize)]
struct WaitResult<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    messages: &'a [crate::routing::Message],
    body: String,
}

async fn wait_async(
    ctx: Context,
    msg_id: Option<String>,
    timeout: Option<u64>,
    since: Option<i64>,
    json: bool,
) -> Result<(), CliError> {
    let timeout_secs = timeout.unwrap_or(30);
    let url = format!("{}/api/v1/events", ctx.base_url);
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| CliError::new("client", e.to_string()))?;

    let mut req = client
        .get(&url)
        .header("X-AgTalk-Address", ctx.address)
        .header("X-AgTalk-Pid", ctx.pid.to_string())
        .header("X-AgTalk-Start-Time", ctx.start_time.to_string());
    if let Some(s) = since {
        req = req.header("Last-Event-ID", s.to_string());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| CliError::new("wait_failed", e.to_string()))?;
    if !resp.status().is_success() {
        return Err(CliError::new(
            "wait_failed",
            format!("wait failed: {}", resp.status()),
        ));
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let mut collected: Vec<crate::routing::Message> = Vec::new();

    loop {
        let chunk = tokio::time::timeout_at(deadline, stream.next()).await;
        match chunk {
            Ok(Some(Ok(bytes))) => {
                buf.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(pos) = buf.find("\n\n") {
                    let event_text: String = buf.drain(..pos + 2).collect();
                    let event_text = event_text.trim();
                    let event = parse_single_sse_event(event_text);
                    if let Ok(msg) = serde_json::from_str::<crate::routing::Message>(&event.data) {
                        let matched = msg_id
                            .as_ref()
                            .map(|id| msg.reply_to_id.as_deref() == Some(id.as_str()))
                            .unwrap_or(true);
                        if matched {
                            let more = is_more_coming(&msg.metadata);
                            collected.push(msg);
                            if !more {
                                print_wait_result(json, &collected);
                                return Ok(());
                            }
                        }
                    }
                }
            }
            Ok(Some(Err(e))) => {
                return Err(CliError::new("wait_failed", e.to_string()));
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    Err(CliError::new("timeout", "wait timeout".to_string()))
}

fn print_wait_result(json: bool, msgs: &[crate::routing::Message]) {
    let body: String = msgs.iter().map(|m| m.body.as_str()).collect();
    if json {
        let result = WaitResult {
            ty: "wait_result",
            messages: msgs,
            body,
        };
        println!("{}", serde_json::to_string(&result).unwrap_or_default());
    } else {
        if let Some(first) = msgs.first() {
            println!("from: {} <{}>", first.from_name, first.from_address);
        }
        println!("{}", body);
    }
}

fn is_more_coming(metadata: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(metadata)
        .ok()
        .and_then(|v| v.get("more_coming").and_then(|m| m.as_bool()))
        .unwrap_or(false)
}

#[derive(Default)]
struct SseEvent {
    id: String,
    event: String,
    data: String,
}

fn parse_single_sse_event(text: &str) -> SseEvent {
    let mut event = SseEvent::default();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("id:") {
            event.id = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("event:") {
            event.event = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            if !event.data.is_empty() {
                event.data.push('\n');
            }
            event.data.push_str(value.trim_start());
        }
    }
    event
}
