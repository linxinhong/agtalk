<script setup lang="ts">
import { useI18n } from 'vue-i18n'

const { t, locale } = useI18n()

function toggleLocale() {
  locale.value = locale.value === 'zh-CN' ? 'en-US' : 'zh-CN'
}
</script>

<template>
  <div>
    <h1>{{ t('app.title') }}</h1>
    <p>{{ t('app.description') }}</p>
    <button @click="toggleLocale">{{ t('app.switchLanguage') }}</button>
  </div>
</template>
