// Agtalk HTTP API 客户端（MV3 Service Worker / Content Script 共用）
// 封装 agtalk daemon 的单一端点 POST /api 协议

export class AgtalkClient {
  constructor(baseUrl = 'http://127.0.0.1:19527') {
    this.baseUrl = baseUrl.replace(/\/$/, '');
    this.sessionId = null;
    this.token = null;
    this.me = null;
  }

  setSession(sessionId, token) {
    this.sessionId = sessionId;
    this.token = token;
  }

  clearSession() {
    this.sessionId = null;
    this.token = null;
    this.me = null;
  }

  async _api(type, payload = {}, { needsAuth = true } = {}) {
    const url = `${this.baseUrl}/api`;
    const headers = { 'Content-Type': 'application/json' };
    if (needsAuth) {
      if (!this.sessionId || !this.token) {
        throw Object.assign(new Error('session_not_ready'), { code: 'session_not_ready' });
      }
      headers['X-Agtalk-Session-Id'] = this.sessionId;
      headers['X-Agtalk-Token'] = this.token;
    }

    const body = JSON.stringify({ type, ...payload });
    const response = await fetch(url, { method: 'POST', headers, body });

    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }

    const data = await response.json();
    if (data.type === 'error') {
      const err = new Error(data.message || `agtalk error: ${data.code}`);
      err.code = data.code;
      throw err;
    }

    return data.data || data;
  }

  async ping() {
    try {
      await this._api('ping', {}, { needsAuth: false });
      return true;
    } catch (err) {
      return false;
    }
  }

  async join({
    workspaceRoot,
    workspaceName,
    name,
    participantType = 'web',
    role = 'web',
    intro = '',
    capabilities = [],
    transport = 'http',
    takeover = true,
  }) {
    const data = await this._api(
      'join',
      {
        workspace_root: workspaceRoot,
        workspace_name: workspaceName,
        name,
        participant_type: participantType,
        role,
        intro,
        capabilities: Array.isArray(capabilities) ? capabilities : [],
        transport,
        takeover,
      },
      { needsAuth: false }
    );

    if (data.session_id && data.token) {
      this.setSession(data.session_id, data.token);
    }
    if (data.participant) {
      this.me = data.participant;
    }
    return data;
  }

  async auth(sessionId, token) {
    if (sessionId && token) {
      this.setSession(sessionId, token);
    }
    const data = await this._api(
      'auth',
      { session_id: this.sessionId, token: this.token },
      { needsAuth: false }
    );
    if (data.participant) {
      this.me = data.participant;
    }
    return data;
  }

  async send({ to, body, conversationId, replyTo, contentType = 'text', notify = true, metadata }) {
    return await this._api('send', {
      to,
      body,
      conversation_id: conversationId || null,
      reply_to: replyTo || null,
      content_type: contentType,
      notify,
      metadata: metadata || null,
    });
  }

  async inbox({ participant, status = 'pending', limit = 50, peek = false } = {}) {
    const target = participant || (this.me ? this.me.name : null);
    if (!target) {
      throw new Error('participant_not_set');
    }
    return await this._api('inbox', { participant: target, status, limit, peek });
  }

  async done(msgId, participant) {
    const target = participant || (this.me ? this.me.name : null);
    return await this._api('done', { msg_id: msgId, participant: target });
  }

  async read(msgId, participant) {
    const target = participant || (this.me ? this.me.name : null);
    return await this._api('read', { msg_id: msgId, participant: target });
  }

  async listParticipants(participantType) {
    return await this._api('list_participants', { participant_type: participantType || null });
  }

  async whoAmI() {
    return await this._api('who_am_i', {});
  }
}

export default AgtalkClient;
