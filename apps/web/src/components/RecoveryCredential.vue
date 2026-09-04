<script setup lang="ts">
import { computed, ref } from 'vue'

const props = defineProps<{ token: string }>()
const emit = defineEmits<{ dismiss: [] }>()
const copyResult = ref<'idle' | 'copied' | 'failed'>('idle')
const recoveryLink = computed(
  () => `${window.location.origin}${window.location.pathname}#recovery=${props.token}`,
)

async function copyRecoveryLink(): Promise<void> {
  try {
    await navigator.clipboard.writeText(recoveryLink.value)
    copyResult.value = 'copied'
  } catch {
    copyResult.value = 'failed'
  }
}
</script>

<template>
  <section class="recovery-credential" aria-labelledby="recovery-credential-heading">
    <h3 id="recovery-credential-heading">Guarde seu link individual</h3>
    <p id="recovery-credential-guidance">
      Este é o link válido mais recente. Ele recupera somente sua posição e exige também a senha
      da sala. O link não será exibido novamente depois que você sair desta tela.
    </p>
    <label for="issued-recovery-link">Link de recuperação</label>
    <textarea
      id="issued-recovery-link"
      aria-describedby="recovery-credential-guidance"
      readonly
      rows="3"
      :value="recoveryLink"
    ></textarea>
    <div class="recovery-credential-actions">
      <button class="text-button" type="button" @click="copyRecoveryLink()">
        {{ copyResult === 'copied' ? 'Copiar link novamente' : 'Copiar link' }}
      </button>
      <button class="text-button" type="button" @click="emit('dismiss')">
        Já guardei o link
      </button>
    </div>
    <p v-if="copyResult === 'copied'" class="copy-feedback" role="status">
      Link individual copiado.
    </p>
    <p
      v-else-if="copyResult === 'failed'"
      class="copy-feedback copy-feedback--error"
      role="alert"
    >
      Não foi possível copiar. Selecione o link e copie manualmente.
    </p>
  </section>
</template>
