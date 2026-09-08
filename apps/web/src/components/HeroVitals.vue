<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import type { GameResources } from '../contracts/identity-access.generated'

const props = defineProps<{ resources: GameResources; epoch: number; damage?: { key: string; before?: number; after?: number } | undefined }>()
const trail = ref(props.resources.health)
let timer: ReturnType<typeof setTimeout> | undefined
watch(() => props.resources.health, (health, before) => {
  clearTimeout(timer)
  trail.value = Math.max(before, health)
  timer = setTimeout(() => { trail.value = props.resources.health }, 80)
})
watch(() => props.epoch, () => { clearTimeout(timer); trail.value = props.resources.health })
onBeforeUnmount(() => clearTimeout(timer))
</script>

<template>
  <span class="hero-vitals">
    <b>Vida {{ resources.health }}</b>
    <span class="health-meter" aria-hidden="true">
      <span class="health-meter__trail" :style="{ transform: `scaleX(${trail / 10})` }" />
      <span class="health-meter__current" :style="{ transform: `scaleX(${resources.health / 10})` }" />
      <span v-if="damage?.before !== undefined && damage.after !== undefined" :key="damage.key" class="health-meter__damage"
        :style="{ left: `${damage.after * 10}%`, width: `${(damage.before - damage.after) * 10}%` }" />
    </span>
    <small><span>Ataque {{ resources.attack }}</span><span>Influência {{ resources.influence }}</span></small>
  </span>
</template>
