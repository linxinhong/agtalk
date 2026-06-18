<script setup lang="ts">
import { ref } from "vue";

const participantName = ref("me");
const theme = ref("system");
</script>

<template>
  <div class="settings-view">
    <h2>设置</h2>

    <div class="setting-group">
      <label>当前参与者名称</label>
      <input v-model="participantName" placeholder="输入名称..." />
    </div>

    <div class="setting-group">
      <label>主题</label>
      <select v-model="theme">
        <option value="system">跟随系统</option>
        <option value="light">浅色</option>
        <option value="dark">深色</option>
      </select>
    </div>

    <div class="setting-group">
      <label>daemon 状态</label>
      <div style="color: var(--text-secondary); font-size: 13px;">
        通过 agtalk daemon start 启动 daemon
      </div>
    </div>

    <div class="setting-group">
      <label>关于</label>
      <div style="color: var(--text-secondary); font-size: 13px;">
        agtalk v2 v0.1.0<br />
        多方通信桌面应用 — 统一 agent ↔ agent 与 agent ↔ human 对话
      </div>
    </div>
  </div>
</template>
