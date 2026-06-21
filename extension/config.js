// 全局默认配置 —— content.js / background.js / popup.js 共享
// 使用 globalThis 确保在不同脚本上下文（经典脚本 / ES module）中都能访问
const AGTALK_CONFIG = {
  // agtalk daemon HTTP API 地址（popup 可覆盖，字段名 daemonUrl）
  daemonUrl: 'http://127.0.0.1:19527',
  // 兼容旧配置名
  agtalkUrl: 'http://127.0.0.1:19527',

  // web agent 默认身份（popup 可覆盖）
  agentName: '',
  agentRole: 'web',
  agentBio: 'Web AI bridge participant',
  agentCapabilities: '',

  // 目标 peer：agtalk 消息默认转发给谁
  targetAgent: '',

  // 行为开关
  enabled: false,
  autoForward: true,
  autoReceive: true,

  // inbox 轮询间隔（毫秒）
  pollInterval: 5000,

  // workspace 仅作为逻辑分组，不从 UI 暴露
  workspaceRoot: '/virtual/web-bridge',
  workspaceName: 'web-bridge',

  // 平台选择器更新防抖动
  captureDelay: 300,
};

globalThis.AGTALK_CONFIG = AGTALK_CONFIG;

if (typeof module !== 'undefined' && module.exports) {
  module.exports = { AGTALK_CONFIG };
}
