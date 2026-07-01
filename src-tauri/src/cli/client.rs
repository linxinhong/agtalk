//! CLI HTTP 客户端：封装 ClientMsg / ServerMsg 交互。

use crate::cli::context::Context;
use crate::proto::{ClientMsg, ServerMsg};
use reqwest::blocking::Client;
use std::time::Duration;
use tokio_stream::StreamExt;

pub fn join(
    ctx: Context,
    name: Option<String>,
    intro: Option<String>,
    workspace: Option<String>,
) -> Result<(), String> {
    let msg = ClientMsg::Join {
        name,
        intro,
        workspace,
        pid: ctx.pid,
        start_time: ctx.start_time,
    };
    let resp = post(&ctx, msg)?;
    print_server_msg(&resp);
    Ok(())
}

pub fn leave(ctx: Context, name: Option<String>) -> Result<(), String> {
    let msg = ClientMsg::Leave {
        name,
        address: ctx.address.clone(),
    };
    let resp = post(&ctx, msg)?;
    print_server_msg(&resp);
    Ok(())
}

pub fn whoami(ctx: Context) -> Result<(), String> {
    let resp = post(&ctx, ClientMsg::Whoami)?;
    print_server_msg(&resp);
    Ok(())
}

pub fn lookup(ctx: Context, name: Option<String>) -> Result<(), String> {
    let resp = post(&ctx, ClientMsg::Lookup { name })?;
    print_server_msg(&resp);
    Ok(())
}

pub fn send(
    ctx: Context,
    to: String,
    body: String,
    content_type: Option<String>,
    reply_to: Option<String>,
    more_coming: bool,
) -> Result<(), String> {
    let msg = ClientMsg::Send {
        to,
        body,
        content_type,
        reply_to_id: reply_to,
        metadata: None,
        more_coming,
    };
    let resp = post(&ctx, msg)?;
    print_server_msg(&resp);
    Ok(())
}

pub fn human(ctx: Context, body: String, choices: Vec<String>) -> Result<(), String> {
    let choices = if choices.is_empty() {
        None
    } else {
        Some(choices)
    };
    let msg = ClientMsg::Human { body, choices };
    let resp = post(&ctx, msg)?;
    print_server_msg(&resp);
    Ok(())
}

pub fn reply(
    ctx: Context,
    message_id: String,
    body: String,
    choice: Option<String>,
) -> Result<(), String> {
    let msg = ClientMsg::Reply {
        message_id,
        body,
        choice,
    };
    let resp = post(&ctx, msg)?;
    print_server_msg(&resp);
    Ok(())
}

pub fn inbox(ctx: Context, all: bool) -> Result<(), String> {
    let resp = post(&ctx, ClientMsg::Inbox { include_done: all })?;
    print_server_msg(&resp);
    Ok(())
}

pub fn detail(ctx: Context, message_id: String) -> Result<(), String> {
    let resp = post(&ctx, ClientMsg::Detail { message_id })?;
    print_server_msg(&resp);
    Ok(())
}

pub fn wait(
    ctx: Context,
    msg_id: Option<String>,
    timeout: Option<u64>,
    since: Option<i64>,
) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(wait_async(ctx, msg_id, timeout, since))
}

async fn wait_async(
    ctx: Context,
    msg_id: Option<String>,
    timeout: Option<u64>,
    since: Option<i64>,
) -> Result<(), String> {
    let timeout_secs = timeout.unwrap_or(30);
    let url = format!("{}/events", ctx.base_url);
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client
        .get(&url)
        .header("X-AgTalk-Address", ctx.address)
        .header("X-AgTalk-Pid", ctx.pid.to_string())
        .header("X-AgTalk-Start-Time", ctx.start_time.to_string());
    if let Some(s) = since {
        req = req.header("Last-Event-ID", s.to_string());
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("wait failed: {}", resp.status()));
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
                                print_collected(&collected);
                                return Ok(());
                            }
                        }
                    }
                }
            }
            Ok(Some(Err(e))) => return Err(e.to_string()),
            Ok(None) => break,
            Err(_) => break,
        }
    }

    Err("wait timeout".to_string())
}

fn is_more_coming(metadata: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(metadata)
        .ok()
        .and_then(|v| v.get("more_coming").and_then(|m| m.as_bool()))
        .unwrap_or(false)
}

fn print_collected(msgs: &[crate::routing::Message]) {
    if let Some(first) = msgs.first() {
        println!("from: {} <{}>", first.from_name, first.from_address);
    }
    let body: String = msgs.iter().map(|m| m.body.as_str()).collect();
    if body.trim().is_empty() {
        if let Some(last) = msgs.last() {
            println!("{}", serde_json::to_string_pretty(last).unwrap_or_default());
        }
    } else {
        println!("{}", body);
    }
}

fn post(ctx: &Context, msg: ClientMsg) -> Result<ServerMsg, String> {
    let client = Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/api", ctx.base_url);
    let resp = client
        .post(&url)
        .header("X-AgTalk-Address", ctx.address.clone())
        .header("X-AgTalk-Pid", ctx.pid.to_string())
        .header("X-AgTalk-Start-Time", ctx.start_time.to_string())
        .json(&msg)
        .send()
        .map_err(|e| e.to_string())?;

    let status = resp.status();
    let body = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, body));
    }
    serde_json::from_str(&body).map_err(|e| format!("解析响应失败: {}", e))
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

fn print_server_msg(msg: &ServerMsg) {
    match msg {
        ServerMsg::Pong => println!("pong"),
        ServerMsg::Ok { id } => println!("{}", id),
        ServerMsg::Error { code, message } => eprintln!("{}: {}", code, message),
        ServerMsg::LookupResult { mailboxes } => {
            for mb in mailboxes {
                println!(
                    "{}\t{}\t{}\t{}",
                    mb.address, mb.name, mb.workspace, mb.intro
                );
            }
        }
        ServerMsg::InboxResult { messages } => {
            for m in messages {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    m.id, m.event_id, m.from_name, m.from_address, m.content_type, m.status, m.body
                );
            }
        }
        ServerMsg::MessageDetail(m) => {
            println!("{}", serde_json::to_string_pretty(m).unwrap_or_default());
        }
        ServerMsg::WhoamiResult {
            address,
            name,
            workspace,
            intro,
        } => {
            println!("address   : {}", address);
            println!("name      : {}", name);
            println!("workspace : {}", workspace);
            println!("intro     : {}", intro);
        }
    }
}
