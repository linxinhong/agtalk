<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import PopupView from './components/PopupView.vue'

const { t, locale } = useI18n()

// `agtalk __popup` 窗口以 ?popup=1 加载，切到弹窗模式
const isPopup = new URLSearchParams(window.location.search).has('popup')

function toggleLocale() {
  locale.value = locale.value === 'zh-CN' ? 'en-US' : 'zh-CN'
}
</script>

<template>
  <PopupView v-if="isPopup" />
  <div v-else>
    <h1>{{ t('app.title') }}</h1>
    <p>{{ t('app.description') }}</p>
    <button @click="toggleLocale">{{ t('app.switchLanguage') }}</button>
  </div>
</template>
