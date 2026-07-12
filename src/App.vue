<script setup lang="ts">
import PopupView from './components/PopupView.vue'
import ConfigView from './components/ConfigView.vue'

// `agtalk __popup` 窗口以 ?popup=1 加载，切到弹窗模式；其余为配置 GUI（agtalk config gui）
const isPopup = new URLSearchParams(window.location.search).has('popup')
</script>

<template>
  <PopupView v-if="isPopup" />
  <ConfigView v-else />
</template>
