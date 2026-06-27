import { defineBackground } from 'wxt/sandbox';
import { MessageType } from '@/shared/messaging/message-types';
import { createBackgroundRouter } from '@/shared/messaging/handlers';
import { getHealth, getStatus, getInbox, join, listParticipants } from '@/shared/api/client';
import { storage } from '@/shared/storage/storage';

export default defineBackground(() => {
  console.log('[WXT BG] agtalk service worker started');

  const router = createBackgroundRouter({
    [MessageType.PING_BACKGROUND]: async () => ({
      ok: true,
      pong: true,
      source: 'wxt-background',
    }),

    [MessageType.OPEN_APP_PAGE]: async () => {
      try {
        await chrome.tabs.create({ url: chrome.runtime.getURL('/app.html') });
        return { ok: true };
      } catch (err) {
        return { ok: false, error: err instanceof Error ? err.message : '打开页面失败' };
      }
    },

    [MessageType.API_HEALTH_CHECK]: async () => getHealth(),
    [MessageType.API_GET_STATUS]: async () => getStatus(),

    [MessageType.GET_CONFIG]: async () => {
      try {
        const config = await storage.getConfig();
        return { ok: true, data: config };
      } catch (err) {
        return { ok: false, error: err instanceof Error ? err.message : '读取配置失败' };
      }
    },

    [MessageType.SAVE_CONFIG]: async (message) => {
      try {
        const patch = ((message.payload || (message as any).config || {}) as Record<string, unknown>) || {};
        const config = await storage.saveConfig(patch);
        return { ok: true, data: config };
      } catch (err) {
        return { ok: false, error: err instanceof Error ? err.message : '保存配置失败' };
      }
    },

    [MessageType.GET_CONNECTED_PEERS]: async () => {
      const [config, peersRes] = await Promise.all([storage.getConfig(), listParticipants()]);
      if (!peersRes.ok) return peersRes;

      const connectedSet = new Set(config.connectedPeers || []);
      const autoInjectSet = new Set(config.autoInjectPeers || []);
      const peers = peersRes.data.map((p) => ({
        ...p,
        connected: connectedSet.has(p.name),
        autoInject: autoInjectSet.has(p.name),
      }));
      const activePeer = config.activePeer || config.targetAgent || '';
      return {
        ok: true,
        data: {
          peers,
          connectedPeers: config.connectedPeers || [],
          autoInjectPeers: config.autoInjectPeers || [],
          activePeer,
          agentName: config.agentName || '',
        },
      };
    },

    [MessageType.REGISTER_AGENT]: async () => {
      const config = await storage.getConfig();
      if (!config.agentName) {
        return { ok: false, error: 'agentName 为空，无法注册' };
      }
      const session = await storage.getSession();
      if (session?.participant === config.agentName) {
        return { ok: true, data: { session_id: session.session_id, participant: session.participant } };
      }
      const res = await join(config.agentName, config);
      if (!res.ok) return res;
      return { ok: true, data: { session_id: res.data.session_id, participant: config.agentName } };
    },

    [MessageType.RECONNECT]: async () => {
      const config = await storage.getConfig();
      if (!config.agentName) {
        return { ok: false, error: 'agentName 为空，无法重连' };
      }
      const res = await join(config.agentName, config);
      if (!res.ok) return res;
      return { ok: true, data: { session_id: res.data.session_id, participant: config.agentName } };
    },

    [MessageType.GET_RECENT_MESSAGES]: async () => {
      // 本地 IndexedDB 消息缓存未迁移，返回结构化错误，避免 UI 使用假数据
      return { ok: false, error: { code: 'not_migrated', message: '本地消息缓存未迁移' } };
    },

    [MessageType.AGTALK_INBOX]: async (message) => {
      const limit = (message.payload as { limit?: number })?.limit ?? 5;
      return getInbox(limit, 'all');
    },

    [MessageType.AGTALK_INBOX_STATS]: async () => {
      const status = await getStatus();
      if (!status.ok) return status;
      return {
        ok: true,
        data: {
          unread: status.data.inboxUnread ?? 0,
          total: status.data.inboxTotal ?? 0,
          authError: status.data.authError,
          inboxError: status.data.inboxError,
        },
      };
    },

    [MessageType.PAUSE_ALL_AUTO_REPLY]: async () => {
      // content script 自动回复逻辑未迁移；这里只消费消息，不执行复杂业务
      return { ok: true, data: { paused: true } };
    },
  });

  chrome.runtime.onMessage.addListener(router);
});
