<script setup lang="ts">
import { ref, watch } from "vue";

const props = defineProps<{
  participant: string;
  daemonOnline: boolean;
}>();

const theme = ref("system");

watch(theme, (v) => {
  document.documentElement.classList.remove("theme-light", "theme-dark");
  if (v === "light") document.documentElement.classList.add("theme-light");
  if (v === "dark") document.documentElement.classList.add("theme-dark");
});
</script>

<template>
  <div class="settings-view">
    <h2>设置</h2>

    <div class="setting-group">
      <label>当前身份</label>
      <input :value="props.participant || '未选择'" disabled />
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
      <div :style="{ color: props.daemonOnline ? 'var(--accent-green)' : 'var(--danger)', fontSize: '13px' }">
        {{ props.daemonOnline ? "在线" : "离线" }}
      </div>
      <div style="color: var(--text-secondary); font-size: 12px; margin-top: 4px;">
        通过 agtalk daemon start 启动 daemon
      </div>
    </div>

    <div class="setting-group">
      <label>关于</label>
      <div style="color: var(--text-secondary); font-size: 13px;">
        agtalk v0.3.0<br />
        本地 agent/agent 与 agent/human 协作控制台
      </div>
    </div>
  </div>
</template>
