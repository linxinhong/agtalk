<script setup lang="ts">
// 执行者头像（Tim 设计稿已批准）：djb2(name) → hue%360 圆形底色 + variant%8 选 1/8 极简 bot 脸。
// 同名必同像；在线=彩色实线环，离线/未认领=grayscale+灰虚线环。零依赖，SVG path 内联。
import { computed } from 'vue'

const props = defineProps<{
  participant: string
  online?: boolean
  size?: number
}>()

function djb2(s: string): number {
  let h = 5381
  for (let i = 0; i < s.length; i++) h = (h * 33) ^ s.charCodeAt(i)
  return h >>> 0
}

// 8 个极简 bot 脸变体（眼睛/表情 path，viewBox 0 0 100 100）
const FACES = [
  'M38 30a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM62 30a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z',
  'M34 24h8v8h-8zM58 24h8v8h-8z',
  'M32 27h12v3H32zM56 27h12v3H56z',
  'M37 27a4 4 0 1 0 0-8 4 4 0 0 0 0 8ZM63 27a4 4 0 1 0 0-8 4 4 0 0 0 0 8Z',
  'M36 28l6-8 6 8zM52 28l6-8 6 8z',
  'M50 27a6 6 0 1 0 0-12 6 6 0 0 0 0 12Z',
  'M33 30q7-8 14 0M53 30q7-8 14 0',
  'M50 18a6 6 0 1 0 0-12 6 6 0 0 0 0 12ZM38 30a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM62 30a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z',
]

const hash = computed(() => djb2(props.participant))
const hue = computed(() => hash.value % 360)
const face = computed(() => FACES[(hash.value >> 8) % FACES.length])
const size = computed(() => props.size ?? 36)
</script>

<template>
  <div
    class="pa-wrap"
    :class="{ 'pa-offline': online === false }"
    :style="{
      width: `${size}px`,
      height: `${size}px`,
      '--pa-hue': String(hue),
    }"
  >
    <svg :viewBox="'0 0 100 100'" class="pa-svg">
      <circle cx="50" cy="55" r="42" :fill="`hsl(${hue}, 55%, 82%)`" />
      <path :d="face" fill="currentColor" stroke="none" />
    </svg>
  </div>
</template>

<style scoped>
.pa-wrap {
  border-radius: 50%;
  border: 2px solid hsl(var(--pa-hue), 60%, 45%);
  overflow: hidden;
  flex-shrink: 0;
  box-sizing: border-box;
}
.pa-wrap.pa-offline {
  filter: grayscale(1);
  border-style: dashed;
  border-color: #9ca3af;
  opacity: 0.75;
}
.pa-svg {
  width: 100%;
  height: 100%;
  display: block;
  color: hsl(var(--pa-hue), 55%, 30%);
}
</style>
