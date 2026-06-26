// 各平台的核心 DOM selector 与 URL match 配置，集中以数据形式存放。
// 注意：这只是 Phase 2 的最小共享层，不迁移完整的 content 注入逻辑。

export interface PlatformSelectors {
  chatContainer: string;
  messageItems: string;
  userMessageAttr?: string | null;
  aiMessageAttr?: string | null;
  messageText: string;
  inputBox: string;
  sendButton: string;
  stopButton?: string | null;
  stopButtonText?: string;
  aiContentArea?: string;
}

export interface PlatformDefinition {
  id: 'chatgpt' | 'claude' | 'sider' | 'chatglm' | 'custom';
  name: string;
  match: string[];
  selectors: PlatformSelectors;
}

export const BUILTIN_PLATFORMS: PlatformDefinition[] = [
  {
    id: 'chatgpt',
    name: 'ChatGPT',
    match: ['https://chatgpt.com/*'],
    selectors: {
      chatContainer: 'main [role="presentation"] > div > div > div',
      messageItems: '[data-testid^="conversation-turn-"]',
      userMessageAttr: 'user',
      aiMessageAttr: 'assistant',
      messageText: '.markdown, [data-message-content]',
      inputBox: '#prompt-textarea',
      sendButton: '[data-testid="send-button"]',
      stopButton: '[data-testid="stop-button"]',
    },
  },
  {
    id: 'claude',
    name: 'Claude',
    match: ['https://claude.ai/*'],
    selectors: {
      chatContainer: '.flex-1.flex.flex-col.relative main',
      messageItems: '[data-testid="user-message"], [data-testid="assistant-message"]',
      userMessageAttr: 'user-message',
      aiMessageAttr: 'assistant-message',
      messageText: '.font-claude-message, .whitespace-pre-wrap',
      inputBox: '[contenteditable="true"][data-testid="chat-input"], [contenteditable="true"][data-placeholder*="message"]',
      sendButton: '[data-testid="send-button"], button[class*="send"]:not([disabled])',
      stopButton: '[data-testid="stop-button"]',
    },
  },
  {
    id: 'sider',
    name: 'Sider',
    match: ['https://sider.ai/*'],
    selectors: {
      chatContainer: '.chat-list',
      messageItems: '.message-item',
      userMessageAttr: null,
      aiMessageAttr: '.content-area',
      messageText: '.message-inner',
      aiContentArea: '.content-area',
      inputBox: 'textarea[class*="chatBox-input"]',
      sendButton: '.send-btn:not([disabled]), button[type="submit"]:not([disabled]), [data-testid="send-button"]:not([disabled]), button[class*="send"]:not([disabled])',
      stopButton: null,
      stopButtonText: '停止生成',
    },
  },
  {
    id: 'chatglm',
    name: 'ChatGLM',
    match: ['https://chatglm.cn/*'],
    selectors: {
      chatContainer: '.main-chat-content, .chat-content, main, .chat-container',
      messageItems: '.answer-content, .question-content, .message-item, .chat-message',
      userMessageAttr: null,
      aiMessageAttr: null,
      messageText: '.markdown-body',
      inputBox: '[contenteditable="true"], .input-wrap textarea, .input-box textarea, .input-area textarea, textarea[placeholder*="输入"], textarea.gm-input',
      sendButton: '',
      stopButton: null,
    },
  },
];

export function matchPlatform(url: string): PlatformDefinition | null {
  for (const platform of BUILTIN_PLATFORMS) {
    for (const pattern of platform.match) {
      const prefix = pattern.replace(/\/\*$/, '');
      if (url.startsWith(prefix)) return platform;
    }
  }
  return null;
}
