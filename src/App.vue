<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import PopupView from './components/PopupView.vue'
import ConfigView from './components/ConfigView.vue'

const { t, locale } = useI18n()

// `agtalk __popup` 窗口以 ?popup=1 加载，切到弹窗模式；其余为配置 GUI（agtalk config gui）
const isPopup = new URLSearchParams(window.location.search).has('popup')

function toggleLocale() {
  locale.value = locale.value === 'zh-CN' ? 'en-US' : 'zh-CN'
}
</script>

<template>
  <PopupView v-if="isPopup" />
  <div v-else class="main">
    <button class="locale" @click="toggleLocale">{{ t('app.switchLanguage') }}</button>
    <ConfigView />
  </div>
</template>

<style scoped>
.main {
  position: relative;
  height: 100vh;
}

.locale {
  position: absolute;
  top: 10px;
  right: 14px;
  z-index: 10;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 14px;
}
</style>
