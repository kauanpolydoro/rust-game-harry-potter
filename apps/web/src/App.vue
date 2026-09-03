<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'

import { type Availability, useHealthStore } from './stores/health'
import { useRoomCreationStore } from './stores/roomCreation'

const health = useHealthStore()
const roomCreation = useRoomCreationStore()
const displayName = ref('')
const recoveryPassword = ref('')
const passwordVisible = ref(false)
const copyResult = ref<'idle' | 'copied' | 'failed'>('idle')

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
const displayNameError = computed(() =>
  roomCreation.errorCode === 'INVALID_DISPLAY_NAME'
    ? 'Informe um nome entre 1 e 40 caracteres.'
    : null,
)
const passwordError = computed(() =>
  roomCreation.errorCode === 'WEAK_RECOVERY_PASSWORD'
    ? 'Escolha uma senha mais longa e menos previsível.'
    : null,
)
const formError = computed(() => {
  switch (roomCreation.errorCode) {
    case 'NETWORK_UNAVAILABLE':
      return 'A confirmação não chegou. Tente novamente para consultar a mesma criação.'
    case 'INVALID_DISPLAY_NAME':
    case 'WEAK_RECOVERY_PASSWORD':
    case null:
      return null
    case 'IDEMPOTENCY_KEY_REUSED':
      return roomCreation.recoveringPendingIntent
        ? 'O nome ou a senha não correspondem à criação pendente. Reinsira os mesmos dados ou descarte a tentativa.'
        : 'Não foi possível retomar a criação. Descarte a tentativa pendente para começar outra.'
    default:
      return 'Não foi possível criar a sala. Revise os dados e tente novamente.'
  }
})
const submitLabel = computed(() => {
  if (roomCreation.status === 'submitting') {
    return 'Criando sala'
  }
  if (roomCreation.recoveringPendingIntent) {
    return 'Retomar criação pendente'
  }
  return roomCreation.status === 'failed' ? 'Tentar criar novamente' : 'Criar sala privada'
})

function retry(): void {
  if (health.availability !== 'checking') {
    void health.check()
  }
}

async function createRoom(): Promise<void> {
  await roomCreation.createRoom({
    display_name: displayName.value,
    recovery_password: recoveryPassword.value,
  })

  await nextTick()
  if (roomCreation.roomCreation) {
    document.getElementById('room-success-heading')?.focus()
  } else if (roomCreation.errorCode === 'INVALID_DISPLAY_NAME') {
    document.getElementById('display-name')?.focus()
  } else if (roomCreation.errorCode === 'WEAK_RECOVERY_PASSWORD') {
    document.getElementById('recovery-password')?.focus()
  }
}

function togglePassword(): void {
  passwordVisible.value = !passwordVisible.value
}

function discardPendingRequest(): void {
  roomCreation.discardPendingRequest()
  displayName.value = ''
  recoveryPassword.value = ''
  passwordVisible.value = false
}

async function copyRoomCode(): Promise<void> {
  const code = roomCreation.roomCreation?.room.code
  if (!code) {
    return
  }

  try {
    await navigator.clipboard.writeText(code)
    copyResult.value = 'copied'
  } catch {
    copyResult.value = 'failed'
  }
}

watch([displayName, recoveryPassword], () => roomCreation.resetPendingRequest())

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
      v-if="health.availability !== 'ready'"
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

    <section
      v-else-if="roomCreation.roomCreation"
      class="room-success"
      aria-labelledby="room-success-heading"
      aria-live="polite"
    >
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">3</span>
        <span class="cue-line"></span>
        <span class="cue-label">Sala aberta</span>
      </div>

      <div class="room-stage room-stage--success">
        <p class="service-confirmation" role="status">
          <span class="state-signal" aria-hidden="true"></span>
          Servidor pronto
        </p>
        <h2 id="room-success-heading" tabindex="-1">Sala pronta</h2>
        <p class="stage-description">
          Este código localiza a sala para o grupo, mas não recupera nenhuma participação.
        </p>

        <div class="room-code-block">
          <span id="room-code-label">Código da sala</span>
          <output aria-labelledby="room-code-label">{{ roomCreation.roomCreation?.room.code }}</output>
        </div>

        <dl class="room-details">
          <div>
            <dt>Anfitrião da sala</dt>
            <dd>{{ roomCreation.roomCreation?.participant.display_name }}</dd>
          </div>
          <div>
            <dt>Sessão</dt>
            <dd>Protegida neste navegador</dd>
          </div>
        </dl>
        <p v-if="copyResult === 'copied'" class="copy-feedback" role="status">Código copiado.</p>
        <p v-else-if="copyResult === 'failed'" class="copy-feedback copy-feedback--error" role="alert">
          Não foi possível copiar. Selecione o código e copie manualmente.
        </p>
      </div>
    </section>

    <section
      v-else
      class="room-setup"
      :class="{ 'room-setup--pending': Boolean(roomCreation.pendingIntent) }"
      aria-labelledby="room-setup-heading"
    >
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">2</span>
        <span class="cue-line"></span>
        <span class="cue-label">Abrir a mesa</span>
      </div>

      <div class="room-stage">
        <p class="service-confirmation" role="status">
          <span class="state-signal" aria-hidden="true"></span>
          Servidor pronto
        </p>
        <h2 id="room-setup-heading">Abra uma sala para o seu grupo</h2>
        <p class="stage-description">
          Você será o anfitrião e continuará reconhecido neste navegador, sem criar uma conta.
        </p>

        <form
          id="create-room"
          class="room-form"
          :aria-busy="roomCreation.status === 'submitting'"
          @submit.prevent="createRoom()"
        >
          <div
            v-if="roomCreation.pendingIntent && roomCreation.status !== 'submitting'"
            class="pending-intent"
          >
            <p role="status">Existe uma criação pendente neste navegador.</p>
            <p>
              Retome com o mesmo nome e senha. Descartar inicia outra sala sem excluir a anterior.
            </p>
            <button type="button" @click="discardPendingRequest()">
              Descartar e começar outra
            </button>
          </div>

          <div class="field">
            <label for="display-name">Seu nome</label>
            <input
              v-model="displayName"
              :aria-invalid="Boolean(displayNameError)"
              aria-describedby="display-name-error"
              autocomplete="nickname"
              id="display-name"
              maxlength="40"
              name="display-name"
              :readonly="roomCreation.status === 'submitting' || Boolean(roomCreation.pendingInput)"
              required
              type="text"
            />
            <p id="display-name-error" class="field-error" role="alert">{{ displayNameError }}</p>
          </div>

          <div class="field">
            <label for="recovery-password">Senha de recuperação</label>
            <div class="password-control">
              <input
                v-model="recoveryPassword"
                :aria-invalid="Boolean(passwordError)"
                :type="passwordVisible ? 'text' : 'password'"
                aria-describedby="password-guidance password-error"
                autocomplete="new-password"
                id="recovery-password"
                maxlength="128"
                minlength="12"
                name="recovery-password"
                :readonly="roomCreation.status === 'submitting' || Boolean(roomCreation.pendingInput)"
                required
              />
              <button
                class="password-toggle"
                type="button"
                aria-controls="recovery-password"
                @click="togglePassword()"
              >
                {{ passwordVisible ? 'Ocultar senha' : 'Mostrar senha' }}
              </button>
            </div>
          </div>
          <p id="password-guidance" class="field-guidance">
            Use ao menos 12 caracteres e evite frases previsíveis. A senha não será exibida de novo.
          </p>
          <p id="password-error" class="field-error" role="alert">{{ passwordError }}</p>

          <p v-if="formError" class="form-error" role="alert">{{ formError }}</p>
        </form>
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
      <button
        v-else-if="roomCreation.roomCreation"
        class="primary-button"
        type="button"
        @click="copyRoomCode()"
      >
        {{ copyResult === 'copied' ? 'Copiar novamente' : 'Copiar código' }}
      </button>
      <button
        v-else
        class="primary-button"
        :disabled="roomCreation.status === 'submitting'"
        form="create-room"
        type="submit"
      >
        {{ submitLabel }}
      </button>
    </footer>
  </main>
</template>
