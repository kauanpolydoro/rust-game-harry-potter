<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'

import type { HeroId } from './contracts/identity-access.generated'
import { type Availability, useHealthStore } from './stores/health'
import { useRoomAccessStore } from './stores/roomAccess'
import { useRoomCreationStore } from './stores/roomCreation'

const health = useHealthStore()
const roomAccess = useRoomAccessStore()
const roomCreation = useRoomCreationStore()
const entryMode = ref<'create' | 'join'>('create')
const displayName = ref('')
const recoveryPassword = ref('')
const roomCode = ref('')
const selectedHero = ref<HeroId | ''>('')
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
const lobby = computed(() => roomAccess.lobby)
const isHost = computed(() => lobby.value?.participant.role === 'host')
const isRestoringSession = computed(() => roomAccess.status === 'restoring')
const sessionNeedsRecovery = computed(() => roomAccess.sessionExpected && !lobby.value)
const lookupCode = computed(() => roomAccess.roomLookup?.room.code ?? '')
const lookupHeroes = computed(() => roomAccess.roomLookup?.heroes ?? [])
const serviceHeading = computed(() => {
  if (isRestoringSession.value) {
    return 'Retomando sua sessão'
  }
  if (health.availability === 'ready' && sessionNeedsRecovery.value) {
    return 'Não foi possível retomar'
  }
  return currentStatus.value.label
})
const serviceDescription = computed(() => {
  if (isRestoringSession.value) {
    return 'Confirmando sua posição durável nesta mesa.'
  }
  if (health.availability === 'ready' && sessionNeedsRecovery.value) {
    return 'Sua posição continua vinculada a este navegador. Tente novamente quando a conexão voltar.'
  }
  return currentStatus.value.description
})
const displayNameError = computed(() =>
  roomCreation.errorCode === 'INVALID_DISPLAY_NAME' ||
  roomAccess.errorCode === 'INVALID_DISPLAY_NAME'
    ? 'Informe um nome entre 1 e 40 caracteres.'
    : null,
)
const passwordError = computed(() =>
  roomCreation.errorCode === 'WEAK_RECOVERY_PASSWORD'
    ? 'Escolha uma senha mais longa e menos previsível.'
    : null,
)
const createFormError = computed(() => {
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
const joinFormError = computed(() => {
  switch (roomAccess.errorCode) {
    case null:
      return null
    case 'NETWORK_UNAVAILABLE':
      return roomAccess.roomLookup
        ? 'A confirmação não chegou. Tente entrar novamente com os mesmos dados.'
        : 'Não foi possível localizar a sala. Confira sua conexão e tente novamente.'
    case 'ROOM_NOT_FOUND':
    case 'ROOM_UNAVAILABLE':
      return 'Não foi possível encontrar uma sala aberta com esse código.'
    case 'ROOM_FULL':
      return 'A sala já tem quatro participantes.'
    case 'HERO_UNAVAILABLE':
      return 'Outro participante escolheu esse Herói primeiro. Escolha um dos disponíveis.'
    case 'INVALID_HERO':
      return 'Escolha um Herói disponível.'
    default:
      return 'Não foi possível entrar na sala. Revise os dados e tente novamente.'
  }
})
const lobbyError = computed(() => {
  switch (roomAccess.errorCode) {
    case 'HERO_UNAVAILABLE':
      return 'Outro participante escolheu esse Herói primeiro. Atualize sua escolha.'
    case 'NETWORK_UNAVAILABLE':
      return 'A confirmação não chegou. Tente confirmar o Herói novamente.'
    case null:
      return null
    default:
      return 'Não foi possível atualizar seu Herói.'
  }
})
const createSubmitLabel = computed(() => {
  if (roomCreation.status === 'submitting') {
    return 'Criando sala'
  }
  if (roomCreation.recoveringPendingIntent) {
    return 'Retomar criação pendente'
  }
  return roomCreation.status === 'failed' ? 'Tentar criar novamente' : 'Criar sala privada'
})
const joinSubmitLabel = computed(() => {
  if (!roomAccess.roomLookup) {
    return roomAccess.status === 'looking_up' ? 'Localizando sala' : 'Localizar sala'
  }
  if (roomAccess.status === 'joining') {
    return 'Entrando na sala'
  }
  return roomAccess.errorCode === 'NETWORK_UNAVAILABLE'
    ? 'Tentar entrar novamente'
    : 'Entrar na sala'
})

function retry(): void {
  if (health.availability !== 'checking') {
    void health.check()
  }
}

function retrySession(): void {
  void roomAccess.restoreSession()
}

async function createRoom(): Promise<void> {
  await roomCreation.createRoom({
    display_name: displayName.value,
    recovery_password: recoveryPassword.value,
  })

  if (roomCreation.roomCreation) {
    roomAccess.adoptCreatedRoom(roomCreation.roomCreation)
  }
  await focusAfterAction(roomCreation.errorCode)
}

async function findRoom(): Promise<void> {
  await roomAccess.findRoom(roomCode.value)
  if (roomAccess.roomLookup) {
    roomCode.value = roomAccess.roomLookup.room.code
    await nextTick()
    document.getElementById('join-display-name')?.focus()
  } else {
    await nextTick()
    document.getElementById('room-code')?.focus()
  }
}

async function joinRoom(): Promise<void> {
  if (!selectedHero.value) {
    return
  }
  await roomAccess.joinRoom({
    display_name: displayName.value,
    hero_id: selectedHero.value,
  })
  if (roomAccess.errorCode === 'HERO_UNAVAILABLE') {
    selectedHero.value = ''
  }
  await focusAfterAction(roomAccess.errorCode)
}

async function confirmHero(): Promise<void> {
  if (!selectedHero.value) {
    return
  }
  await roomAccess.selectHero(selectedHero.value)
  await focusAfterAction(roomAccess.errorCode)
}

async function focusAfterAction(errorCode: string | null): Promise<void> {
  await nextTick()
  if (lobby.value) {
    document.getElementById('room-success-heading')?.focus()
  } else if (errorCode === 'INVALID_DISPLAY_NAME') {
    document.getElementById(entryMode.value === 'join' ? 'join-display-name' : 'display-name')?.focus()
  } else if (errorCode === 'WEAK_RECOVERY_PASSWORD') {
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

function showJoin(): void {
  entryMode.value = 'join'
  displayName.value = ''
  roomCreation.resetPendingRequest()
}

function showCreate(): void {
  entryMode.value = 'create'
  displayName.value = ''
  roomCode.value = ''
  selectedHero.value = ''
  roomAccess.clearLookup()
}

function heroIsSelectable(heroId: HeroId, available: boolean): boolean {
  return available || lobby.value?.participant.hero?.id === heroId
}

async function copyRoomCode(): Promise<void> {
  const code = lobby.value?.room.code
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

onMounted(async () => {
  await Promise.all([health.check(), roomAccess.restoreSession()])
  if (lobby.value) {
    await nextTick()
    document.getElementById('room-success-heading')?.focus()
  }
})
</script>

<template>
  <main class="shell">
    <header class="masthead">
      <span class="cue-mark" aria-hidden="true"></span>
      <h1>Batalha de Hogwarts</h1>
      <span class="edition">Mesa cooperativa</span>
    </header>

    <section
      v-if="health.availability !== 'ready' || isRestoringSession || sessionNeedsRecovery"
      class="service-check"
      :class="`service-check--${health.availability}`"
      aria-labelledby="service-heading"
      :aria-busy="health.availability === 'checking' || roomAccess.status === 'restoring'"
    >
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">1</span>
        <span class="cue-line"></span>
        <span class="cue-label">Estado oficial</span>
      </div>

      <div class="service-state" role="status" aria-live="polite" aria-atomic="true">
        <div class="state-heading">
          <span class="state-signal" aria-hidden="true"></span>
          <h2 id="service-heading">{{ serviceHeading }}</h2>
        </div>
        <p class="state-description">
          {{ serviceDescription }}
        </p>
      </div>
    </section>

    <section
      v-else-if="lobby"
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
          Posição confirmada
        </p>
        <h2 id="room-success-heading" tabindex="-1">
          {{ isHost ? 'Sala pronta' : 'Sala aberta' }}
        </h2>
        <p class="stage-description">
          Sua participação está vinculada a esta sessão. O código apenas localiza a sala.
        </p>

        <div class="room-code-block">
          <span id="room-code-label">Código da sala</span>
          <output aria-labelledby="room-code-label">{{ lobby.room.code }}</output>
        </div>

        <dl class="room-details">
          <div>
            <dt>{{ isHost ? 'Anfitrião da sala' : 'Sua participação' }}</dt>
            <dd>{{ lobby.participant.display_name }}</dd>
          </div>
          <div>
            <dt>Posição durável</dt>
            <dd>Posição {{ lobby.participant.position }}</dd>
          </div>
          <div>
            <dt>Herói</dt>
            <dd>{{ lobby.participant.hero?.name ?? 'Ainda não escolhido' }}</dd>
          </div>
          <div>
            <dt>Sessão</dt>
            <dd>Protegida neste navegador</dd>
          </div>
        </dl>

        <form
          v-if="!lobby.participant.hero"
          class="hero-selection"
          :aria-busy="roomAccess.status === 'selecting_hero'"
          @submit.prevent="confirmHero()"
        >
          <fieldset>
            <legend>Escolha seu Herói</legend>
            <div class="hero-options">
              <template v-for="hero in lobby.heroes" :key="hero.id">
                <label v-if="heroIsSelectable(hero.id, hero.available)" class="hero-option">
                  <input
                    v-model="selectedHero"
                    :value="hero.id"
                    name="lobby-hero"
                    type="radio"
                  />
                  <span>{{ hero.name }}</span>
                  <small aria-hidden="true">Disponível</small>
                </label>
                <label v-else class="hero-option hero-option--unavailable">
                  <input :disabled="true" :value="hero.id" name="lobby-hero" type="radio" />
                  <span>{{ hero.name }}</span>
                  <small aria-hidden="true">Indisponível</small>
                </label>
              </template>
            </div>
          </fieldset>
          <button
            class="secondary-button"
            :disabled="!selectedHero || roomAccess.status === 'selecting_hero'"
            type="submit"
          >
            {{ roomAccess.status === 'selecting_hero' ? 'Confirmando Herói' : 'Confirmar Herói' }}
          </button>
          <p v-if="lobbyError" class="form-error" role="alert">{{ lobbyError }}</p>
        </form>

        <div class="participant-lineup">
          <h3>Participantes</h3>
          <ol>
            <li v-for="participant in lobby.participants" :key="participant.position">
              Posição {{ participant.position }} · {{ participant.display_name }} ·
              {{ participant.hero?.name ?? 'Herói pendente' }}
            </li>
          </ol>
        </div>

        <p v-if="copyResult === 'copied'" class="copy-feedback" role="status">Código copiado.</p>
        <p v-else-if="copyResult === 'failed'" class="copy-feedback copy-feedback--error" role="alert">
          Não foi possível copiar. Selecione o código e copie manualmente.
        </p>
      </div>
    </section>

    <section
      v-else-if="entryMode === 'create'"
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
              id="display-name"
              v-model="displayName"
              :aria-invalid="Boolean(displayNameError)"
              aria-describedby="display-name-error"
              autocomplete="nickname"
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
                id="recovery-password"
                v-model="recoveryPassword"
                :aria-invalid="Boolean(passwordError)"
                :type="passwordVisible ? 'text' : 'password'"
                aria-describedby="password-guidance password-error"
                autocomplete="new-password"
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

          <p v-if="createFormError" class="form-error" role="alert">{{ createFormError }}</p>
          <p class="alternate-path">
            Já recebeu um código?
            <button type="button" @click="showJoin()">Entrar em uma sala</button>
          </p>
        </form>
      </div>
    </section>

    <section v-else class="room-setup" aria-labelledby="join-heading">
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">2</span>
        <span class="cue-line"></span>
        <span class="cue-label">Entrar na mesa</span>
      </div>

      <div class="room-stage">
        <p class="service-confirmation" role="status">
          <span class="state-signal" aria-hidden="true"></span>
          Servidor pronto
        </p>
        <h2 id="join-heading">
          {{ roomAccess.roomLookup ? 'Escolha seu lugar à mesa' : 'Entre na sala do grupo' }}
        </h2>
        <p class="stage-description">
          {{
            roomAccess.roomLookup
              ? `Sala ${lookupCode} está aberta. Escolha somente entre os Heróis disponíveis.`
              : 'Use o código compartilhado pelo anfitrião. Ele localiza a sala, mas não recupera uma participação.'
          }}
        </p>

        <form
          v-if="!roomAccess.roomLookup"
          id="find-room"
          class="room-form"
          :aria-busy="roomAccess.status === 'looking_up'"
          @submit.prevent="findRoom()"
        >
          <div class="field">
            <label for="room-code">Código da sala</label>
            <input
              id="room-code"
              v-model="roomCode"
              aria-describedby="room-code-guidance join-form-error"
              autocomplete="off"
              inputmode="text"
              maxlength="8"
              minlength="8"
              name="room-code"
              pattern="[23456789A-HJ-NP-Za-hj-np-z]{8}"
              required
              spellcheck="false"
              type="text"
            />
            <p id="room-code-guidance" class="field-guidance">
              O código tem oito letras e números.
            </p>
          </div>
          <p v-if="joinFormError" id="join-form-error" class="form-error" role="alert">
            {{ joinFormError }}
          </p>
          <p class="alternate-path">
            Precisa abrir a mesa?
            <button type="button" @click="showCreate()">Criar uma sala</button>
          </p>
        </form>

        <form
          v-else
          id="join-room"
          class="room-form"
          :aria-busy="roomAccess.status === 'joining'"
          @submit.prevent="joinRoom()"
        >
          <div class="field">
            <label for="join-display-name">Seu nome</label>
            <input
              id="join-display-name"
              v-model="displayName"
              :aria-invalid="Boolean(displayNameError)"
              aria-describedby="join-display-name-error"
              autocomplete="nickname"
              maxlength="40"
              name="join-display-name"
              :readonly="roomAccess.status === 'joining' || Boolean(roomAccess.pendingInput)"
              required
              type="text"
            />
            <p id="join-display-name-error" class="field-error" role="alert">
              {{ displayNameError }}
            </p>
          </div>

          <fieldset class="hero-fieldset">
            <legend>Herói</legend>
            <div class="hero-options">
              <template v-for="hero in lookupHeroes" :key="hero.id">
                <label v-if="hero.available" class="hero-option">
                  <input
                    v-model="selectedHero"
                    :disabled="roomAccess.status === 'joining'"
                    :value="hero.id"
                    name="join-hero"
                    required
                    type="radio"
                  />
                  <span>{{ hero.name }}</span>
                  <small aria-hidden="true">Disponível</small>
                </label>
                <label v-else class="hero-option hero-option--unavailable">
                  <input
                    :disabled="true"
                    :value="hero.id"
                    name="join-hero"
                    required
                    type="radio"
                  />
                  <span>{{ hero.name }}</span>
                  <small aria-hidden="true">Indisponível</small>
                </label>
              </template>
            </div>
          </fieldset>
          <p v-if="joinFormError" class="form-error" role="alert">{{ joinFormError }}</p>
          <button class="text-button" type="button" @click="roomAccess.clearLookup()">
            Usar outro código
          </button>
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
        v-else-if="sessionNeedsRecovery"
        class="retry-button"
        type="button"
        :aria-disabled="roomAccess.status === 'restoring'"
        @click="retrySession()"
      >
        {{ isRestoringSession ? 'Retomando sessão' : 'Tentar retomar sessão' }}
      </button>
      <button
        v-else-if="lobby && isHost"
        class="primary-button"
        type="button"
        @click="copyRoomCode()"
      >
        {{ copyResult === 'copied' ? 'Copiar novamente' : 'Copiar código' }}
      </button>
      <p v-else-if="lobby" class="continuity-note">
        <span aria-hidden="true"></span>
        Sua posição continuará protegida nesta sessão.
      </p>
      <button
        v-else-if="entryMode === 'create'"
        class="primary-button"
        :disabled="roomCreation.status === 'submitting'"
        form="create-room"
        type="submit"
      >
        {{ createSubmitLabel }}
      </button>
      <button
        v-else-if="entryMode === 'join' && !roomAccess.roomLookup"
        class="primary-button"
        :disabled="roomAccess.status === 'looking_up'"
        form="find-room"
        type="submit"
      >
        {{ joinSubmitLabel }}
      </button>
      <button
        v-else
        class="primary-button"
        :disabled="roomAccess.status === 'joining' || !selectedHero"
        form="join-room"
        type="submit"
      >
        {{ joinSubmitLabel }}
      </button>
    </footer>
  </main>
</template>
