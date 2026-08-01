<script setup lang="ts">
import PopupView from './components/PopupView.vue'
import ConfigView from './components/ConfigView.vue'
import GraphView from './components/GraphView.vue'

// `agtalk __popup` 窗口以 ?popup=1 加载 → 弹窗模式；
// `agtalk graph gui`（M4）以 ?view=graph 加载 → 图工程管理界面；
// 其余为配置 GUI（agtalk config gui）
const params = new URLSearchParams(window.location.search)
const isPopup = params.has('popup')
const isGraph = params.get('view') === 'graph'
</script>

<template>
  <PopupView v-if="isPopup" />
  <GraphView v-else-if="isGraph" />
  <ConfigView v-else />
</template>
