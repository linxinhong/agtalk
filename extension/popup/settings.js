// agtalk Web Bridge 设置页面
const $ = (id) => document.getElementById(id);

const DEFAULT_CONFIG = {
  daemonUrl: 'http://127.0.0.1:19527',
  agtalkUrl: 'http://127.0.0.1:19527',
  agentName: '',
  agentRole: 'web',
  agentBio: 'Web AI bridge participant',
  agentCapabilities: '',
  targetAgent: '',
  enabled: true,
  autoForward: false,
  autoReceive: true,
  autoInject: false,
  enableChatgpt: true,
  enableClaude: true,
  enableSider: true,
  enableChatglm: true,
  enableCustom: false,
  pollInterval: 5000,
  workspaceRoot: '/virtual/web-bridge',
  workspaceName: 'web-bridge',
  captureDelay: 300,
};

let currentConfig = { ...DEFAULT_CONFIG };

async function load() {
  const result = await chrome.storage.local.get(['agtalk_config']);
  if (result.agtalk_config) {
    currentConfig = { ...DEFAULT_CONFIG, ...result.agtalk_config };
  } else {
    currentConfig = { ...DEFAULT_CONFIG };
  }
  fillUI(currentConfig);
}

function fillUI(cfg) {
  $('daemon-url-input').value = cfg.daemonUrl || cfg.agtalkUrl || 'http://127.0.0.1:19527';
  $('agent-name').value = cfg.agentName || '';
  $('agent-role').value = cfg.agentRole || 'web';
  $('agent-bio').value = cfg.agentBio || '';
  $('enabled').checked = !!cfg.enabled;
  $('auto-forward').checked = !!cfg.autoForward;
  $('auto-receive').checked = cfg.autoReceive !== false;
  $('auto-inject').checked = !!cfg.autoInject;
  $('enable-chatgpt').checked = cfg.enableChatgpt !== false;
  $('enable-claude').checked = !!cfg.enableClaude;
  $('enable-sider').checked = !!cfg.enableSider;
  $('enable-chatglm').checked = cfg.enableChatglm !== false;
  $('enable-custom').checked = !!cfg.enableCustom;
}

function readUI() {
  return {
    ...currentConfig,
    daemonUrl: $('daemon-url-input').value.trim() || 'http://127.0.0.1:19527',
    agentName: $('agent-name').value.trim(),
    agentRole: $('agent-role').value.trim() || 'web',
    agentBio: $('agent-bio').value.trim(),
    enabled: $('enabled').checked,
    autoForward: $('auto-forward').checked,
    autoReceive: $('auto-receive').checked,
    autoInject: $('auto-inject').checked,
    enableChatgpt: $('enable-chatgpt').checked,
    enableClaude: $('enable-claude').checked,
    enableSider: $('enable-sider').checked,
    enableChatglm: $('enable-chatglm').checked,
    enableCustom: $('enable-custom').checked,
  };
}

async function saveAndJoin() {
  const cfg = readUI();
  if (!cfg.agentName) {
    alert('请填写 Agent 名称');
    return;
  }
  currentConfig = cfg;
  $('save-btn').disabled = true;
  $('save-btn').textContent = '保存中...';

  await chrome.storage.local.set({ agtalk_config: cfg });
  await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'SAVE_CONFIG', config: cfg }, resolve);
  });
  const reg = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'REGISTER_AGENT' }, resolve);
  });

  $('save-btn').disabled = false;
  $('save-btn').textContent = '保存并 Join';
  if (reg?.ok) {
    alert('已保存并注册: ' + cfg.agentName);
  } else {
    alert('保存成功，但注册失败: ' + (reg?.error || '未知错误'));
  }
}

$('save-btn').addEventListener('click', saveAndJoin);

document.addEventListener('DOMContentLoaded', load);
