use super::*;
use clap::Parser;

#[test]
fn parse_bare_agtalk_has_no_command() {
    let cli = Cli::try_parse_from(["agtalk"]).unwrap();
    assert!(!cli.json);
    assert!(cli.command.is_none());
}

#[test]
fn parse_json_without_subcommand() {
    let cli = Cli::try_parse_from(["agtalk", "--json"]).unwrap();
    assert!(cli.json);
    assert!(cli.command.is_none());
}

#[test]
fn parse_config_gui_subcommand() {
    let cli = Cli::try_parse_from(["agtalk", "config", "gui"]).unwrap();
    match cli.command {
        Some(Commands::Config {
            cmd: ConfigCmd::Gui,
        }) => {}
        _ => panic!("expected Config gui"),
    }
}

#[test]
fn parse_id_join_without_notify_defaults_to_none_option() {
    let cli = Cli::try_parse_from(["agtalk", "id", "join", "nora"]).unwrap();
    let cmd = match cli.command {
        Some(Commands::Id { cmd }) => cmd,
        _ => panic!("expected Id join"),
    };
    match cmd {
        IdCmd::Join { notify, .. } => assert!(notify.is_none()),
        _ => panic!("expected Join"),
    }
}

#[test]
fn parse_id_join_notify_none_is_some() {
    let cli = Cli::try_parse_from(["agtalk", "id", "join", "nora", "--notify", "none"]).unwrap();
    let cmd = match cli.command {
        Some(Commands::Id { cmd }) => cmd,
        _ => panic!("expected Id join"),
    };
    match cmd {
        IdCmd::Join { notify, .. } => assert_eq!(notify, Some("none".to_string())),
        _ => panic!("expected Join"),
    }
}

#[test]
fn parse_msg_ask_waits_by_default_and_no_wait_flag() {
    let cli = Cli::try_parse_from(["agtalk", "msg", "ask", "deploy?"]).unwrap();
    match cli.command {
        Some(Commands::Msg {
            cmd: MsgCmd::Ask {
                no_wait, timeout, ..
            },
        }) => {
            assert!(!no_wait, "默认应等待回复");
            assert_eq!(timeout, None);
        }
        _ => panic!("expected Msg ask"),
    }

    let cli = Cli::try_parse_from([
        "agtalk",
        "msg",
        "ask",
        "deploy?",
        "--no-wait",
        "--timeout",
        "60",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Msg {
            cmd: MsgCmd::Ask {
                no_wait, timeout, ..
            },
        }) => {
            assert!(no_wait);
            assert_eq!(timeout, Some(60));
        }
        _ => panic!("expected Msg ask"),
    }

    // 旧 --wait 参数已删除
    assert!(Cli::try_parse_from(["agtalk", "msg", "ask", "q", "--wait"]).is_err());
}

#[test]
fn parse_mem_pack_positional_topic() {
    let cli = Cli::try_parse_from(["agtalk", "mem", "pack", "agent-learning-handbook"]).unwrap();
    let cmd = match cli.command {
        Some(Commands::Mem { cmd }) => cmd,
        _ => panic!("expected Mem pack"),
    };
    match cmd {
        MemCmd::Pack {
            topic_pos, topic, ..
        } => {
            assert_eq!(topic_pos, Some("agent-learning-handbook".to_string()));
            assert!(topic.is_none());
        }
        _ => panic!("expected Pack"),
    }
}

#[test]
fn parse_mem_pack_named_topic_takes_precedence() {
    let cli = Cli::try_parse_from([
        "agtalk",
        "mem",
        "pack",
        "positional-topic",
        "--topic",
        "named-topic",
    ])
    .unwrap();
    let cmd = match cli.command {
        Some(Commands::Mem { cmd }) => cmd,
        _ => panic!("expected Mem pack"),
    };
    match cmd {
        MemCmd::Pack {
            topic_pos, topic, ..
        } => {
            assert_eq!(topic_pos, Some("positional-topic".to_string()));
            assert_eq!(topic, Some("named-topic".to_string()));
        }
        _ => panic!("expected Pack"),
    }
}

#[test]
fn agent_help_text_is_quick_guide() {
    let msg = agent_help_message();
    let text = match msg {
        ServerMsg::AgentHelp { text, .. } => text,
        _ => panic!("expected AgentHelp"),
    };
    assert!(text.starts_with("agtalk agent quick guide"));
    assert!(text.contains("More:"));
    assert!(text.contains("agtalk --agent-guide"));
    assert!(text.contains("agtalk --help"));
    assert!(text.contains("agtalk <cmd> --help"));
    assert!(!text.contains("agtalk mem pack agtalk/agent-guide"));
    assert!(text.contains("inbox_empty means no message, not failure"));
    assert!(text.contains("Prefer agtalk run for reusable or record-worthy sends"));
    assert!(text.contains("If target notify_ready=true, do not wait after send"));
    assert!(text.contains("Wait only when target has no reliable notify"));
    assert!(text.contains("4. Run (preferred send entry)"));
    assert!(text.contains("agtalk run"));
    assert!(!text.contains("agent-learning-handbook"));
    // quick guide 不展开完整 Commands: 树
    assert!(!text.contains("Commands:"));
}

#[test]
fn agent_help_has_all_sections() {
    let msg = agent_help_message();
    let text = match msg {
        ServerMsg::AgentHelp { text, .. } => text,
        _ => panic!("expected AgentHelp"),
    };
    let sections = [
        "1. Identity",
        "2. Find target",
        "3. Send / reply / done",
        "4. Run (preferred send entry)",
        "5. Read loop",
        "6. Wait / ask human",
        "7. Memory / plan",
        "8. Diagnose",
    ];
    for s in sections {
        assert!(text.contains(s), "missing section: {}", s);
    }
    // full guide entry appears exactly in More
    assert!(text.contains("agtalk --agent-guide"));
}

#[test]
fn agent_help_json_has_structured_sections() {
    let msg = agent_help_message();
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "agent_help");
    assert_eq!(parsed["full_docs"], "agtalk --agent-guide");

    let more = parsed["more"].as_array().unwrap();
    assert_eq!(more.len(), 3);
    assert!(more.iter().any(|m| m["command"] == "agtalk --agent-guide"));
    assert!(more.iter().any(|m| m["command"] == "agtalk --help"));
    assert!(more.iter().any(|m| m["command"] == "agtalk <cmd> --help"));

    let sections = parsed["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 8);
    assert_eq!(sections[0]["title"], "Identity");
    assert_eq!(sections[3]["title"], "Run (preferred send entry)");
    assert_eq!(sections[6]["title"], "Memory / plan");
}

#[test]
fn parse_mem_relation_list() {
    let cli = Cli::try_parse_from(["agtalk", "mem", "relation", "list"]).unwrap();
    let cmd = match cli.command {
        Some(Commands::Mem { cmd }) => cmd,
        _ => panic!("expected Mem relation list"),
    };
    match cmd {
        MemCmd::Relation {
            cmd: MemRelationCmd::List { specialty },
        } => assert!(specialty.is_none()),
        _ => panic!("expected relation list"),
    }
}

#[test]
fn parse_mem_relation_list_with_specialty() {
    let cli = Cli::try_parse_from([
        "agtalk",
        "mem",
        "relation",
        "list",
        "--specialty",
        "Rust 实现",
    ])
    .unwrap();
    let cmd = match cli.command {
        Some(Commands::Mem { cmd }) => cmd,
        _ => panic!("expected Mem relation list"),
    };
    match cmd {
        MemCmd::Relation {
            cmd: MemRelationCmd::List { specialty },
        } => assert_eq!(specialty, Some("Rust 实现".to_string())),
        _ => panic!("expected relation list"),
    }
}

#[test]
fn parse_mem_relation_show() {
    let cli = Cli::try_parse_from(["agtalk", "mem", "relation", "show", "nora"]).unwrap();
    let cmd = match cli.command {
        Some(Commands::Mem { cmd }) => cmd,
        _ => panic!("expected Mem relation show"),
    };
    match cmd {
        MemCmd::Relation {
            cmd: MemRelationCmd::Show { name_or_address },
        } => assert_eq!(name_or_address, "nora"),
        _ => panic!("expected relation show"),
    }
}

#[test]
fn parse_mem_relation_update() {
    let cli = Cli::try_parse_from([
        "agtalk",
        "mem",
        "relation",
        "update",
        "nora",
        "--role",
        "reviewer",
        "--tag",
        "rust,frontend",
        "--note",
        "good partner",
        "--specialty",
        "Rust 实现,测试隔离",
        "--preferred-for",
        "功能开发",
    ])
    .unwrap();
    let cmd = match cli.command {
        Some(Commands::Mem { cmd }) => cmd,
        _ => panic!("expected Mem relation update"),
    };
    match cmd {
        MemCmd::Relation {
            cmd:
                MemRelationCmd::Update {
                    name_or_address,
                    role,
                    tag,
                    note,
                    specialty,
                    preferred_for,
                },
        } => {
            assert_eq!(name_or_address, "nora");
            assert_eq!(role, Some("reviewer".to_string()));
            assert_eq!(tag, vec!["rust".to_string(), "frontend".to_string()]);
            assert_eq!(note, Some("good partner".to_string()));
            assert_eq!(
                specialty,
                vec!["Rust 实现".to_string(), "测试隔离".to_string()]
            );
            assert_eq!(preferred_for, vec!["功能开发".to_string()]);
        }
        _ => panic!("expected relation update"),
    }
}

#[test]
fn parse_agent_guide_flag_without_subcommand() {
    let cli = Cli::try_parse_from(["agtalk", "--agent-guide"]).unwrap();
    assert!(cli.agent_guide);
    assert!(cli.command.is_none());
}

#[test]
fn agent_guide_markdown_is_non_empty() {
    let markdown = crate::mem::guide::agent_guide_markdown();
    assert!(!markdown.is_empty());
    assert!(markdown.contains("agtalk --agent-guide"));
    assert!(markdown.contains("inbox_empty"));
    assert!(markdown.contains("agtalk run"));
    assert!(markdown.contains("notify_ready"));
    assert!(markdown.contains("history.jsonl"));
}

#[test]
fn existing_notify_reuses_previous_channel() {
    // 幂等 join：session 已存在 → 复用原 notify（不因 auto 探测失败降级）
    let dir = tempfile::tempdir().unwrap();
    let alan_dir = dir.path().join("alan");
    std::fs::create_dir_all(&alan_dir).unwrap();
    std::fs::write(
        alan_dir.join("session.json"),
        r#"{
            "version": 2,
            "address": "93b837c6-68fa-4ab8-b359-141fada2494a",
            "name": "alan",
            "intro": "t",
            "notify": { "channel": "plugin:cmux", "endpoint": { "target": "pane-1" } }
        }"#,
    )
    .unwrap();
    let ctx = Context {
        dot_agtalk: dir.path().to_path_buf(),
        address: String::new(),
        name: String::new(),
        pid: 0,
        start_time: 0,
        base_url: String::new(),
    };
    let resolved = super::existing_notify(&ctx, Some("alan")).expect("应读到原 notify");
    assert_eq!(resolved.channel, "plugin:cmux");
    assert!(resolved.endpoint.is_some());
}

#[test]
fn existing_notify_none_for_new_session() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Context {
        dot_agtalk: dir.path().to_path_buf(),
        address: String::new(),
        name: String::new(),
        pid: 0,
        start_time: 0,
        base_url: String::new(),
    };
    assert!(
        super::existing_notify(&ctx, Some("nobody")).is_none(),
        "新注册 → 回退 auto"
    );
}
