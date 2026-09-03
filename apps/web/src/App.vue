<script setup lang="ts">
import { computed, onMounted } from 'vue'

import { type Availability, useHealthStore } from './stores/health'

const health = useHealthStore()

const statusPresentation = {
  checking: {
    description: 'Confirmando acesso ao serviço autoritativo.',
    label: 'Verificando servidor',
  },
  ready: {
    description: 'O serviço autoritativo está pronto para receber o grupo.',
    label: 'Servidor pronto',
  },
  unavailable: {
    description: 'Não foi possível confirmar o serviço autoritativo. Tente novamente.',
    label: 'Servidor indisponível',
  },
} satisfies Record<Availability, { description: string; label: string }>

const currentStatus = computed(() => statusPresentation[health.availability])

function retry(): void {
  if (health.availability !== 'checking') {
    void health.check()
  }
}

onMounted(() => health.check())
</script>

<template>
  <main class="shell">
    <header class="masthead">
      <span class="cue-mark" aria-hidden="true"></span>
      <h1>Batalha de Hogwarts</h1>
      <span class="edition">Mesa cooperativa</span>
    </header>

    <section
      class="service-check"
      :class="`service-check--${health.availability}`"
      aria-labelledby="service-heading"
      :aria-busy="health.availability === 'checking'"
    >
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">1</span>
        <span class="cue-line"></span>
        <span class="cue-label">Estado oficial</span>
      </div>

      <div class="service-state" role="status" aria-live="polite" aria-atomic="true">
        <div class="state-heading">
          <span class="state-signal" aria-hidden="true"></span>
          <h2 id="service-heading">{{ currentStatus.label }}</h2>
        </div>
        <p class="state-description">{{ currentStatus.description }}</p>
      </div>
    </section>

    <footer class="action-dock">
      <button
        v-if="health.availability !== 'ready'"
        class="retry-button"
        type="button"
        :aria-disabled="health.availability === 'checking'"
        @click="retry()"
      >
        {{ health.availability === 'checking' ? 'Verificando servidor' : 'Tentar novamente' }}
      </button>
      <p v-else class="continuity-note">
        <span aria-hidden="true"></span>
        Aguardando a próxima etapa da mesa
      </p>
    </footer>
  </main>
</template>
