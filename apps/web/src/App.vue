<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'

import type { HeroId, StartGameRequest } from './contracts/identity-access.generated'
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
const selectedContentKey = ref('')

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
const game = computed(() => roomAccess.game)
const isHost = computed(() => lobby.value?.participant.role === 'host')
const isRestoringSession = computed(() => roomAccess.status === 'restoring')
const sessionNeedsRecovery = computed(
  () => roomAccess.sessionExpected && !lobby.value && !game.value,
)
const lookupCode = computed(() => roomAccess.roomLookup?.room.code ?? '')
const lookupHeroes = computed(() => roomAccess.roomLookup?.heroes ?? [])
const adventureChoices = computed(() =>
  (lobby.value?.content_options ?? []).flatMap((manifest) =>
    manifest.adventures.map((adventure) => ({
      key: `${manifest.manifest_digest}:${adventure.id}`,
      adventure,
      manifest,
      playable: manifest.playable && adventure.playable,
    })),
  ),
)
const selectedContent = computed(() =>
  adventureChoices.value.find((choice) => choice.key === selectedContentKey.value),
)
const lobbyIsReadyToSeal = computed(
  () =>
    Boolean(lobby.value) &&
    (lobby.value?.participants.length ?? 0) >= 2 &&
    lobby.value?.participants.every((participant) => participant.ready && participant.hero),
)
const canStartGame = computed(
  () => Boolean(isHost.value && lobbyIsReadyToSeal.value && selectedContent.value?.playable),
)
const activeParticipant = computed(() =>
  game.value?.participants.find(
    (participant) => participant.position === game.value?.turn.active_position,
  ),
)
const currentGameParticipantPosition = computed(() => game.value?.participant.position)
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
      return 'A confirmação não chegou. Repita a mesma ação para consultar o resultado.'
    case 'INTERNAL_ERROR':
    case 'UNEXPECTED_RESPONSE':
      return 'A confirmação da partida falhou. Tente novamente com a mesma solicitação.'
    case 'ROOM_SEALED':
      return 'A sala já foi selada. Atualize para receber sua projeção inicial.'
    case 'ROOM_PARTICIPANT_COUNT_INVALID':
      return 'A sala precisa ter entre dois e quatro participantes.'
    case 'PARTICIPANT_HEROES_INVALID':
      return 'Cada participante precisa confirmar um Herói único.'
    case 'PARTICIPANTS_NOT_READY':
      return 'Todos os participantes precisam confirmar que estão prontos.'
    case 'CONTENT_NOT_PLAYABLE':
      return 'O conteúdo selecionado ainda possui lacunas funcionais e não pode iniciar uma partida.'
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

function lobbyIsBusy(): boolean {
  return ['selecting_hero', 'setting_readiness', 'starting_game', 'restoring'].includes(
    roomAccess.status,
  )
}

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

async function toggleReadiness(): Promise<void> {
  if (!lobby.value?.participant.hero) {
    return
  }
  await roomAccess.setReadiness(!lobby.value.participant.ready)
  await focusAfterAction(roomAccess.errorCode)
}

async function startGame(): Promise<void> {
  const content = selectedContent.value
  if (!content || !canStartGame.value) {
    return
  }
  const input: StartGameRequest = {
    adventure_id: content.adventure.id,
    manifest_digest: content.manifest.manifest_digest,
    ruleset_version: content.manifest.ruleset_version,
  }
  await roomAccess.startGame(input)
  await nextTick()
  document.getElementById(game.value ? 'game-heading' : 'room-success-heading')?.focus()
}

async function refreshLobby(): Promise<void> {
  await roomAccess.refreshSession()
  await nextTick()
  document.getElementById(game.value ? 'game-heading' : 'room-success-heading')?.focus()
}

async function focusAfterAction(errorCode: string | null): Promise<void> {
  await nextTick()
  if (lobby.value) {
    const nextAction =
      errorCode === null
        ? document.querySelector<HTMLButtonElement>('.action-dock .primary-button')
        : null
    const focusTarget = nextAction ?? document.getElementById('room-success-heading')
    focusTarget?.focus()
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
watch(
  adventureChoices,
  (choices) => {
    if (!choices.some((choice) => choice.key === selectedContentKey.value)) {
      selectedContentKey.value = choices.find((choice) => choice.playable)?.key ?? ''
    }
  },
  { immediate: true },
)

onMounted(async () => {
  await Promise.all([health.check(), roomAccess.restoreSession()])
  if (lobby.value || game.value) {
    await nextTick()
    document.getElementById(game.value ? 'game-heading' : 'room-success-heading')?.focus()
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
      v-else-if="game"
      class="room-success game-stage"
      aria-labelledby="game-heading"
    >
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">4</span>
        <span class="cue-line"></span>
        <span class="cue-label">Partida selada</span>
      </div>

      <div class="room-stage room-stage--success">
        <p class="service-confirmation" role="status">
          <span class="state-signal" aria-hidden="true"></span>
          Snapshot inicial confirmado
        </p>
        <h2 id="game-heading" tabindex="-1">Partida iniciada</h2>
        <p class="stage-description">
          A sala está selada. Posições, Heróis, aventura e versões permanecem fixos nesta partida.
        </p>

        <dl class="game-situation">
          <div>
            <dt>Turno</dt>
            <dd>{{ game.turn.number }}</dd>
          </div>
          <div>
            <dt>Fase</dt>
            <dd>{{ game.turn.phase === 'dark_arts' ? 'Artes das Trevas' : game.turn.phase }}</dd>
          </div>
          <div>
            <dt>Participante ativo</dt>
            <dd>{{ activeParticipant?.display_name ?? `Posição ${game.turn.active_position}` }}</dd>
          </div>
          <div>
            <dt>Aventura</dt>
            <dd>{{ game.game.adventure.name }}</dd>
          </div>
        </dl>

        <div class="participant-lineup">
          <h3>Posições seladas</h3>
          <ol>
            <li v-for="participant in game.participants" :key="participant.position">
              <span>Posição {{ participant.position }}</span>
              <strong>{{ participant.display_name }}</strong>
              <span>{{ participant.hero.name }}</span>
              <span v-if="participant.position === currentGameParticipantPosition">Você</span>
            </li>
          </ol>
        </div>

        <details class="snapshot-details">
          <summary>Ver versões do Snapshot</summary>
          <dl>
            <div>
              <dt>Estado</dt>
              <dd>v{{ game.snapshot.state_version }} · sequência {{ game.snapshot.sequence }}</dd>
            </div>
            <div>
              <dt>Ruleset</dt>
              <dd>{{ game.snapshot.versions.ruleset }}</dd>
            </div>
            <div>
              <dt>Manifesto</dt>
              <dd>v{{ game.snapshot.versions.manifest }}</dd>
            </div>
            <div>
              <dt>Digest</dt>
              <dd class="digest-value">{{ game.snapshot.digest }}</dd>
            </div>
            <div>
              <dt>PRNG</dt>
              <dd>{{ game.snapshot.versions.prng }}</dd>
            </div>
          </dl>
        </details>
        <p class="seed-note">A seed permanece secreta enquanto a partida estiver em andamento.</p>
      </div>
    </section>

    <section
      v-else-if="lobby"
      class="room-success"
      aria-labelledby="room-success-heading"
      aria-live="polite"
      :aria-busy="lobbyIsBusy()"
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
          <div>
            <dt>Prontidão</dt>
            <dd>{{ lobby.participant.ready ? 'Confirmada' : 'Pendente' }}</dd>
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
            :disabled="!selectedHero || lobbyIsBusy()"
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
              <span>Posição {{ participant.position }}</span>
              <strong>{{ participant.display_name }}</strong>
              <span>{{ participant.hero?.name ?? 'Herói pendente' }}</span>
              <span :class="participant.ready ? 'ready-label' : 'pending-label'">
                {{ participant.ready ? 'Pronto' : 'Preparando' }}
              </span>
            </li>
          </ol>
        </div>

        <div v-if="isHost" class="content-selection">
          <label for="adventure-selection">Aventura e conteúdo da partida</label>
          <select
            id="adventure-selection"
            v-model="selectedContentKey"
            :disabled="lobbyIsBusy() || Boolean(roomAccess.pendingStartInput)"
          >
            <option value="" disabled>Selecione conteúdo jogável</option>
            <template v-for="choice in adventureChoices" :key="choice.key">
              <option v-if="choice.playable" :value="choice.key">
                {{ choice.adventure.name }} · {{ choice.manifest.ruleset_version }}
              </option>
              <option v-else disabled :value="choice.key">
                {{ choice.adventure.name }} · {{ choice.manifest.ruleset_version }} · não jogável
              </option>
            </template>
          </select>
          <p v-if="selectedContent">
            Manifesto v{{ selectedContent.manifest.manifest_version }} ·
            {{ selectedContent.manifest.content_version }}
          </p>
          <p v-if="roomAccess.pendingStartInput" class="pending-selection-note">
            Escolha preservada para repetir a mesma solicitação com segurança.
          </p>
          <p v-if="!selectedContent" class="content-warning" role="status">
            Nenhum Manifesto jogável está publicado. Lacunas funcionais impedem o selo da sala.
          </p>
        </div>

        <div class="lobby-utilities">
          <button class="text-button" type="button" @click="copyRoomCode()">
            {{ copyResult === 'copied' ? 'Copiar código novamente' : 'Copiar código da sala' }}
          </button>
          <button
            v-if="lobby.participant.ready"
            class="text-button"
            type="button"
            :disabled="lobbyIsBusy()"
            @click="toggleReadiness()"
          >
            Reabrir minha preparação
          </button>
        </div>

        <p v-if="copyResult === 'copied'" class="copy-feedback" role="status">Código copiado.</p>
        <p v-else-if="copyResult === 'failed'" class="copy-feedback copy-feedback--error" role="alert">
          Não foi possível copiar. Selecione o código e copie manualmente.
        </p>
        <p v-if="lobbyError" class="form-error lobby-error" role="alert">{{ lobbyError }}</p>
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
      <p v-else-if="game" class="continuity-note">
        <span aria-hidden="true"></span>
        Estado inicial oficial recebido. A seed não foi enviada ao navegador.
      </p>
      <button
        v-else-if="lobby && lobby.participant.hero && !lobby.participant.ready"
        class="primary-button"
        :disabled="lobbyIsBusy()"
        type="button"
        @click="toggleReadiness()"
      >
        {{ roomAccess.status === 'setting_readiness' ? 'Confirmando prontidão' : 'Estou pronto' }}
      </button>
      <button
        v-else-if="lobby && isHost && canStartGame"
        class="primary-button"
        :disabled="lobbyIsBusy()"
        type="button"
        @click="startGame()"
      >
        {{ roomAccess.status === 'starting_game' ? 'Selando sala' : 'Selar sala e iniciar' }}
      </button>
      <button
        v-else-if="lobby && lobby.participant.ready"
        class="primary-button"
        :disabled="lobbyIsBusy()"
        type="button"
        @click="refreshLobby()"
      >
        {{
          roomAccess.status === 'restoring' ? 'Atualizando sala' : 'Atualizar estado da sala'
        }}
      </button>
      <p v-else-if="lobby" class="continuity-note">
        <span aria-hidden="true"></span>
        Escolha um Herói antes de confirmar sua prontidão.
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
