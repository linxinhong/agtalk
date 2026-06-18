agtalk 0.1.0 
Agent 与 Agent，Agent 与人协作的本地通信工具

用法:
  agtalk <命令> [参数]

常用命令:
  agtalk human <消息>               向人类发送消息或提问
  agtalk agent <消息>               向 Agent 发送任务或回复
  agtalk join  <name>              加入本地通信网络
  agtalk inbox                     查看收件箱
  agtalk chats                     查看对话列表
  agtalk peers                     列出所有在线参与者
  
人类对话:
  agtalk human <消息> [选项]
    -q, --question <text>          提出问题，可多次出现
    -o, --option <text>            添加预定义回答选项
    -o!, --option! <text>          同 -o，并标记为推荐答案
    --single                       单选，默认多选
    --select-only                  严格选择，禁用自由文本
    --output <text|json>           输出格式，默认 text

Agent 对话:
  agtalk agent <消息> [选项]
    -n, --name <name>              指定 Agent
    -s, --subject <标题>            消息主题
    -r, --reply-to <msg-id>        回复指定消息
    -d, --done <msg-id>            标记消息已完成
    -i, --notify                   提醒 Agent 查收消息

参与者:
  agtalk join <name>               加入本地通信网络
    --intro <text>                 Agent自我介绍
    --transport <plugin>           Agent的通知方式，指定插件
  agtalk leave                     离开本地通信网络
  agtalk me                        查看Agent自己的信息
  agtalk peers                     列出所有在线参与的Agent
  
收件箱:
  agtalk inbox                     查看收件箱
  agtalk chats                     查看对话列表

环境:
  agtalk init                      初始化环境
  agtalk settings                  打开设置界面
  agtalk daemon <action>           管理后台服务：start, stop, restart, status

帮助:
  agtalk --help, -h                显示帮助信息
  agtalk <命令> --help              子命令详细用法
  agtalk --agent-help              面向 AI 的精简用法
